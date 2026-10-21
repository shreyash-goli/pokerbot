//! Hand logs, deterministic replay, and public-state reconstruction from
//! event streams.
//!
//! * [`HandLog`] — config + starting stacks + the nine cards the hand uses
//!   (deck positions 0–8) + the action list. [`HandLog::replay`] rebuilds the
//!   identical full [`HandState`]. Serializes to a versioned, line-based text
//!   format (`pokerbot-hand-log v1`) with no dependencies.
//! * [`PublicReplayer`] — rebuilds the public state of a hand from a
//!   viewer-filtered event stream alone (no deck, no hidden cards), validating
//!   every event. It drives a *shadow* [`HandState`] dealt from a fixed
//!   placeholder deck: betting dynamics (legal actions, streets, to-act,
//!   stacks, pot) do not depend on cards, so the authoritative rules are
//!   reused rather than duplicated. The shadow's cards are never exposed;
//!   showdown results come from the stream's `Showdown`/`HandEnded` events,
//!   which are re-verified (ranks re-evaluated, settlement recomputed).
//!
//! Divergent input (illegal or tampered actions, malformed text, tampered
//! events) yields a [`ReplayError`], never a panic.

use std::fmt;
use std::str::FromStr;

use crate::action::{
    ActionError, ActionKind, HandError, LegalAction, Seat, Street, TerminalReason,
};
use crate::card::{Card, CardSet};
use crate::deck::Deck;
use crate::eval::{evaluate5, evaluate7};
use crate::events::HandEvent;
use crate::observation::{PlayerObservation, PublicView};
use crate::rules::{self, Chips, HandResult, Position, RulesConfig};
use crate::state::{HandConfig, HandState, ShowdownHand};

/// Header line of the v1 text format.
pub const LOG_HEADER_V1: &str = "pokerbot-hand-log v1";

/// Errors from parsing, replaying or reconstructing a hand.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReplayError {
    /// Malformed text log (1-based line number).
    Parse { line: usize, reason: String },
    /// A `pokerbot-hand-log` header with a version other than `v1`.
    UnsupportedVersion(String),
    /// A card appears twice in a log or event stream.
    DuplicateCard(Card),
    /// The hand could not be created (config, stacks, deck size).
    Hand(HandError),
    /// Logged action `index` (0-based) was rejected by the state machine.
    Action { index: usize, error: ActionError },
    /// A captured log does not reproduce the supplied state.
    Diverged,
    /// Event `index` (0-based) is inconsistent with the stream so far.
    Event { index: usize, reason: &'static str },
    /// The event stream ended in an inconsistent place.
    Incomplete(&'static str),
}

impl fmt::Display for ReplayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReplayError::Parse { line, reason } => write!(f, "hand log line {line}: {reason}"),
            ReplayError::UnsupportedVersion(v) => write!(f, "unsupported hand log version {v:?}"),
            ReplayError::DuplicateCard(c) => write!(f, "duplicate card {c}"),
            ReplayError::Hand(e) => write!(f, "cannot start hand: {e}"),
            ReplayError::Action { index, error } => {
                write!(f, "logged action {index} rejected: {error}")
            }
            ReplayError::Diverged => write!(f, "log does not reproduce the state"),
            ReplayError::Event { index, reason } => write!(f, "event {index}: {reason}"),
            ReplayError::Incomplete(r) => write!(f, "incomplete event stream: {r}"),
        }
    }
}

impl std::error::Error for ReplayError {}

// ------------------------------------------------------------------ HandLog

/// Everything needed to reproduce a hand exactly.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HandLog {
    pub config: HandConfig,
    /// Indexed by [`Seat::index`].
    pub starting_stacks: [Chips; 2],
    /// The nine dealt cards in deck order: BB, BTN, BB, BTN, then the board.
    pub cards: [Card; 9],
    /// Actions in order, with exact amounts (verified on replay).
    pub actions: Vec<LegalAction>,
}

