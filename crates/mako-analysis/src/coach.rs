//! Post-hand coach and replay frames (M-18).
//!
//! Everything here starts from a [`HandRecord`], which holds only what the
//! human seat could know (see [`crate::history`]). Each human decision is
//! re-observed by rebuilding the human's [`PlayerObservation`] from the
//! prefix of the human's own event stream that precedes the decision, so
//! the decision-time analysis is a function of decision-time information
//! only.
//!
//! The review has three clearly separated parts:
//!
//! * **Decision time** — street, board, pot, amount to call, stacks, pot
//!   odds (exact fraction), the legal actions and the action taken, the
//!   trained policy's action mix *at the human's own observation* (supplied
//!   by a [`MixProvider`]; fallbacks are flagged), and an equity estimate vs a
//!   uniformly random hand (own cards and visible board only).
//! * **Hindsight** — only if the opponent's cards were shown at showdown:
//!   equity vs those cards on each street, labelled as not available at
//!   decision time. After a fold the section states that nothing was shown.
//! * **Caveats** — no expected-value or regret figures are computed; the
//!   policy is a coarse approximation and its mix is not a correct answer.
//!
//! [`render`] turns a review into tagged plain-text lines; the CLI wraps
//! and styles them.

use mako_core::action::{ActionKind, ActionRecord, LegalAction, Seat, Street, TerminalReason};
use mako_core::card::Card;
use mako_core::events::HandEvent;
use mako_core::observation::PlayerObservation;
use mako_core::rules::Chips;

use crate::equity::{self, EquityEstimate};
use crate::history::{HandRecord, HistoryError};

/// Where a policy mix came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MixSource {
    /// The trained average strategy for this information set.
    Policy,
    /// The policy has no trained entry; the mix is an explicit fallback
    /// (the reason, e.g. "unseen infoset").
    Fallback(String),
}

/// A policy's probabilities per action class (index = `ActionKind::index`).
#[derive(Clone, Debug, PartialEq)]
pub struct PolicyMix {
    pub probs: [f64; 6],
    pub source: MixSource,
}

/// Supplies "what the policy would play in your seat". Implementations
/// must read nothing but `obs` (the human's own observation).
pub trait MixProvider {
    /// Honest label of the policy.
    fn label(&self) -> String;
    /// Artifact checksum, if any.
    fn checksum(&self) -> Option<u64>;
    /// The mix at `obs` (`obs.to_act == Some(obs.seat)`).
    fn mix(&self, obs: &PlayerObservation) -> Result<PolicyMix, String>;
}

/// `to_call / (pot + to_call)`: the share of the final pot a call pays for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PotOdds {
    pub to_call: Chips,
    /// Chips already in the pot (including current-street bets).
    pub pot: Chips,
}

fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 { a } else { gcd(b, a % b) }
}

impl PotOdds {
    /// Reduced fraction `(numerator, denominator)`.
    pub fn fraction(&self) -> (u64, u64) {
        let n = u64::from(self.to_call);
        let d = u64::from(self.pot) + n;
        let g = gcd(n, d).max(1);
        (n / g, d / g)
    }

    /// The same as a fraction in `[0, 1]`.
    pub fn value(&self) -> f64 {
        let (n, d) = self.fraction();
        n as f64 / d as f64
    }
}

/// One human decision, from decision-time information only.
#[derive(Clone, Debug, PartialEq)]
pub struct DecisionReview {
    /// 1-based among the human's decisions.
    pub number: usize,
    pub street: Street,
    pub board: Vec<Card>,
    pub pot: Chips,
    pub to_call: Chips,
    /// Chips behind before acting, indexed by `Seat::index`.
    pub stacks: [Chips; 2],
    /// `None` when there was nothing to call.
    pub pot_odds: Option<PotOdds>,
    pub legal: Vec<LegalAction>,
    pub chosen: ActionRecord,
    /// `None` when no provider was given; `Some(Err)` if it failed.
    pub mix: Option<Result<PolicyMix, String>>,
    pub equity_vs_random: Option<EquityEstimate>,
}

/// Hindsight equity on one street.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HindsightStreet {
    pub street: Street,
    pub board: Vec<Card>,
    pub equity: Option<EquityEstimate>,
}

/// Information revealed only after the hand.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Hindsight {
    /// The opponent's cards were shown at showdown.
    Revealed {
        opponent_cards: [Card; 2],
        streets: Vec<HindsightStreet>,
    },
    /// The hand ended by a fold: nothing was shown.
    NotShown,
}

