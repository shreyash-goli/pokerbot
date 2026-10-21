//! Paired-seed, seat-swapped match runner.
//!
//! # Pairing
//! Deal `i` of a match uses deck seed [`deal_seed`]`(base_seed, i)`. Each deal
//! is played **twice** with the same deck: once with agent A on the button
//! and once with agent B on the button. The engine deals hole cards by
//! position (BB gets deck cards 0 and 2, the button 1 and 3, the board is
//! cards 4..9), so in the two games of a pair each *position* receives the
//! same cards and the same board; only who sits there changes. Card luck
//! therefore cancels within the pair.
//!
//! Agent A always occupies seat `Zero`; the button alternates between the two
//! games, so A plays the button in game 1 and the big blind in game 2.
//!
//! # Decision sampling
//! At every decision the harness takes the agent's reported distribution,
//! restricts it to the legal classes, and samples with an RNG stream owned by
//! the harness and keyed by `(deal seed, position)`. The agent's own sampled
//! action is checked for legality but otherwise ignored. Because the button's
//! draws in game 1 and game 2 come from the same stream (and likewise for the
//! BB), a mirror match of one deterministic-strategy agent replays exactly the
//! same hand twice with roles swapped, so every pair sums to exactly zero.
//!
//! # Information boundary
//! Agents receive only `state.observation(seat)` for their own seat. The
//! runner asserts, before each call, that the observation belongs to the
//! acting seat and carries no opponent hole cards.
//!
//! # Between-hand observation (adaptive agents)
//! A [`Participant`] may ask for its finished hands. It then receives
//! `state.events_for(Viewer::Seat(own seat))` for **every** hand it played,
//! in play order. Delivery happens at the **pair boundary**: both games of a
//! deal pair are played with the same participant state, and only then are
//! the two hands' events delivered. So the second game of a pair never
//! benefits from what was seen in the first game of the same deal. That
//! keeps the pair a like-for-like comparison, while the profile still
//! accumulates across the whole match. The runner checks that a delivered
//! stream holds no opponent `HoleCards` event.

use std::fmt;

use pokerbot_analysis::opponent::{AdaptStats, AdaptiveAgent};
use pokerbot_core::card::Card;
use pokerbot_core::deck::Deck;
use pokerbot_core::events::{HandEvent, Viewer};
use pokerbot_core::rng::{PokerbotRng, splitmix64_next};
use pokerbot_core::rules::{ActionKind, Position, RulesConfig};
use pokerbot_core::state::{HandConfig, HandState, Seat, TerminalReason};

use crate::agent::{Agent, AgentError, DecisionSource, InferStats};

/// Hard cap on deal pairs in one matchup.
pub const MAX_PAIRS: u64 = 1_000_000;
/// Default cap on actions in one hand (hang detection). The six-action
/// abstraction at 20-100 BB ends far sooner.
pub const DEFAULT_MAX_ACTIONS: u32 = 512;

const DEAL_SALT: u64 = 0x6576_616C_6465_616C; // "evaldeal"
const DRAW_SALT: u64 = 0x6576_616C_6472_6177; // "evaldraw"

/// Deck seed of deal `index` for `base_seed`.
pub fn deal_seed(base_seed: u64, index: u64) -> u64 {
    let mut s = base_seed ^ DEAL_SALT;
    let a = splitmix64_next(&mut s);
    let mut t = a ^ index.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    splitmix64_next(&mut t)
}

/// Seed of the decision-sampling stream for `position` in deal `deal_seed`.
pub fn draw_seed(deal_seed: u64, position: Position) -> u64 {
    let mut s =
        deal_seed ^ DRAW_SALT ^ (position.index() as u64 + 1).wrapping_mul(0xD1B5_4A32_D192_ED03);
    splitmix64_next(&mut s)
}