impl HandLog {
    /// A log with no actions. Validates config, stacks and card uniqueness.
    pub fn new(
        config: HandConfig,
        starting_stacks: [Chips; 2],
        cards: [Card; 9],
    ) -> Result<HandLog, ReplayError> {
        if let Err(d) = CardSet::try_from_cards(&cards) {
            return Err(ReplayError::DuplicateCard(d.0));
        }
        let log = HandLog {
            config,
            starting_stacks,
            cards,
            actions: Vec::new(),
        };
        log.initial_state()?;
        Ok(log)
    }

    /// A log using the next nine cards of `deck`.
    pub fn from_deck(
        config: HandConfig,
        starting_stacks: [Chips; 2],
        deck: &Deck,
    ) -> Result<HandLog, ReplayError> {
        let rest = deck.remaining_cards();
        let cards: [Card; 9] =
            rest.get(..9)
                .and_then(|s| s.try_into().ok())
                .ok_or(ReplayError::Hand(HandError::DeckTooSmall {
                    remaining: rest.len(),
                }))?;
        HandLog::new(config, starting_stacks, cards)
    }

    /// A log for `HandState::from_seed(config, seed)` (equal stacks).
    pub fn from_seed(config: HandConfig, seed: u64) -> Result<HandLog, ReplayError> {
        let s = config.rules.starting_stack;
        HandLog::from_deck(config, [s, s], &Deck::from_seed(seed))
    }

    /// Captures a played hand: `deck` and `starting_stacks` must be what the
    /// hand was created with. Fails with [`ReplayError::Diverged`] unless
    /// replaying the captured log reproduces `state` exactly.
    pub fn capture(
        state: &HandState,
        starting_stacks: [Chips; 2],
        deck: &Deck,
    ) -> Result<HandLog, ReplayError> {
        let mut log = HandLog::from_deck(*state.config(), starting_stacks, deck)?;
        log.actions = state
            .history()
            .iter()
            .map(|r| LegalAction {
                kind: r.kind,
                raise_to: r.raise_to,
                chips_to_commit: r.chips_committed,
            })
            .collect();
        if log.replay()? == *state {
            Ok(log)
        } else {
            Err(ReplayError::Diverged)
        }
    }

    /// The state before any action.
    pub fn initial_state(&self) -> Result<HandState, ReplayError> {
        let deck =
            Deck::from_order(self.cards.to_vec()).map_err(|d| ReplayError::DuplicateCard(d.0))?;
        HandState::new_with_stacks(self.config, deck, self.starting_stacks)
            .map_err(ReplayError::Hand)
    }

    /// Replays every action (each must match a legal action exactly).
    pub fn replay(&self) -> Result<HandState, ReplayError> {
        let mut st = self.initial_state()?;
        for (index, a) in self.actions.iter().enumerate() {
            st.apply_legal(a)
                .map_err(|error| ReplayError::Action { index, error })?;
        }
        Ok(st)
    }

    /// Serializes to the `pokerbot-hand-log v1` text format:
    ///
    /// ```text
    /// pokerbot-hand-log v1
    /// rules <small_blind> <big_blind> <starting_stack>
    /// button <0|1>
    /// stacks <seat0> <seat1>
    /// cards <9 cards, deck order>
    /// action <fold|check|call|halfpot|pot|allin> <raise_to> <chips>
    /// end
    /// ```
    pub fn to_text(&self) -> String {
        let r = self.config.rules;
        let mut s = format!(
            "{LOG_HEADER_V1}\nrules {} {} {}\nbutton {}\nstacks {} {}\ncards",
            r.small_blind,
            r.big_blind,
            r.starting_stack,
            self.config.button.index(),
            self.starting_stacks[0],
            self.starting_stacks[1]
        );
        for c in &self.cards {
            s.push(' ');
            s.push_str(&c.to_string());
        }
        s.push('\n');
        for a in &self.actions {
            s.push_str(&format!(
                "action {} {} {}\n",
                kind_name(a.kind),
                a.raise_to,
                a.chips_to_commit
            ));
        }
        s.push_str("end\n");
        s
    }

