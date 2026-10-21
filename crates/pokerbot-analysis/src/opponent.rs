//! Opt-in opponent tendency model (M-20). **Experimental; not evaluated as
//! stronger than the fixed policy.**
//!
//! # Information boundary
//!
//! [`OpponentProfile`] is built only from **public actions**: who acted,
//! on which street, with which action class, plus the button and how the
//! hand ended. It never reads any hole cards — not the observed player's,
//! not even cards revealed at showdown — so hidden information cannot leak
//! into it by construction. [`OpponentProfile::observe_events`] additionally
//! refuses an event stream that contains the observed player's private
//! `HoleCards` event (i.e. anything but the observer's own view).
//!
//! [`AdaptiveAgent`] decides from the agent's own `PlayerObservation`
//! (what the base agent already receives) plus the profile. Nothing else.
//!
//! # Profile statistics (observed player `P`)
//!
//! * **VPIP / PFR** — as in [`crate::stats`]: over hands where `P` made a
//!   preflop decision; VPIP = a preflop call or bet/raise, PFR = a preflop
//!   bet/raise.
//! * **Facing aggression on street s** — `P` acts on `s` and the previous
//!   action on `s` was the other player's HalfPot/Pot/AllIn. (The blinds are
//!   not aggression: the button's first preflop action is never counted.)
//!   *fold-to-aggression[s]* = folds / times faced; *call-vs-aggression[s]* =
//!   calls / times faced.
//! * **Aggression frequency** — `(bets+raises) / (bets+raises+calls+folds)`
//!   over all of `P`'s actions.
//! * **Showdown frequency** — hands ending at showdown / hands observed.
//!
//! Every statistic carries its sample count; with no samples it is `None`.
//!
//! # Adaptation rules ([`adjust`])
//!
//! Applied only when [`AdaptConfig::enabled`], only on the agent's decision
//! with nothing to call (`to_call == 0`) on street `s`, and only when the
//! relevant statistic has at least [`AdaptConfig::min_samples`] samples.
//! Both rules are **hand-strength aware**: `e` is the agent's own equity vs
//! a uniformly random hand, estimated by [`hand_strength`] from the agent's
//! **own hole cards and the visible board only** (never the opponent's
//! cards). With rule strength `k = (x − t)/(1 − t)` for statistic `x` and
//! threshold `t`, at most `max_shift · k` probability is moved:
//!
//! 1. **Calling station** — call-vs-aggression[s] `c ≥ station_threshold`.
//!    A player who calls too much pays off value bets and ignores bluffs, so:
//!    * `e ≥` [`VALUE_EQUITY`]: move Check → the largest non-all-in legal bet
//!      (Pot, else HalfPot, else AllIn) — **value-bet more**;
//!    * `e ≤` [`BLUFF_EQUITY`]: move bets → Check, taking from AllIn, then
//!      Pot, then HalfPot — **bluff less**;
//!    * otherwise: no change.
//!
//!    (Revision: the first version moved large bets to Check regardless of
//!    hand strength, which also removed value bets; a locked comparator run
//!    measured it as significantly worse than the fixed policy against a
//!    check-call opponent.)
//! 2. **Over-folder** — fold-to-aggression[s] `f ≥ fold_threshold`. Bluffs
//!    and thin bets gain against a player who folds too much, but betting a
//!    strong hand mostly folds out hands that would pay later, so:
//!    * `e <` [`VALUE_EQUITY`]: move Check → the smallest legal bet
//!      (HalfPot, else Pot, else AllIn) — **bluff more**;
//!    * `e ≥` [`VALUE_EQUITY`]: no change (keep the policy's own mix).
//!
//! At most `max_shift` probability mass is moved in total (`max_shift ≤`
//! [`MAX_SHIFT_LIMIT`]); mass only moves between **legal** actions; the
//! result is renormalised. If no rule applies, the base decision is returned
//! unchanged — same action, same distribution, same RNG stream — so a
//! disabled or under-sampled adaptive agent is bit-identical to its base.

use std::fmt;

use pokerbot_cfr::infer::{Agent, AgentError, Decision, InferStats};
use pokerbot_core::action::{ActionKind, ActionRecord, Seat, Street, TerminalReason};
use pokerbot_core::card::{Card, CardSet};
use pokerbot_core::eval::evaluate7;
use pokerbot_core::events::HandEvent;
use pokerbot_core::observation::PlayerObservation;
use pokerbot_core::rng::PokerbotRng;

use crate::history::HandRecord;
use crate::stats::{fmt_num, fmt_pct};