/// Errors from running a match.
#[derive(Clone, Debug, PartialEq)]
pub enum MatchError {
    /// An agent returned an error.
    Agent { label: String, error: AgentError },
    /// An agent's distribution had no positive finite mass on legal actions.
    BadDistribution {
        label: String,
        distribution: [f64; 6],
    },
    /// An agent's own sampled action was not legal.
    IllegalAction { label: String, action: ActionKind },
    /// The engine rejected an action (should be unreachable).
    Engine(String),
    /// A hand exceeded the action cap.
    TooManyActions(u32),
    /// The information boundary was violated (harness bug).
    InformationLeak(String),
    /// Invalid parameters.
    Config(String),
    /// A participant rejected a finished hand's events.
    Observer { label: String, error: String },
}

impl fmt::Display for MatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MatchError::Agent { label, error } => write!(f, "{label}: {error}"),
            MatchError::BadDistribution {
                label,
                distribution,
            } => write!(
                f,
                "{label}: distribution {distribution:?} has no legal mass"
            ),
            MatchError::IllegalAction { label, action } => {
                write!(f, "{label}: proposed illegal action {action:?}")
            }
            MatchError::Engine(m) => write!(f, "engine rejected action: {m}"),
            MatchError::TooManyActions(n) => write!(f, "hand exceeded {n} actions"),
            MatchError::InformationLeak(m) => write!(f, "information boundary violated: {m}"),
            MatchError::Config(m) => write!(f, "invalid match parameters: {m}"),
            MatchError::Observer { label, error } => {
                write!(f, "{label}: rejected hand events: {error}")
            }
        }
    }
}

impl std::error::Error for MatchError {}

/// Per-agent decision counters gathered by the harness.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DecisionCounts {
    /// Decisions taken.
    pub decisions: u64,
    /// Decisions whose source was a fallback (e.g. unseen info set).
    pub fallbacks: u64,
}

impl DecisionCounts {
    /// Fallback fraction (0 when there were no decisions).
    pub fn fallback_rate(&self) -> f64 {
        if self.decisions == 0 {
            0.0
        } else {
            self.fallbacks as f64 / self.decisions as f64
        }
    }
}

/// Referee's record of one finished hand (full information; for tests and
/// post-hoc analysis, never shown to agents).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HandLog {
    pub deal_seed: u64,
    pub button: Seat,
    /// Hole cards per seat index.
    pub hole: [[Card; 2]; 2],
    /// Full five-card board (as dealt, revealed or not).
    pub board: [Card; 5],
    /// Net chips per seat index.
    pub payoffs: [i64; 2],
    pub showdown: bool,
    pub actions: u32,
    /// Viewer-filtered events per seat index; empty unless that seat's
    /// participant asked for them.
    pub events: [Vec<HandEvent>; 2],
}

/// One deal pair: A's net chips in each game.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PairResult {
    pub deal_seed: u64,
    /// A's net chips in the game where A held the button.
    pub a_button: i64,
    /// A's net chips in the game where A held the big blind.
    pub a_big_blind: i64,
}

impl PairResult {
    /// A's net over both games.
    pub fn a_total(&self) -> i64 {
        self.a_button + self.a_big_blind
    }
}

/// Raw match output.
#[derive(Clone, Debug, PartialEq)]
pub struct MatchResult {
    pub label_a: String,
    pub label_b: String,
    pub rules: RulesConfig,
    pub base_seed: u64,
    pub pairs: Vec<PairResult>,
    pub counts_a: DecisionCounts,
    pub counts_b: DecisionCounts,
    /// The agents' own counters at the end of the match, if they keep any.
    pub infer_a: Option<InferStats>,
    pub infer_b: Option<InferStats>,
    /// Adaptation counters at the end of the match (adaptive agents only).
    pub adapt_a: Option<AdaptStats>,
    pub adapt_b: Option<AdaptStats>,
    /// Finished hands delivered to each participant.
    pub delivered_a: u64,
    pub delivered_b: u64,
    pub showdowns: u64,
}