    /// Parses the v1 text format strictly (see [`HandLog::to_text`]).
    /// Validates structure, numbers, config, stacks and card uniqueness;
    /// action legality is checked by [`HandLog::replay`].
    pub fn parse(text: &str) -> Result<HandLog, ReplayError> {
        let mut lines = text.lines().enumerate().map(|(i, l)| (i + 1, l));
        let perr = |line: usize, reason: &str| ReplayError::Parse {
            line,
            reason: reason.to_string(),
        };

        let (_, header) = lines.next().ok_or_else(|| perr(1, "empty log"))?;
        if header != LOG_HEADER_V1 {
            return match header.strip_prefix("pokerbot-hand-log ") {
                Some(v) => Err(ReplayError::UnsupportedVersion(v.to_string())),
                None => Err(perr(1, "missing 'pokerbot-hand-log v1' header")),
            };
        }
        let mut field = |key: &str, n: usize| -> Result<(usize, Vec<&str>), ReplayError> {
            let (ln, l) = lines
                .next()
                .ok_or_else(|| perr(0, &format!("missing '{key}' line")))?;
            let toks: Vec<&str> = l.split(' ').collect();
            if toks[0] != key || toks.len() != n + 1 {
                return Err(perr(ln, &format!("expected '{key}' with {n} values")));
            }
            Ok((ln, toks[1..].to_vec()))
        };
        let (ln, r) = field("rules", 3)?;
        let rules = RulesConfig {
            small_blind: num(ln, r[0])?,
            big_blind: num(ln, r[1])?,
            starting_stack: num(ln, r[2])?,
        };
        let (ln, b) = field("button", 1)?;
        let button = match b[0] {
            "0" => Seat::Zero,
            "1" => Seat::One,
            _ => return Err(perr(ln, "button must be 0 or 1")),
        };
        let (ln, st) = field("stacks", 2)?;
        let stacks = [num(ln, st[0])?, num(ln, st[1])?];
        let (ln, cs) = field("cards", 9)?;
        let mut cards = [Card::from_index(0).expect("valid"); 9];
        for (slot, tok) in cards.iter_mut().zip(&cs) {
            *slot = tok
                .parse::<Card>()
                .map_err(|_| perr(ln, &format!("invalid card {tok:?}")))?;
        }

        let mut actions = Vec::new();
        let mut ended = false;
        for (ln, l) in lines.by_ref() {
            if l == "end" {
                ended = true;
                break;
            }
            let toks: Vec<&str> = l.split(' ').collect();
            if toks.len() != 4 || toks[0] != "action" {
                return Err(perr(
                    ln,
                    "expected 'action <kind> <raise_to> <chips>' or 'end'",
                ));
            }
            let kind = parse_kind(toks[1]).ok_or_else(|| perr(ln, "unknown action kind"))?;
            actions.push(LegalAction {
                kind,
                raise_to: num(ln, toks[2])?,
                chips_to_commit: num(ln, toks[3])?,
            });
        }
        if !ended {
            return Err(perr(0, "missing 'end' line (truncated log)"));
        }
        if let Some((ln, _)) = lines.next() {
            return Err(perr(ln, "content after 'end'"));
        }
        let mut log = HandLog::new(HandConfig { rules, button }, stacks, cards)?;
        log.actions = actions;
        Ok(log)
    }
}

impl fmt::Display for HandLog {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_text())
    }
}

impl FromStr for HandLog {
    type Err = ReplayError;
    fn from_str(s: &str) -> Result<HandLog, ReplayError> {
        HandLog::parse(s)
    }
}

/// Replays a log into the identical full state.
pub fn replay(log: &HandLog) -> Result<HandState, ReplayError> {
    log.replay()
}

fn num(line: usize, tok: &str) -> Result<Chips, ReplayError> {
    if tok.is_empty() || !tok.bytes().all(|b| b.is_ascii_digit()) || tok.len() > 10 {
        return Err(ReplayError::Parse {
            line,
            reason: format!("invalid number {tok:?}"),
        });
    }
    tok.parse().map_err(|_| ReplayError::Parse {
        line,
        reason: format!("number out of range {tok:?}"),
    })
}

fn kind_name(k: ActionKind) -> &'static str {
    match k {
        ActionKind::Fold => "fold",
        ActionKind::Check => "check",
        ActionKind::Call => "call",
        ActionKind::HalfPot => "halfpot",
        ActionKind::Pot => "pot",
        ActionKind::AllIn => "allin",
    }
}

fn parse_kind(s: &str) -> Option<ActionKind> {
    ActionKind::ALL.into_iter().find(|&k| kind_name(k) == s)
}