/// The full review of one hand.
#[derive(Clone, Debug, PartialEq)]
pub struct HandReview {
    pub hand_number: u64,
    pub human_seat: Seat,
    pub button: Seat,
    pub human_cards: [Card; 2],
    pub opponent: String,
    /// `(label, checksum)` of the policy used for mixes.
    pub policy: Option<(String, Option<u64>)>,
    pub decisions: Vec<DecisionReview>,
    pub reason: TerminalReason,
    pub human_payoff: i64,
    pub hindsight: Hindsight,
}

/// The human's observation before each action of the hand (both seats'
/// actions), rebuilt from the human's own event stream, plus the final
/// observation.
pub fn observations(
    record: &HandRecord,
) -> Result<(Vec<(PlayerObservation, ActionRecord)>, PlayerObservation), HistoryError> {
    let events = record.to_events();
    let bad = |e: mako_core::replay::ReplayError| HistoryError::Invalid {
        hand_number: record.hand_number,
        reason: format!("cannot rebuild observation: {e}"),
    };
    let mut out = Vec::new();
    for (i, e) in events.iter().enumerate() {
        if let HandEvent::Action(a) = e {
            let obs =
                PlayerObservation::from_events(record.human_seat, &events[..i]).map_err(bad)?;
            out.push((obs, *a));
        }
    }
    let last = PlayerObservation::from_events(record.human_seat, &events).map_err(bad)?;
    Ok((out, last))
}

fn to_call(obs: &PlayerObservation) -> Chips {
    let me = obs.seat.index();
    let owed = obs.street_contributions[1 - me].saturating_sub(obs.street_contributions[me]);
    owed.min(obs.stacks[me])
}

/// Reviews a finished hand. `policy` supplies decision-time mixes.
pub fn review(
    record: &HandRecord,
    policy: Option<&dyn MixProvider>,
) -> Result<HandReview, HistoryError> {
    let human = record.human_seat;
    let (steps, _) = observations(record)?;
    let mut decisions = Vec::new();
    for (obs, a) in steps.into_iter().filter(|(_, a)| a.seat == human) {
        let tc = to_call(&obs);
        decisions.push(DecisionReview {
            number: decisions.len() + 1,
            street: obs.street,
            board: obs.board.clone(),
            pot: obs.pot,
            to_call: tc,
            stacks: obs.stacks,
            pot_odds: (tc > 0).then_some(PotOdds {
                to_call: tc,
                pot: obs.pot,
            }),
            legal: obs.legal_actions.clone(),
            chosen: a,
            mix: policy.map(|p| p.mix(&obs)),
            equity_vs_random: equity::vs_random(obs.hole_cards, &obs.board),
        });
    }
    let hindsight = match (record.reason, record.opponent_cards) {
        (TerminalReason::Showdown, Some(opp)) => Hindsight::Revealed {
            opponent_cards: opp,
            streets: [
                (Street::Preflop, 0),
                (Street::Flop, 3),
                (Street::Turn, 4),
                (Street::River, 5),
            ]
            .into_iter()
            .filter(|&(_, n)| n <= record.board.len())
            .map(|(street, n)| HindsightStreet {
                street,
                board: record.board[..n].to_vec(),
                equity: equity::vs_hand(record.human_cards, opp, &record.board[..n]),
            })
            .collect(),
        },
        _ => Hindsight::NotShown,
    };
    Ok(HandReview {
        hand_number: record.hand_number,
        human_seat: human,
        button: record.button,
        human_cards: record.human_cards,
        opponent: record.opponent.name.clone(),
        policy: policy.map(|p| (p.label(), p.checksum())),
        decisions,
        reason: record.reason,
        human_payoff: record.payoffs[human.index()],
        hindsight,
    })
}

// ------------------------------------------------------------- rendering

/// Which part of the review a line belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Section {
    /// Hand number, seat, own cards, opponent label.
    Header,
    /// Decision-time information only.
    DecisionTime,
    /// Revealed after the hand; not available when deciding.
    Hindsight,
    /// The result of the hand.
    Result,
    /// Limitations of the analysis.
    Caveat,
}

/// Visual role of a line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LineKind {
    Heading,
    Text,
    /// Something the reader should notice (e.g. a policy fallback).
    Warn,
    /// Muted explanatory text.
    Note,
}

/// One rendered line (unwrapped; the caller wraps to its width).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoachLine {
    pub section: Section,
    pub kind: LineKind,
    /// Indentation level (0 or 1).
    pub depth: u8,
    pub text: String,
}

/// Lowercase action-class name for mixes.
pub fn kind_label(k: ActionKind) -> &'static str {
    match k {
        ActionKind::Fold => "fold",
        ActionKind::Check => "check",
        ActionKind::Call => "call",
        ActionKind::HalfPot => "half-pot",
        ActionKind::Pot => "pot",
        ActionKind::AllIn => "all-in",
    }
}