impl MatchResult {
    /// Hands played (two per pair).
    pub fn hands(&self) -> u64 {
        2 * self.pairs.len() as u64
    }
}

/// Match parameters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MatchSpec {
    pub rules: RulesConfig,
    pub base_seed: u64,
    pub pairs: u64,
    pub max_actions: u32,
}

/// A seat occupant as seen by the runner: an [`Agent`] plus optional
/// between-hand observation.
pub trait Participant {
    /// The decision-making agent.
    fn agent(&mut self) -> &mut dyn Agent;
    /// Label of the agent.
    fn label(&self) -> String;
    /// The agent's own inference counters.
    fn infer_stats(&self) -> Option<InferStats>;
    /// Whether to deliver finished hands to [`Participant::hand_finished`].
    fn wants_events(&self) -> bool {
        false
    }
    /// Receives one finished hand: its own viewer-filtered events.
    fn hand_finished(&mut self, _events: &[HandEvent]) -> Result<(), String> {
        Ok(())
    }
    /// Adaptation counters, for adaptive agents.
    fn adapt_stats(&self) -> Option<AdaptStats> {
        None
    }
}

/// A plain agent that does not observe finished hands.
pub struct Plain<'a>(pub &'a mut dyn Agent);

impl Participant for Plain<'_> {
    fn agent(&mut self) -> &mut dyn Agent {
        &mut *self.0
    }
    fn label(&self) -> String {
        self.0.label()
    }
    fn infer_stats(&self) -> Option<InferStats> {
        self.0.infer_stats()
    }
}

impl<A: Agent + ?Sized> Participant for AdaptiveAgent<A> {
    fn agent(&mut self) -> &mut dyn Agent {
        self
    }
    fn label(&self) -> String {
        Agent::label(self)
    }
    fn infer_stats(&self) -> Option<InferStats> {
        Agent::infer_stats(self)
    }
    fn wants_events(&self) -> bool {
        true
    }
    fn hand_finished(&mut self, events: &[HandEvent]) -> Result<(), String> {
        self.observe_hand(events).map_err(|e| e.to_string())
    }
    fn adapt_stats(&self) -> Option<AdaptStats> {
        Some(AdaptiveAgent::adapt_stats(self))
    }
}

/// Samples a legal class from `dist` with `u` in `[0, 1)`.
pub fn sample_legal(dist: &[f64; 6], legal: &[ActionKind], u: f64) -> Option<ActionKind> {
    let w = |k: ActionKind| {
        let p = dist[k.index()];
        if p.is_finite() && p > 0.0 { p } else { 0.0 }
    };
    let total: f64 = legal.iter().map(|&k| w(k)).sum();
    if !(total.is_finite() && total > 0.0) {
        return None;
    }
    let target = u * total;
    let mut cum = 0.0;
    let mut last = None;
    for &k in legal {
        let p = w(k);
        if p == 0.0 {
            continue;
        }
        cum += p;
        last = Some(k);
        if target < cum {
            return Some(k);
        }
    }
    last
}

/// Plays one hand to completion. `agents[i]` sits in seat `i`.
pub fn play_hand(
    rules: RulesConfig,
    button: Seat,
    deal_seed: u64,
    agents: [&mut dyn Agent; 2],
    counts: &mut [DecisionCounts; 2],
    max_actions: u32,
) -> Result<HandLog, MatchError> {
    play_hand_with(
        rules,
        button,
        deal_seed,
        agents,
        counts,
        max_actions,
        [false; 2],
    )
}