/// Hard upper bound on [`AdaptConfig::max_shift`].
pub const MAX_SHIFT_LIMIT: f64 = 0.15;
/// Own equity (vs a random hand) at or above which a hand counts as strong.
pub const VALUE_EQUITY: f64 = 0.65;
/// Own equity at or below which a hand counts as weak (a bluff candidate).
pub const BLUFF_EQUITY: f64 = 0.40;
/// Trials of [`hand_strength`]'s seeded Monte Carlo estimate.
pub const STRENGTH_SAMPLES: u32 = 400;

/// The agent's own equity vs a uniformly random opponent hand, from its own
/// hole cards and the visible board only: a seeded Monte Carlo estimate
/// ([`STRENGTH_SAMPLES`] trials; the seed comes from these cards, so the
/// value is deterministic). `None` on invalid input.
pub fn hand_strength(hole: [Card; 2], board: &[Card]) -> Option<f64> {
    let mut known = hole.to_vec();
    known.extend_from_slice(board);
    if !matches!(board.len(), 0 | 3 | 4 | 5) || CardSet::try_from_cards(&known).is_err() {
        return None;
    }
    let used: CardSet = known.iter().copied().collect();
    let mut rest: Vec<Card> = Card::all().filter(|c| !used.contains(*c)).collect();
    let mut seed: u64 = 0x6a09_e667_f3bc_c908;
    for c in &known {
        seed = (seed ^ (u64::from(c.index()) + 1)).wrapping_mul(0x0100_0000_01b3);
    }
    let mut rng = PokerbotRng::seed_from_u64(seed);
    let need = 2 + 5 - board.len();
    let mut points = 0u64;
    for _ in 0..STRENGTH_SAMPLES {
        for k in 0..need {
            let j = k + rng.gen_range((rest.len() - k) as u64) as usize;
            rest.swap(k, j);
        }
        let mut b = [rest[0]; 5];
        b[..board.len()].copy_from_slice(board);
        b[board.len()..].copy_from_slice(&rest[2..need]);
        let me = evaluate7(&[hole[0], hole[1], b[0], b[1], b[2], b[3], b[4]]);
        let opp = evaluate7(&[rest[0], rest[1], b[0], b[1], b[2], b[3], b[4]]);
        points += match me.cmp(&opp) {
            std::cmp::Ordering::Greater => 2,
            std::cmp::Ordering::Equal => 1,
            std::cmp::Ordering::Less => 0,
        };
    }
    Some(points as f64 / (2.0 * f64::from(STRENGTH_SAMPLES)))
}

const STREETS: [Street; 4] = [Street::Preflop, Street::Flop, Street::Turn, Street::River];

fn street_index(s: Street) -> Option<usize> {
    STREETS.iter().position(|&x| x == s)
}

/// A frequency with its sample count.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Freq {
    pub hits: u64,
    pub samples: u64,
}

impl Freq {
    fn add(&mut self, hit: bool) {
        self.samples += 1;
        self.hits += u64::from(hit);
    }

    /// `hits / samples`, `None` with no samples.
    pub fn value(&self) -> Option<f64> {
        (self.samples > 0).then(|| self.hits as f64 / self.samples as f64)
    }

    fn show(&self) -> String {
        format!("{} (n={})", fmt_pct(self.value()), self.samples)
    }
}

/// Public tendencies of one observed player.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OpponentProfile {
    pub hands: u64,
    pub vpip: Freq,
    pub pfr: Freq,
    /// Indexed by street (preflop, flop, turn, river).
    pub fold_to_aggression: [Freq; 4],
    pub call_vs_aggression: [Freq; 4],
    pub aggression: Freq,
    pub showdown: Freq,
}

/// Why an event stream was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProfileError {
    /// The stream contains the observed player's private hole cards.
    HiddenInformation,
    /// The stream is not a valid finished hand.
    Invalid(String),
}

impl fmt::Display for ProfileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProfileError::HiddenInformation => {
                write!(f, "stream contains the observed player's hidden cards")
            }
            ProfileError::Invalid(e) => write!(f, "invalid hand: {e}"),
        }
    }
}

impl std::error::Error for ProfileError {}

impl OpponentProfile {
    pub fn new() -> OpponentProfile {
        OpponentProfile::default()
    }

    /// Forgets everything.
    pub fn reset(&mut self) {
        *self = OpponentProfile::default();
    }