/// Describes an action of the human (second person) with exact amounts.
pub fn describe_action(
    kind: ActionKind,
    raise_to: Chips,
    committed: Chips,
    to_call: Chips,
) -> String {
    let tag = match kind {
        ActionKind::HalfPot => " (half pot)",
        ActionKind::Pot => " (pot)",
        ActionKind::AllIn => " (all-in)",
        _ => "",
    };
    match kind {
        ActionKind::Fold => "fold".into(),
        ActionKind::Check => "check".into(),
        ActionKind::Call => format!("call {committed}"),
        _ if to_call == 0 && committed == raise_to => format!("bet {raise_to}{tag}"),
        _ if to_call == 0 => format!("bet {committed}, to {raise_to}{tag}"),
        _ => format!("raise to {raise_to}{tag}"),
    }
}

fn cards(cs: &[Card]) -> String {
    if cs.is_empty() {
        "none".into()
    } else {
        cs.iter()
            .map(|c| c.to_string())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

fn street_title(s: Street) -> &'static str {
    match s {
        Street::Preflop => "Preflop",
        Street::Flop => "Flop",
        Street::Turn => "Turn",
        Street::River => "River",
        Street::Showdown => "Showdown",
    }
}

/// The fixed caveat text (also used by tests to check wording).
pub const CAVEAT: &str = "Caveats: the Mako policy is a coarse CFR approximation \
    over a six-action abstraction, trained on a small budget; its mix is not a \
    correct answer and is not GTO. This review computes no expected value and no \
    regret. Pot odds and equity ignore future betting; sampled equity is an \
    estimate.";

/// Renders a review as tagged lines.
pub fn render(r: &HandReview) -> Vec<CoachLine> {
    let mut out = Vec::new();
    let mut push = |section, kind, depth, text: String| {
        out.push(CoachLine {
            section,
            kind,
            depth,
            text,
        })
    };
    let me = r.human_seat.index();
    let opp = 1 - me;
    let pos = if r.button == r.human_seat {
        "the button (small blind)"
    } else {
        "the big blind"
    };
    push(
        Section::Header,
        LineKind::Heading,
        0,
        format!("Analysis of hand {}", r.hand_number),
    );
    push(
        Section::Header,
        LineKind::Text,
        1,
        format!(
            "You were {pos} with {}. Opponent: {}.",
            cards(&r.human_cards),
            r.opponent
        ),
    );
    push(
        Section::DecisionTime,
        LineKind::Heading,
        0,
        "At decision time (only what you could see when acting):".into(),
    );
    match &r.policy {
        Some((label, ck)) => push(
            Section::DecisionTime,
            LineKind::Note,
            1,
            format!(
                "Policy mixes are what the Mako policy would play in your seat: {label}{}.",
                ck.map_or(String::new(), |c| format!(", artifact checksum {c:#018x}"))
            ),
        ),
        None => push(
            Section::DecisionTime,
            LineKind::Note,
            1,
            "No trained policy is available, so there is no action mix to compare.".into(),
        ),
    }
    if r.decisions.is_empty() {
        push(
            Section::DecisionTime,
            LineKind::Text,
            1,
            "You made no decisions this hand (the opponent acted first and the hand ended).".into(),
        );
    }
    for d in &r.decisions {
        push(
            Section::DecisionTime,
            LineKind::Heading,
            1,
            format!(
                "Decision {} -- {}, board {}: pot {}, to call {}, stacks you {} / bot {}",
                d.number,
                street_title(d.street),
                cards(&d.board),
                d.pot,
                d.to_call,
                d.stacks[me],
                d.stacks[opp]
            ),
        );
        let odds = match d.pot_odds {
            Some(po) => {
                let (n, den) = po.fraction();
                format!(
                    "Pot odds: call {} into {} = {n}/{den} = {:.1}% of the final pot; a call \
                     breaks even with at least that equity (ignoring future betting).",
                    po.to_call,
                    po.pot,
                    100.0 * po.value()
                )
            }
            None => "Pot odds: n/a (nothing to call).".into(),
        };
        push(Section::DecisionTime, LineKind::Text, 1, odds);
        let legal: Vec<String> = d
            .legal
            .iter()
            .map(|a| describe_action(a.kind, a.raise_to, a.chips_to_commit, d.to_call))
            .collect();
        push(
            Section::DecisionTime,
            LineKind::Text,
            1,
            format!(
                "Legal: {}. You chose: {}.",
                legal.join(" | "),
                describe_action(
                    d.chosen.kind,
                    d.chosen.raise_to,
                    d.chosen.chips_committed,
                    d.to_call
                )
            ),
        );
        match &d.mix {
            None => {}
            Some(Err(e)) => push(
                Section::DecisionTime,
                LineKind::Warn,
                1,
                format!("Policy mix unavailable: {e}."),
            ),
            Some(Ok(m)) => {
                let parts: Vec<String> = d
                    .legal
                    .iter()
                    .map(|a| {
                        format!(
                            "{} {:.1}%",
                            kind_label(a.kind),
                            100.0 * m.probs[a.kind.index()]
                        )
                    })
                    .collect();
                let mine = 100.0 * m.probs[d.chosen.kind.index()];
                match &m.source {
                    MixSource::Policy => push(
                        Section::DecisionTime,
                        LineKind::Text,
                        1,
                        format!(
                            "Policy mix: {}. Your action had {mine:.1}% weight.",
                            parts.join(" | ")
                        ),
                    ),
                    MixSource::Fallback(why) => push(
                        Section::DecisionTime,
                        LineKind::Warn,
                        1,
                        format!(
                            "Policy FALLBACK ({why}): the policy has no trained mix here and \
                             would play uniformly over the legal actions ({}).",
                            parts.join(" | ")
                        ),
                    ),
                }
            }
        }
        let eq = match &d.equity_vs_random {
            Some(e) => e.describe(),
            None => "n/a".into(),
        };
        push(
            Section::DecisionTime,
            LineKind::Text,
            1,
            format!("Your equity estimate vs a random hand: {eq} -- not vs the bot's range."),
        );
    }

    push(
        Section::Hindsight,
        LineKind::Heading,
        0,
        "Hindsight (not available at decision time):".into(),
    );
    match &r.hindsight {
        Hindsight::NotShown => push(
            Section::Hindsight,
            LineKind::Text,
            1,
            "The bot's cards were not shown (the hand ended by a fold), so there is no \
             hindsight equity."
                .into(),
        ),
        Hindsight::Revealed {
            opponent_cards,
            streets,
        } => {
            push(
                Section::Hindsight,
                LineKind::Text,
                1,
                format!("The bot showed {} at showdown.", cards(opponent_cards)),
            );
            for s in streets {
                push(
                    Section::Hindsight,
                    LineKind::Text,
                    1,
                    format!(
                        "{} (board {}): your equity vs the shown cards {}",
                        street_title(s.street),
                        cards(&s.board),
                        s.equity.map_or("n/a".into(), |e| e.describe())
                    ),
                );
            }
        }
    }

    let how = match r.reason {
        TerminalReason::Showdown => "at showdown",
        TerminalReason::Fold { winner } if winner == r.human_seat => "after the bot folded",
        TerminalReason::Fold { .. } => "after you folded",
    };
    let res = match r.human_payoff {
        p if p > 0 => format!("Result: you won {p} chips {how}."),
        p if p < 0 => format!("Result: you lost {} chips {how}.", -p),
        _ => format!("Result: you broke even {how} (split pot)."),
    };
    push(Section::Result, LineKind::Text, 0, res);
    let mut caveat = CAVEAT.to_string();
    if r.decisions.iter().any(|d| {
        matches!(
            &d.mix,
            Some(Ok(PolicyMix {
                source: MixSource::Fallback(_),
                ..
            }))
        )
    }) {
        caveat.push_str(" Decisions marked FALLBACK had no trained policy entry.");
    }
    push(Section::Caveat, LineKind::Note, 0, caveat);
    out
}

// ---------------------------------------------------------------- replay

/// One step of a hand replay, from the human's view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplayFrame {
    /// 1-based step.
    pub step: usize,
    pub street: Street,
    pub board: Vec<Card>,
    /// Pot and stacks before the action.
    pub pot: Chips,
    pub stacks: [Chips; 2],
    /// Street contributions before the action, indexed by `Seat::index`.
    pub street_contributions: [Chips; 2],
    pub action: ActionRecord,
    /// `to_call` the actor faced.
    pub to_call: Chips,
}

/// Every action of the hand as a frame, plus the final observation.
pub fn replay_frames(
    record: &HandRecord,
) -> Result<(Vec<ReplayFrame>, PlayerObservation), HistoryError> {
    let (steps, last) = observations(record)?;
    let frames = steps
        .into_iter()
        .enumerate()
        .map(|(i, (obs, a))| {
            let actor = a.seat.index();
            let owed = obs.street_contributions[1 - actor]
                .saturating_sub(obs.street_contributions[actor])
                .min(obs.stacks[actor]);
            ReplayFrame {
                step: i + 1,
                street: obs.street,
                board: obs.board,
                pot: obs.pot,
                stacks: obs.stacks,
                street_contributions: obs.street_contributions,
                action: a,
                to_call: owed,
            }
        })
        .collect();
    Ok((frames, last))
}