fn play_hand_with(
    rules: RulesConfig,
    button: Seat,
    deal_seed: u64,
    mut agents: [&mut dyn Agent; 2],
    counts: &mut [DecisionCounts; 2],
    max_actions: u32,
    want_events: [bool; 2],
) -> Result<HandLog, MatchError> {
    let deck = Deck::from_seed(deal_seed);
    let order: Vec<Card> = deck.order()[..9].to_vec();
    let mut state = HandState::new(HandConfig { rules, button }, deck)
        .map_err(|e| MatchError::Config(e.to_string()))?;
    let mut rngs = [Position::Button, Position::BigBlind]
        .map(|p| PokerbotRng::seed_from_u64(draw_seed(deal_seed, p)));
    let mut actions = 0u32;
    while let Some(seat) = state.to_act() {
        if actions >= max_actions {
            return Err(MatchError::TooManyActions(max_actions));
        }
        let obs = state.observation(seat);
        if obs.seat != seat || !obs.is_my_turn() {
            return Err(MatchError::InformationLeak(
                "observation is not for the acting seat".into(),
            ));
        }
        if obs.opponent_hole_cards.is_some() {
            return Err(MatchError::InformationLeak(
                "opponent hole cards present at a decision".into(),
            ));
        }
        let agent = &mut agents[seat.index()];
        let label = agent.label();
        let decision = agent.act(&obs).map_err(|error| MatchError::Agent {
            label: label.clone(),
            error,
        })?;
        let legal: Vec<ActionKind> = obs.legal_actions.iter().map(|a| a.kind).collect();
        if !legal.contains(&decision.action) {
            return Err(MatchError::IllegalAction {
                label,
                action: decision.action,
            });
        }
        let c = &mut counts[seat.index()];
        c.decisions += 1;
        if matches!(decision.source, DecisionSource::Fallback(_)) {
            c.fallbacks += 1;
        }
        let pos = if seat == button {
            Position::Button
        } else {
            Position::BigBlind
        };
        let u = rngs[pos.index()].gen_f64();
        let kind =
            sample_legal(&decision.distribution, &legal, u).ok_or(MatchError::BadDistribution {
                label,
                distribution: decision.distribution,
            })?;
        state
            .apply(kind)
            .map_err(|e| MatchError::Engine(e.to_string()))?;
        actions += 1;
    }
    let payoffs = state
        .payoffs()
        .ok_or_else(|| MatchError::Engine("hand ended without payoffs".into()))?;
    if payoffs[0] + payoffs[1] != 0 {
        return Err(MatchError::Engine(format!(
            "payoffs {payoffs:?} not zero-sum"
        )));
    }
    let total: i64 = Seat::ALL
        .iter()
        .map(|&s| i64::from(state.final_stacks().expect("terminal")[s.index()]))
        .sum();
    if total != 2 * i64::from(rules.starting_stack) {
        return Err(MatchError::Engine("chips not conserved".into()));
    }
    let events = Seat::ALL.map(|seat| {
        if want_events[seat.index()] {
            state.events_for(Viewer::Seat(seat))
        } else {
            Vec::new()
        }
    });
    for seat in Seat::ALL {
        let leak = events[seat.index()]
            .iter()
            .any(|e| matches!(e, HandEvent::HoleCards { seat: s, .. } if *s != seat));
        if leak {
            return Err(MatchError::InformationLeak(
                "opponent hole-card event in a seat's event stream".into(),
            ));
        }
    }
    Ok(HandLog {
        deal_seed,
        button,
        hole: [state.hole_cards(Seat::Zero), state.hole_cards(Seat::One)],
        board: [order[4], order[5], order[6], order[7], order[8]],
        payoffs,
        showdown: state.terminal_reason() == Some(TerminalReason::Showdown),
        actions,
        events,
    })
}

/// Plays deal `seed` twice, A on the button then B on the button. Returns
/// the pair result and both hand logs.
pub fn play_pair(
    rules: RulesConfig,
    seed: u64,
    a: &mut dyn Agent,
    b: &mut dyn Agent,
    counts_a: &mut DecisionCounts,
    counts_b: &mut DecisionCounts,
    max_actions: u32,
) -> Result<(PairResult, [HandLog; 2]), MatchError> {
    let mut counts = [*counts_a, *counts_b];
    let r = play_pair_with(
        rules,
        seed,
        &mut Plain(a),
        &mut Plain(b),
        &mut counts,
        max_actions,
    )?;
    *counts_a = counts[0];
    *counts_b = counts[1];
    Ok(r)
}