    /// Adds one finished hand from public data only.
    pub fn observe_public(
        &mut self,
        observed: Seat,
        actions: &[ActionRecord],
        reason: TerminalReason,
    ) {
        self.hands += 1;
        self.showdown.add(reason == TerminalReason::Showdown);
        let (mut decided, mut vpip, mut pfr) = (false, false, false);
        let mut prev: Option<&ActionRecord> = None;
        for a in actions {
            if prev.is_some_and(|p| p.street != a.street) {
                prev = None;
            }
            if a.seat == observed {
                let aggr = a.kind.is_aggressive();
                match a.kind {
                    ActionKind::Check => {}
                    k => self.aggression.add(aggr && k != ActionKind::Fold),
                }
                let faced = prev.is_some_and(|p| p.seat != observed && p.kind.is_aggressive());
                if faced && let Some(i) = street_index(a.street) {
                    self.fold_to_aggression[i].add(a.kind == ActionKind::Fold);
                    self.call_vs_aggression[i].add(a.kind == ActionKind::Call);
                }
                if a.street == Street::Preflop {
                    decided = true;
                    vpip |= a.kind == ActionKind::Call || aggr;
                    pfr |= aggr;
                }
            }
            prev = Some(a);
        }
        if decided {
            self.vpip.add(vpip);
            self.pfr.add(pfr);
        }
    }

    /// Adds a finished hand from the **observer's** viewer-filtered events.
    /// Refuses streams that contain `observed`'s private hole cards.
    pub fn observe_events(
        &mut self,
        observed: Seat,
        events: &[HandEvent],
    ) -> Result<(), ProfileError> {
        if events
            .iter()
            .any(|e| matches!(e, HandEvent::HoleCards { seat, .. } if *seat == observed))
        {
            return Err(ProfileError::HiddenInformation);
        }
        let obs = PlayerObservation::from_events(observed.other(), events)
            .map_err(|e| ProfileError::Invalid(e.to_string()))?;
        let reason = obs
            .terminal
            .ok_or_else(|| ProfileError::Invalid("hand not finished".into()))?;
        self.observe_public(observed, &obs.history, reason);
        Ok(())
    }

    /// Adds a saved hand, observing its human seat. Only the action list and
    /// result are read (no cards).
    pub fn observe_record(&mut self, rec: &HandRecord) {
        self.observe_public(rec.human_seat, &rec.actions, rec.reason);
    }

    /// Printable summary with sample sizes.
    pub fn diagnostics(&self) -> Vec<String> {
        let per = |fs: &[Freq; 4]| {
            STREETS
                .iter()
                .zip(fs)
                .map(|(s, f)| format!("{} {}", street_word(*s), f.show()))
                .collect::<Vec<_>>()
                .join(" | ")
        };
        vec![
            format!(
                "Profile of your play from public actions only (no cards): {} hand(s)",
                self.hands
            ),
            format!("VPIP {} | PFR {}", self.vpip.show(), self.pfr.show()),
            format!("Fold to a bet/raise: {}", per(&self.fold_to_aggression)),
            format!("Call vs a bet/raise: {}", per(&self.call_vs_aggression)),
            format!(
                "Aggression frequency {} | reached showdown {}",
                self.aggression.show(),
                self.showdown.show()
            ),
        ]
    }
}

fn street_word(s: Street) -> &'static str {
    match s {
        Street::Preflop => "preflop",
        Street::Flop => "flop",
        Street::Turn => "turn",
        Street::River => "river",
        Street::Showdown => "showdown",
    }
}

/// Adaptation settings. `Default` is **disabled**.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AdaptConfig {
    pub enabled: bool,
    /// Samples a statistic needs before any rule uses it.
    pub min_samples: u64,
    /// Most probability mass moved per decision (≤ [`MAX_SHIFT_LIMIT`]).
    pub max_shift: f64,
    /// Rule 1 threshold on fold-to-aggression.
    pub fold_threshold: f64,
    /// Rule 2 threshold on call-vs-aggression.
    pub station_threshold: f64,
    /// Seed of the adaptive agent's own sampling RNG.
    pub seed: u64,
}

impl Default for AdaptConfig {
    fn default() -> Self {
        AdaptConfig {
            enabled: false,
            min_samples: 20,
            max_shift: 0.10,
            fold_threshold: 0.60,
            station_threshold: 0.60,
            seed: 0,
        }
    }
}

impl AdaptConfig {
    /// Checks the ranges documented on each field.
    pub fn validate(&self) -> Result<(), String> {
        if !(0.0..=MAX_SHIFT_LIMIT).contains(&self.max_shift) {
            return Err(format!("max_shift must be in 0..={MAX_SHIFT_LIMIT}"));
        }
        for (name, t) in [
            ("fold_threshold", self.fold_threshold),
            ("station_threshold", self.station_threshold),
        ] {
            if !(t > 0.0 && t < 1.0) {
                return Err(format!("{name} must be strictly between 0 and 1"));
            }
        }
        if self.min_samples == 0 {
            return Err("min_samples must be at least 1".into());
        }
        Ok(())
    }
}