// ------------------------------------------------------- PublicReplayer

#[derive(Clone, Debug, PartialEq, Eq)]
struct Ended {
    reason: TerminalReason,
    payoffs: [i64; 2],
    final_stacks: [Chips; 2],
}

/// Incrementally rebuilds and validates the public state of a hand from a
/// (viewer-filtered or omniscient) event stream. See the module docs.
#[derive(Clone, Debug)]
pub struct PublicReplayer {
    shadow: Option<HandState>,
    board: Vec<Card>,
    hole: [Option<[Card; 2]>; 2],
    showdown: Option<[ShowdownHand; 2]>,
    ended: Option<Ended>,
    seen: CardSet,
    index: usize,
}

impl Default for PublicReplayer {
    fn default() -> Self {
        PublicReplayer::new()
    }
}

impl PublicReplayer {
    /// An empty replayer expecting `HandStarted` first.
    pub fn new() -> PublicReplayer {
        PublicReplayer {
            shadow: None,
            board: Vec::new(),
            hole: [None; 2],
            showdown: None,
            ended: None,
            seen: CardSet::new(),
            index: 0,
        }
    }

    fn see(&mut self, cards: &[Card]) -> Result<(), ReplayError> {
        for &c in cards {
            if !self.seen.insert(c) {
                return Err(ReplayError::DuplicateCard(c));
            }
        }
        Ok(())
    }

    /// Applies one event, validating it against everything seen so far.
    pub fn apply_event(&mut self, event: &HandEvent) -> Result<(), ReplayError> {
        let index = self.index;
        let bad = |reason: &'static str| ReplayError::Event { index, reason };
        if self.ended.is_some() {
            return Err(bad("event after HandEnded"));
        }
        match event {
            HandEvent::HandStarted {
                rules,
                button,
                starting_stacks,
                blinds_posted,
            } => {
                if self.shadow.is_some() {
                    return Err(bad("duplicate HandStarted"));
                }
                let config = HandConfig {
                    rules: *rules,
                    button: *button,
                };
                let placeholder: Vec<Card> = Card::all().take(9).collect();
                let deck = Deck::from_order(placeholder).expect("distinct placeholder cards");
                let shadow = HandState::new_with_stacks(config, deck, *starting_stacks)
                    .map_err(ReplayError::Hand)?;
                if Seat::ALL.map(|s| shadow.total_contribution(s)) != *blinds_posted {
                    return Err(bad("blinds do not match the config"));
                }
                self.shadow = Some(shadow);
            }
            _ if self.shadow.is_none() => return Err(bad("first event must be HandStarted")),
            HandEvent::HoleCards { seat, cards } => {
                let shadow = self.shadow.as_ref().expect("checked");
                if !shadow.history().is_empty() || !self.board.is_empty() {
                    return Err(bad("hole cards dealt after betting began"));
                }
                if self.hole[seat.index()].is_some() {
                    return Err(bad("duplicate HoleCards for a seat"));
                }
                self.see(cards)?;
                self.hole[seat.index()] = Some(*cards);
            }
            HandEvent::Action(r) => {
                let shadow = self.shadow.as_mut().expect("checked");
                if shadow.is_terminal() {
                    return Err(bad("action after the hand ended"));
                }
                if self.board.len() != shadow.board().len() {
                    return Err(bad("board event missing before action"));
                }
                if Some(r.seat) != shadow.to_act() || r.street != shadow.street() {
                    return Err(bad("action out of turn or on the wrong street"));
                }
                let a = LegalAction {
                    kind: r.kind,
                    raise_to: r.raise_to,
                    chips_to_commit: r.chips_committed,
                };
                shadow
                    .apply_legal(&a)
                    .map_err(|_| bad("action is not legal here"))?;
            }
            HandEvent::BoardDealt { street, cards } => {
                let (expect_street, n) = match self.board.len() {
                    0 => (Street::Flop, 3),
                    3 => (Street::Turn, 1),
                    4 => (Street::River, 1),
                    _ => return Err(bad("board already complete")),
                };
                if *street != expect_street || cards.len() != n {
                    return Err(bad("wrong board street or card count"));
                }
                let shadow = self.shadow.as_ref().expect("checked");
                if shadow.board().len() < self.board.len() + n {
                    return Err(bad("board dealt before its street"));
                }
                self.see(cards)?;
                self.board.extend_from_slice(cards);
            }
            HandEvent::Showdown { hands } => {
                let shadow = self.shadow.as_ref().expect("checked");
                if shadow.terminal_reason() != Some(TerminalReason::Showdown)
                    || self.board.len() != 5
                    || self.showdown.is_some()
                {
                    return Err(bad("showdown not reached"));
                }
                for seat in Seat::ALL {
                    let h = &hands[seat.index()];
                    if h.seat != seat {
                        return Err(bad("showdown hands out of seat order"));
                    }
                    match self.hole[seat.index()] {
                        Some(known) if known != h.hole_cards => {
                            return Err(bad("showdown cards differ from dealt cards"));
                        }
                        Some(_) => {}
                        None => {
                            self.see(&h.hole_cards)?;
                            self.hole[seat.index()] = Some(h.hole_cards);
                        }
                    }
                    let b = &self.board;
                    let seven = [
                        h.hole_cards[0],
                        h.hole_cards[1],
                        b[0],
                        b[1],
                        b[2],
                        b[3],
                        b[4],
                    ];
                    if evaluate7(&seven) != h.rank
                        || evaluate5(&h.best_five) != h.rank
                        || !h.best_five.iter().all(|c| seven.contains(c))
                    {
                        return Err(bad("showdown rank or best five is wrong"));
                    }
                }
                self.showdown = Some(*hands);
            }
            HandEvent::HandEnded {
                reason,
                payoffs,
                final_stacks,
            } => {
                let shadow = self.shadow.as_ref().expect("checked");
                if !shadow.is_terminal() || self.board.len() != shadow.board().len() {
                    return Err(bad("hand ended prematurely"));
                }
                let expected = match (shadow.terminal_reason(), &self.showdown) {
                    (Some(r @ TerminalReason::Fold { .. }), _) => Ended {
                        reason: r,
                        payoffs: shadow.payoffs().expect("terminal"),
                        final_stacks: shadow.final_stacks().expect("terminal"),
                    },
                    (Some(TerminalReason::Showdown), Some(h)) => {
                        showdown_result(shadow, h[0].rank.cmp(&h[1].rank))
                    }
                    _ => return Err(bad("HandEnded without a Showdown event")),
                };
                let got = Ended {
                    reason: *reason,
                    payoffs: *payoffs,
                    final_stacks: *final_stacks,
                };
                if got != expected {
                    return Err(bad("result does not match the hand"));
                }
                self.ended = Some(got);
            }
        }
        self.index += 1;
        Ok(())
    }