fn deliver(p: &mut dyn Participant, seat: usize, games: [&HandLog; 2]) -> Result<(), MatchError> {
    for g in games {
        p.hand_finished(&g.events[seat])
            .map_err(|error| MatchError::Observer {
                label: p.label(),
                error,
            })?;
    }
    Ok(())
}

/// [`play_pair`] for participants; delivers both hands' events to those
/// that want them after the pair (see the module docs).
pub fn play_pair_with(
    rules: RulesConfig,
    seed: u64,
    a: &mut dyn Participant,
    b: &mut dyn Participant,
    counts: &mut [DecisionCounts; 2],
    max_actions: u32,
) -> Result<(PairResult, [HandLog; 2]), MatchError> {
    let want = [a.wants_events(), b.wants_events()];
    let g1 = play_hand_with(
        rules,
        Seat::Zero,
        seed,
        [a.agent(), b.agent()],
        counts,
        max_actions,
        want,
    )?;
    let g2 = play_hand_with(
        rules,
        Seat::One,
        seed,
        [a.agent(), b.agent()],
        counts,
        max_actions,
        want,
    )?;
    if want[0] {
        deliver(a, 0, [&g1, &g2])?;
    }
    if want[1] {
        deliver(b, 1, [&g1, &g2])?;
    }
    let pair = PairResult {
        deal_seed: seed,
        a_button: g1.payoffs[0],
        a_big_blind: g2.payoffs[0],
    };
    Ok((pair, [g1, g2]))
}

/// Plays a full paired match between plain agents.
pub fn run_match(
    spec: &MatchSpec,
    a: &mut dyn Agent,
    b: &mut dyn Agent,
) -> Result<MatchResult, MatchError> {
    run_match_with(spec, &mut Plain(a), &mut Plain(b))
}

/// Plays a full paired match between participants (A in seat `Zero`).
pub fn run_match_with(
    spec: &MatchSpec,
    a: &mut dyn Participant,
    b: &mut dyn Participant,
) -> Result<MatchResult, MatchError> {
    if spec.pairs == 0 || spec.pairs > MAX_PAIRS {
        return Err(MatchError::Config(format!(
            "pairs must be in 1..={MAX_PAIRS}, got {}",
            spec.pairs
        )));
    }
    spec.rules
        .validate()
        .map_err(|e| MatchError::Config(e.to_string()))?;
    let mut res = MatchResult {
        label_a: a.label(),
        label_b: b.label(),
        rules: spec.rules,
        base_seed: spec.base_seed,
        pairs: Vec::with_capacity(spec.pairs as usize),
        counts_a: DecisionCounts::default(),
        counts_b: DecisionCounts::default(),
        infer_a: None,
        infer_b: None,
        adapt_a: None,
        adapt_b: None,
        delivered_a: 0,
        delivered_b: 0,
        showdowns: 0,
    };
    let mut counts = [DecisionCounts::default(); 2];
    for i in 0..spec.pairs {
        let seed = deal_seed(spec.base_seed, i);
        let (pair, logs) = play_pair_with(spec.rules, seed, a, b, &mut counts, spec.max_actions)?;
        if a.wants_events() {
            res.delivered_a += 2;
        }
        if b.wants_events() {
            res.delivered_b += 2;
        }
        res.showdowns += logs.iter().filter(|l| l.showdown).count() as u64;
        res.pairs.push(pair);
    }
    [res.counts_a, res.counts_b] = counts;
    res.infer_a = a.infer_stats();
    res.infer_b = b.infer_stats();
    res.adapt_a = a.adapt_stats();
    res.adapt_b = b.adapt_stats();
    Ok(res)
}