/// Result of [`adjust`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Adjustment {
    pub distribution: [f64; 6],
    /// Total probability mass moved (≤ `max_shift`).
    pub moved: f64,
    /// Which rule fired.
    pub rule: &'static str,
}

fn strength(f: Freq, threshold: f64, min: u64) -> Option<f64> {
    let v = f.value()?;
    (f.samples >= min && v >= threshold)
        .then(|| ((v - threshold) / (1.0 - threshold)).clamp(0.0, 1.0))
}

/// The adapted distribution for `obs`, or `None` if no rule applies (see
/// the module docs). Reads only `base`, `obs` (the agent's own view) and
/// `profile`.
pub fn adjust(
    base: &[f64; 6],
    obs: &PlayerObservation,
    profile: &OpponentProfile,
    cfg: &AdaptConfig,
) -> Option<Adjustment> {
    if !cfg.enabled || cfg.validate().is_err() || !obs.is_my_turn() {
        return None;
    }
    let s = street_index(obs.street)?;
    let legal = |k: ActionKind| obs.legal_actions.iter().any(|a| a.kind == k);
    if !legal(ActionKind::Check) {
        return None; // rules only act when nothing is owed
    }
    let mut d = *base;
    for (i, p) in d.iter_mut().enumerate() {
        if !legal(ActionKind::ALL[i]) {
            *p = 0.0;
        }
    }
    let fold = strength(
        profile.fold_to_aggression[s],
        cfg.fold_threshold,
        cfg.min_samples,
    );
    let station = strength(
        profile.call_vs_aggression[s],
        cfg.station_threshold,
        cfg.min_samples,
    );
    if fold.is_none() && station.is_none() {
        return None; // cheap exit: no equity estimate needed
    }
    let e = hand_strength(obs.hole_cards, &obs.board)?;
    let mut budget = cfg.max_shift;
    let mut moved = 0.0;
    let mut rules: Vec<&'static str> = Vec::new();
    let ck = ActionKind::Check.index();
    let first_legal = |ks: [ActionKind; 3]| ks.into_iter().find(|&k| legal(k));
    let check_to = |d: &mut [f64; 6], dest: ActionKind, want: f64, budget: &mut f64| {
        let m = want.min(d[ck]).min(*budget);
        if m > 0.0 {
            d[ck] -= m;
            d[dest.index()] += m;
            *budget -= m;
        }
        m.max(0.0)
    };
    if let Some(k) = station {
        let want = cfg.max_shift * k;
        if e >= VALUE_EQUITY {
            if let Some(dest) =
                first_legal([ActionKind::Pot, ActionKind::HalfPot, ActionKind::AllIn])
            {
                let m = check_to(&mut d, dest, want, &mut budget);
                if m > 0.0 {
                    moved += m;
                    rules.push("calling station + strong hand: more value bets");
                }
            }
        } else if e <= BLUFF_EQUITY {
            let mut left = want.min(budget);
            for kind in [ActionKind::AllIn, ActionKind::Pot, ActionKind::HalfPot] {
                let take = left.min(d[kind.index()]);
                if take > 0.0 {
                    d[kind.index()] -= take;
                    d[ck] += take;
                    left -= take;
                    budget -= take;
                    moved += take;
                }
            }
            if moved > 0.0 {
                rules.push("calling station + weak hand: fewer bluffs");
            }
        }
    }
    if let Some(k) = fold
        && e < VALUE_EQUITY
        && let Some(dest) = first_legal([ActionKind::HalfPot, ActionKind::Pot, ActionKind::AllIn])
    {
        let m = check_to(&mut d, dest, cfg.max_shift * k, &mut budget);
        if m > 0.0 {
            moved += m;
            rules.push("over-folder + non-strong hand: more small bets");
        }
    }
    let rule = match rules.as_slice() {
        [] => "",
        [one] => one,
        _ => "calling station and over-folder",
    };
    if moved <= 0.0 {
        return None;
    }
    let total: f64 = d.iter().sum();
    if total <= 0.0 {
        return None;
    }
    Some(Adjustment {
        distribution: d.map(|p| p / total),
        moved,
        rule,
    })
}