    /// Checks that the stream so far is a complete prefix (every revealed
    /// board card and, if terminal, the result have been delivered).
    pub fn check_complete(&self) -> Result<(), ReplayError> {
        let shadow = self
            .shadow
            .as_ref()
            .ok_or(ReplayError::Incomplete("no HandStarted event"))?;
        if self.board.len() != shadow.board().len() {
            return Err(ReplayError::Incomplete("board events missing"));
        }
        if shadow.is_terminal() && self.ended.is_none() {
            return Err(ReplayError::Incomplete(
                "hand is over but HandEnded missing",
            ));
        }
        Ok(())
    }

    /// The rebuilt public view (call [`PublicReplayer::check_complete`]
    /// first, or use [`rebuild_public`]).
    pub fn view(&self) -> Result<PublicView, ReplayError> {
        self.check_complete()?;
        let s = self.shadow.as_ref().expect("checked by check_complete");
        let rules = s.config().rules;
        Ok(PublicView {
            button: s.button(),
            street: s.street(),
            board: self.board.clone(),
            pot: s.pot(),
            stacks: match &self.ended {
                Some(e) => e.final_stacks,
                None => Seat::ALL.map(|x| s.stack(x)),
            },
            street_contributions: Seat::ALL.map(|x| s.street_contribution(x)),
            small_blind: rules.small_blind,
            big_blind: rules.big_blind,
            history: s.history().to_vec(),
            to_act: s.to_act(),
            terminal: s.terminal_reason(),
            payoffs: self.ended.as_ref().map(|e| e.payoffs),
        })
    }