/// Counters of the adaptive agent.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AdaptStats {
    pub decisions: u64,
    /// Decisions whose distribution was changed by a rule.
    pub adapted: u64,
    /// Sum of probability mass moved.
    pub mass_moved: f64,
}

/// Wraps a base agent and conservatively shifts its distribution using an
/// [`OpponentProfile`] (see the module docs). Experimental: **not evaluated
/// as stronger** than the base agent.
pub struct AdaptiveAgent<A: Agent + ?Sized> {
    cfg: AdaptConfig,
    observed: Seat,
    profile: OpponentProfile,
    rng: PokerbotRng,
    stats: AdaptStats,
    last_adapted: bool,
    base: Box<A>,
}

/// Label suffix shown while adaptation is on.
pub const ADAPTIVE_TAG: &str = "adaptive (experimental; not evaluated as stronger)";

impl<A: Agent + ?Sized> AdaptiveAgent<A> {
    /// Wraps `base`; `observed` is the seat whose public actions the profile
    /// learns from (the agent's opponent).
    pub fn new(base: Box<A>, observed: Seat, cfg: AdaptConfig) -> Result<Self, String> {
        cfg.validate()?;
        Ok(AdaptiveAgent {
            rng: PokerbotRng::seed_from_u64(cfg.seed),
            cfg,
            observed,
            profile: OpponentProfile::new(),
            stats: AdaptStats::default(),
            last_adapted: false,
            base,
        })
    }

    pub fn config(&self) -> &AdaptConfig {
        &self.cfg
    }

    pub fn is_enabled(&self) -> bool {
        self.cfg.enabled
    }

    /// Turns adaptation on or off (the profile is kept).
    pub fn set_enabled(&mut self, on: bool) {
        self.cfg.enabled = on;
    }

    pub fn profile(&self) -> &OpponentProfile {
        &self.profile
    }

    /// Clears the profile (adaptation then waits for `min_samples` again).
    pub fn reset_profile(&mut self) {
        self.profile.reset();
    }

    pub fn adapt_stats(&self) -> AdaptStats {
        self.stats
    }

    /// Whether the most recent decision was adapted.
    pub fn last_adapted(&self) -> bool {
        self.last_adapted
    }

    pub fn base(&self) -> &A {
        &self.base
    }

    /// Feeds a finished hand: the agent's own viewer-filtered events.
    pub fn observe_hand(&mut self, events: &[HandEvent]) -> Result<(), ProfileError> {
        self.profile.observe_events(self.observed, events)
    }

    /// Profile diagnostics plus the adaptation state.
    pub fn diagnostics(&self) -> Vec<String> {
        let mut out = vec![format!(
            "Adaptation {} ({ADAPTIVE_TAG}): moves at most {:.0}% of the policy's probability \
             per decision, only after {} samples of a statistic.",
            if self.cfg.enabled { "ON" } else { "OFF" },
            100.0 * self.cfg.max_shift,
            self.cfg.min_samples
        )];
        out.extend(self.profile.diagnostics());
        out.push(format!(
            "Adapted decisions: {} of {} (average mass moved when adapted: {}).",
            self.stats.adapted,
            self.stats.decisions,
            fmt_num(
                (self.stats.adapted > 0).then(|| self.stats.mass_moved / self.stats.adapted as f64)
            )
        ));
        out
    }
}

impl<A: Agent + ?Sized> Agent for AdaptiveAgent<A> {
    fn label(&self) -> String {
        if self.cfg.enabled {
            format!("{} + {ADAPTIVE_TAG}", self.base.label())
        } else {
            self.base.label()
        }
    }

    fn act(&mut self, obs: &PlayerObservation) -> Result<Decision, AgentError> {
        let d = self.base.act(obs)?;
        self.stats.decisions += 1;
        self.last_adapted = false;
        let Some(adj) = adjust(&d.distribution, obs, &self.profile, &self.cfg) else {
            return Ok(d);
        };
        let u = self.rng.gen_f64();
        let mut acc = 0.0;
        let mut pick = None;
        for (i, &p) in adj.distribution.iter().enumerate() {
            if p > 0.0 {
                acc += p;
                pick = Some(i);
                if u < acc {
                    break;
                }
            }
        }
        let slot = pick.expect("adjusted distribution has legal mass");
        self.stats.adapted += 1;
        self.stats.mass_moved += adj.moved;
        self.last_adapted = true;
        Ok(Decision {
            action: ActionKind::ALL[slot],
            distribution: adj.distribution,
            source: d.source,
        })
    }

    fn infer_stats(&self) -> Option<InferStats> {
        self.base.infer_stats()
    }
}