    /// Hole cards seen so far for `seat` (own deal or showdown reveal).
    pub fn known_hole_cards(&self, seat: Seat) -> Option<[Card; 2]> {
        self.hole[seat.index()]
    }

    /// Showdown hands, if a Showdown event was seen.
    pub fn showdown_hands(&self) -> Option<[ShowdownHand; 2]> {
        self.showdown
    }

    /// Legal actions in the rebuilt state (card-independent).
    pub fn legal_actions(&self) -> Vec<LegalAction> {
        self.shadow
            .as_ref()
            .map(|s| s.legal_actions())
            .unwrap_or_default()
    }
}

/// Recomputes a showdown result from the shadow's contributions.
fn showdown_result(shadow: &HandState, cmp: std::cmp::Ordering) -> Ended {
    let btn = shadow.button();
    let pos = |seat: Seat| {
        if seat == btn {
            Position::Button
        } else {
            Position::BigBlind
        }
    };
    let winner = match cmp {
        std::cmp::Ordering::Greater => Some(pos(Seat::Zero)),
        std::cmp::Ordering::Less => Some(pos(Seat::One)),
        std::cmp::Ordering::Equal => None,
    };
    let contrib = [
        shadow.total_contribution(btn),
        shadow.total_contribution(btn.other()),
    ];
    let st = rules::settle(contrib, HandResult::Showdown { winner });
    let mut final_stacks = [0; 2];
    for seat in Seat::ALL {
        let p = pos(seat).index();
        // Before settlement the shadow's stacks are the chips behind; after
        // its own (placeholder-card) settlement they are not, so rebuild
        // from starting stack minus contribution.
        let behind = shadow.starting_stack(seat) - shadow.total_contribution(seat);
        final_stacks[seat.index()] = behind + st.refunds[p] + st.awards[p];
    }
    let payoffs =
        Seat::ALL.map(|s| i64::from(final_stacks[s.index()]) - i64::from(shadow.starting_stack(s)));
    Ended {
        reason: TerminalReason::Showdown,
        payoffs,
        final_stacks,
    }
}

/// Rebuilds the public view from a complete event stream (any viewer).
pub fn rebuild_public(events: &[HandEvent]) -> Result<PublicView, ReplayError> {
    let mut r = PublicReplayer::new();
    for e in events {
        r.apply_event(e)?;
    }
    r.view()
}

/// Implementation of [`PlayerObservation::from_events`]. Rejects streams
/// that contain the opponent's private `HoleCards` (i.e. not a seat stream).
pub(crate) fn observation_from_events(
    seat: Seat,
    events: &[HandEvent],
) -> Result<PlayerObservation, ReplayError> {
    let mut r = PublicReplayer::new();
    for (index, e) in events.iter().enumerate() {
        if matches!(e, HandEvent::HoleCards { seat: s, .. } if *s != seat) {
            return Err(ReplayError::Event {
                index,
                reason: "stream contains the opponent's private hole cards",
            });
        }
        r.apply_event(e)?;
    }
    if !events
        .iter()
        .any(|e| matches!(e, HandEvent::HoleCards { seat: s, .. } if *s == seat))
    {
        return Err(ReplayError::Incomplete(
            "stream lacks the seat's own hole cards",
        ));
    }
    let view = r.view()?;
    let hole_cards = r.known_hole_cards(seat).ok_or(ReplayError::Incomplete(
        "stream lacks the seat's own hole cards",
    ))?;
    let opponent_hole_cards = r
        .showdown_hands()
        .map(|h| h[seat.other().index()].hole_cards);
    Ok(PlayerObservation {
        seat,
        button: view.button,
        street: view.street,
        hole_cards,
        board: view.board,
        pot: view.pot,
        stacks: view.stacks,
        street_contributions: view.street_contributions,
        small_blind: view.small_blind,
        big_blind: view.big_blind,
        history: view.history,
        to_act: view.to_act,
        legal_actions: if view.to_act == Some(seat) {
            r.legal_actions()
        } else {
            Vec::new()
        },
        terminal: view.terminal,
        payoffs: view.payoffs,
        opponent_hole_cards,
    })
}
