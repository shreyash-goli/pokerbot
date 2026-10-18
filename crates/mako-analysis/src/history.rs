//! Compact per-hand history records built from the **human's** view.
//!
//! A [`HandRecord`] is derived exclusively from the human seat's
//! viewer-filtered event stream (`HandState::events_for(Viewer::Seat(human))`).
//! It therefore contains:
//!
//! * the human's own hole cards,
//! * the opponent's hole cards **only** if they were revealed at showdown,
//! * the board exactly as revealed (no undealt cards after a fold),
//! * the public action history, blinds, stacks and the result.
//!
//! The omniscient log and the opponent's unrevealed cards are never stored.
//! [`HandRecord::from_events`] rejects any stream containing the opponent's
//! private `HoleCards` event.
//!
//! # Text format (`mako-history v1`)
//!
//! Line based, single spaces, `\n` line endings, written and parsed by this
//! module only (no external dependencies). A file is the header line followed
//! by zero or more hand blocks:
//!
//! ```text
//! mako-history v1
//! hand <n>                          (n >= 1, strictly increasing in a file)
//! rules <small_blind> <big_blind> <starting_stack>
//! button <0|1>
//! human <0|1>
//! stacks <seat0> <seat1>            (starting stacks)
//! opponent <checksum|-> <label...>  (checksum: 16 lowercase hex digits)
//! hole <card> <card>                (the human's own cards)
//! shown <card> <card> | shown -     (opponent cards, showdown only)
//! board <cards...> | board -        (revealed board, 0/3/4/5 cards)
//! action <seat> <street> <kind> <raise_to> <chips>   (zero or more)
//! result fold <winner> | result showdown
//! payoffs <seat0> <seat1>
//! final <seat0> <seat1>
//! end
//! ```
//!
//! `street` is one of `preflop flop turn river`; `kind` one of
//! `fold check call halfpot pot allin`. Parsing is strict: unknown keys,
//! wrong token counts, bad numbers, a missing `end` (truncation), trailing
//! garbage or an unsupported version are errors. After the syntax check every
//! block is re-validated semantically by replaying it through
//! [`mako_core::replay::PublicReplayer`] (legality of every action, board
//! timing, showdown ranks, payoffs and final stacks). Parsing never panics.

use std::fmt;

use mako_core::action::{ActionKind, ActionRecord, Seat, Street, TerminalReason};
use mako_core::card::Card;
use mako_core::eval::{best_five, evaluate7};
use mako_core::events::{HandEvent, Viewer};
use mako_core::observation::PlayerObservation;
use mako_core::rules::{self, Chips, RulesConfig};
use mako_core::state::{HandState, ShowdownHand};

/// Header line of the v1 history format.
pub const HISTORY_HEADER_V1: &str = "mako-history v1";
/// Prefix shared by every version of the header.
pub const HISTORY_HEADER_PREFIX: &str = "mako-history ";
/// Longest accepted opponent label, in bytes.
pub const MAX_LABEL_LEN: usize = 200;
/// Most actions accepted in one hand block (real hands have far fewer).
const MAX_ACTIONS: usize = 1024;

/// Who the human played against.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct OpponentLabel {
    /// Human-readable name, e.g. `"CFR policy"` or `"baseline"`. Sanitised
    /// on construction: control characters become spaces, the result is
    /// trimmed, truncated to [`MAX_LABEL_LEN`] bytes, and `"unknown"` if empty.
    pub name: String,
    /// Policy artifact checksum, if the opponent was a trained artifact.
    pub checksum: Option<u64>,
}

impl OpponentLabel {
    /// Builds a sanitised label.
    pub fn new(name: &str, checksum: Option<u64>) -> OpponentLabel {
        OpponentLabel {
            name: sanitize_label(name),
            checksum,
        }
    }
}

fn sanitize_label(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let mut s = cleaned.trim().to_string();
    if s.len() > MAX_LABEL_LEN {
        let mut cut = MAX_LABEL_LEN;
        while !s.is_char_boundary(cut) {
            cut -= 1;
        }
        s.truncate(cut);
        s = s.trim_end().to_string();
    }
    if s.is_empty() {
        "unknown".to_string()
    } else {
        s
    }
}

/// One finished hand, as the human could legitimately know it.
/// Arrays are indexed by [`Seat::index`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HandRecord {
    /// 1-based hand number within the saved session.
    pub hand_number: u64,
    /// Blinds and configured depth (`starting_stack`).
    pub rules: RulesConfig,
    pub button: Seat,
    pub human_seat: Seat,
    pub starting_stacks: [Chips; 2],
    pub opponent: OpponentLabel,
    /// The human's own hole cards.
    pub human_cards: [Card; 2],
    /// The opponent's hole cards, `Some` only if shown at showdown.
    pub opponent_cards: Option<[Card; 2]>,
    /// Board cards in deal order, only those revealed.
    pub board: Vec<Card>,
    /// Public voluntary actions in order (blinds are implied by `rules`).
    pub actions: Vec<ActionRecord>,
    pub reason: TerminalReason,
    pub payoffs: [i64; 2],
    pub final_stacks: [Chips; 2],
}

/// Errors building, parsing or validating history records.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HistoryError {
    /// Malformed text (1-based line number; 0 = end of input).
    Parse { line: usize, reason: String },
    /// A `mako-history` header with a version other than `v1`.
    UnsupportedVersion(String),
    /// The hand is not finished yet.
    NotTerminal,
    /// The event stream contains the opponent's private hole cards
    /// (omniscient or wrong-seat stream). Refused to protect hidden info.
    HiddenInformation,
    /// The record (or stream) is inconsistent with the rules.
    Invalid { hand_number: u64, reason: String },
}

impl fmt::Display for HistoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HistoryError::Parse { line, reason } => write!(f, "history line {line}: {reason}"),
            HistoryError::UnsupportedVersion(v) => {
                write!(f, "unsupported history version {v:?}")
            }
            HistoryError::NotTerminal => write!(f, "the hand is not finished"),
            HistoryError::HiddenInformation => write!(
                f,
                "event stream contains the opponent's unrevealed hole cards"
            ),
            HistoryError::Invalid {
                hand_number,
                reason,
            } => write!(f, "hand {hand_number} is invalid: {reason}"),
        }
    }
}

impl std::error::Error for HistoryError {}

impl HandRecord {
    /// Builds a record of a finished hand from the human's filtered events.
    /// The full state is only used to obtain
    /// `state.events_for(Viewer::Seat(human))`.
    pub fn from_state(
        hand_number: u64,
        state: &HandState,
        human: Seat,
        opponent: OpponentLabel,
    ) -> Result<HandRecord, HistoryError> {
        if !state.is_terminal() {
            return Err(HistoryError::NotTerminal);
        }
        HandRecord::from_events(
            hand_number,
            human,
            &state.events_for(Viewer::Seat(human)),
            opponent,
        )
    }

    /// Builds a record from a seat-filtered event stream of a finished hand.
    /// Rejects streams containing the opponent's private `HoleCards`.
    pub fn from_events(
        hand_number: u64,
        human: Seat,
        events: &[HandEvent],
        opponent: OpponentLabel,
    ) -> Result<HandRecord, HistoryError> {
        if events
            .iter()
            .any(|e| matches!(e, HandEvent::HoleCards { seat, .. } if *seat != human))
        {
            return Err(HistoryError::HiddenInformation);
        }
        let invalid = |reason: String| HistoryError::Invalid {
            hand_number,
            reason,
        };
        if hand_number == 0 {
            return Err(invalid("hand numbers start at 1".into()));
        }
        let obs = PlayerObservation::from_events(human, events)
            .map_err(|e| invalid(format!("event stream rejected: {e}")))?;
        let (Some(reason), Some(payoffs)) = (obs.terminal, obs.payoffs) else {
            return Err(HistoryError::NotTerminal);
        };
        let Some(HandEvent::HandStarted {
            rules,
            starting_stacks,
            ..
        }) = events.first()
        else {
            return Err(invalid("missing HandStarted".into()));
        };
        let rec = HandRecord {
            hand_number,
            rules: *rules,
            button: obs.button,
            human_seat: human,
            starting_stacks: *starting_stacks,
            opponent,
            human_cards: obs.hole_cards,
            opponent_cards: obs.opponent_hole_cards,
            board: obs.board,
            actions: obs.history,
            reason,
            payoffs,
            final_stacks: obs.stacks,
        };
        rec.validate()?;
        Ok(rec)
    }

    /// The opponent's seat.
    pub fn opponent_seat(&self) -> Seat {
        self.human_seat.other()
    }

    /// The big blind size of this hand.
    pub fn big_blind(&self) -> Chips {
        self.rules.big_blind
    }

    /// Rebuilds the human's filtered event stream from the record.
    pub fn to_events(&self) -> Vec<HandEvent> {
        let btn = self.button;
        let bb = btn.other();
        let posted = rules::post_blinds(
            &self.rules,
            self.starting_stacks[btn.index()],
            self.starting_stacks[bb.index()],
        );
        let mut blinds = [0; 2];
        blinds[btn.index()] = posted.button_posted;
        blinds[bb.index()] = posted.big_blind_posted;
        let mut ev = vec![
            HandEvent::HandStarted {
                rules: self.rules,
                button: btn,
                starting_stacks: self.starting_stacks,
                blinds_posted: blinds,
            },
            HandEvent::HoleCards {
                seat: self.human_seat,
                cards: self.human_cards,
            },
        ];
        let mut shown = 0usize;
        let mut emit = |target: usize, ev: &mut Vec<HandEvent>| {
            for (street, lo, hi) in [
                (Street::Flop, 0, 3),
                (Street::Turn, 3, 4),
                (Street::River, 4, 5),
            ] {
                if hi > shown && hi <= target && hi <= self.board.len() {
                    ev.push(HandEvent::BoardDealt {
                        street,
                        cards: self.board[lo..hi].to_vec(),
                    });
                    shown = hi;
                }
            }
        };
        for a in &self.actions {
            let need = match a.street {
                Street::Preflop => 0,
                Street::Flop => 3,
                Street::Turn => 4,
                Street::River | Street::Showdown => 5,
            };
            emit(need, &mut ev);
            ev.push(HandEvent::Action(*a));
        }
        emit(5, &mut ev);
        if let (TerminalReason::Showdown, Some(opp), 5) =
            (self.reason, self.opponent_cards, self.board.len())
        {
            let mut hands = [None; 2];
            for (seat, hole) in [
                (self.human_seat, self.human_cards),
                (self.opponent_seat(), opp),
            ] {
                let b = &self.board;
                let seven = [hole[0], hole[1], b[0], b[1], b[2], b[3], b[4]];
                hands[seat.index()] = Some(ShowdownHand {
                    seat,
                    hole_cards: hole,
                    best_five: best_five(&seven),
                    rank: evaluate7(&seven),
                });
            }
            if let [Some(a), Some(b)] = hands {
                ev.push(HandEvent::Showdown { hands: [a, b] });
            }
        }
        ev.push(HandEvent::HandEnded {
            reason: self.reason,
            payoffs: self.payoffs,
            final_stacks: self.final_stacks,
        });
        ev
    }

    /// Semantic validation: replays the record through the engine's public
    /// replayer and checks it reproduces exactly.
    pub fn validate(&self) -> Result<(), HistoryError> {
        let invalid = |reason: &str| HistoryError::Invalid {
            hand_number: self.hand_number,
            reason: reason.to_string(),
        };
        if self.hand_number == 0 {
            return Err(invalid("hand numbers start at 1"));
        }
        if self.opponent != OpponentLabel::new(&self.opponent.name, self.opponent.checksum) {
            return Err(invalid("opponent label is not sanitised"));
        }
        match (self.reason, self.opponent_cards) {
            (TerminalReason::Showdown, None) => {
                return Err(invalid("showdown without the opponent's shown cards"));
            }
            (TerminalReason::Fold { .. }, Some(_)) => {
                return Err(invalid(
                    "opponent cards stored for a hand that ended by fold",
                ));
            }
            _ => {}
        }
        if self.actions.len() > MAX_ACTIONS {
            return Err(invalid("too many actions"));
        }
        let events = self.to_events();
        let obs = PlayerObservation::from_events(self.human_seat, &events)
            .map_err(|e| invalid(&format!("does not replay: {e}")))?;
        let ok = obs.terminal == Some(self.reason)
            && obs.payoffs == Some(self.payoffs)
            && obs.stacks == self.final_stacks
            && obs.board == self.board
            && obs.history == self.actions
            && obs.hole_cards == self.human_cards
            && obs.opponent_hole_cards == self.opponent_cards;
        if !ok {
            return Err(invalid("record does not reproduce under replay"));
        }
        Ok(())
    }

    /// Appends this record's text block (without the file header).
    pub fn write_block(&self, out: &mut String) {
        use std::fmt::Write as _;
        let r = self.rules;
        let cards = |cs: &[Card]| {
            if cs.is_empty() {
                "-".to_string()
            } else {
                cs.iter()
                    .map(|c| c.to_string())
                    .collect::<Vec<_>>()
                    .join(" ")
            }
        };
        let _ = writeln!(out, "hand {}", self.hand_number);
        let _ = writeln!(
            out,
            "rules {} {} {}",
            r.small_blind, r.big_blind, r.starting_stack
        );
        let _ = writeln!(out, "button {}", self.button.index());
        let _ = writeln!(out, "human {}", self.human_seat.index());
        let _ = writeln!(
            out,
            "stacks {} {}",
            self.starting_stacks[0], self.starting_stacks[1]
        );
        let ck = match self.opponent.checksum {
            Some(c) => format!("{c:016x}"),
            None => "-".to_string(),
        };
        let _ = writeln!(out, "opponent {ck} {}", self.opponent.name);
        let _ = writeln!(out, "hole {}", cards(&self.human_cards));
        let _ = writeln!(
            out,
            "shown {}",
            cards(self.opponent_cards.as_ref().map_or(&[][..], |c| &c[..]))
        );
        let _ = writeln!(out, "board {}", cards(&self.board));
        for a in &self.actions {
            let _ = writeln!(
                out,
                "action {} {} {} {} {}",
                a.seat.index(),
                street_name(a.street),
                kind_name(a.kind),
                a.raise_to,
                a.chips_committed
            );
        }
        match self.reason {
            TerminalReason::Fold { winner } => {
                let _ = writeln!(out, "result fold {}", winner.index());
            }
            TerminalReason::Showdown => out.push_str("result showdown\n"),
        }
        let _ = writeln!(out, "payoffs {} {}", self.payoffs[0], self.payoffs[1]);
        let _ = writeln!(
            out,
            "final {} {}",
            self.final_stacks[0], self.final_stacks[1]
        );
        out.push_str("end\n");
    }
}

/// Serialises a whole history file (header plus one block per record).
pub fn to_text(records: &[HandRecord]) -> String {
    let mut s = String::with_capacity(64 + records.len() * 320);
    s.push_str(HISTORY_HEADER_V1);
    s.push('\n');
    for r in records {
        r.write_block(&mut s);
    }
    s
}

/// Parses a whole history file strictly (see the module docs).
pub fn parse(text: &str) -> Result<Vec<HandRecord>, HistoryError> {
    let mut p = Parser {
        lines: text.split('\n').collect(),
        pos: 0,
    };
    // A well-formed file ends with '\n', so the final split piece is empty.
    match p.lines.pop() {
        Some("") => {}
        _ => {
            return Err(HistoryError::Parse {
                line: p.lines.len() + 1,
                reason: "missing final newline (truncated file)".into(),
            });
        }
    }
    let Some(&header) = p.lines.first() else {
        return Err(perr(1, "empty file"));
    };
    if header != HISTORY_HEADER_V1 {
        return match header.strip_prefix(HISTORY_HEADER_PREFIX) {
            Some(v) => Err(HistoryError::UnsupportedVersion(v.to_string())),
            None => Err(perr(1, "missing 'mako-history v1' header")),
        };
    }
    p.pos = 1;
    let mut out: Vec<HandRecord> = Vec::new();
    while p.pos < p.lines.len() {
        let rec = p.block()?;
        if let Some(prev) = out.last() {
            if rec.hand_number <= prev.hand_number {
                return Err(HistoryError::Invalid {
                    hand_number: rec.hand_number,
                    reason: "hand numbers must strictly increase".into(),
                });
            }
        }
        out.push(rec);
    }
    Ok(out)
}

fn perr(line: usize, reason: &str) -> HistoryError {
    HistoryError::Parse {
        line,
        reason: reason.to_string(),
    }
}

struct Parser<'a> {
    lines: Vec<&'a str>,
    pos: usize,
}

impl<'a> Parser<'a> {
    /// Next line split on single spaces, requiring `key` first; returns the
    /// 1-based line number and the remaining tokens.
    fn field(&mut self, key: &str) -> Result<(usize, Vec<&'a str>), HistoryError> {
        let Some(&l) = self.lines.get(self.pos) else {
            return Err(perr(0, &format!("missing '{key}' line (truncated file)")));
        };
        self.pos += 1;
        let ln = self.pos;
        let mut toks = l.split(' ');
        if toks.next() != Some(key) {
            return Err(perr(ln, &format!("expected '{key}'")));
        }
        Ok((ln, toks.collect()))
    }

    fn fixed(&mut self, key: &str, n: usize) -> Result<(usize, Vec<&'a str>), HistoryError> {
        let (ln, t) = self.field(key)?;
        if t.len() != n {
            return Err(perr(ln, &format!("'{key}' takes {n} values")));
        }
        Ok((ln, t))
    }

    fn block(&mut self) -> Result<HandRecord, HistoryError> {
        let (ln, t) = self.fixed("hand", 1)?;
        let hand_number: u64 = num(ln, t[0])?;
        if hand_number == 0 {
            return Err(perr(ln, "hand numbers start at 1"));
        }
        let (ln, t) = self.fixed("rules", 3)?;
        let rules = RulesConfig {
            small_blind: num(ln, t[0])?,
            big_blind: num(ln, t[1])?,
            starting_stack: num(ln, t[2])?,
        };
        let (ln, t) = self.fixed("button", 1)?;
        let button = seat(ln, t[0])?;
        let (ln, t) = self.fixed("human", 1)?;
        let human_seat = seat(ln, t[0])?;
        let (ln, t) = self.fixed("stacks", 2)?;
        let starting_stacks = [num(ln, t[0])?, num(ln, t[1])?];

        let (ln, t) = self.field("opponent")?;
        if t.len() < 2 {
            return Err(perr(ln, "'opponent' takes a checksum and a label"));
        }
        let checksum = match t[0] {
            "-" => None,
            h if h.len() == 16
                && h.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) =>
            {
                Some(u64::from_str_radix(h, 16).map_err(|_| perr(ln, "bad checksum"))?)
            }
            _ => return Err(perr(ln, "checksum must be '-' or 16 lowercase hex digits")),
        };
        let name = t[1..].join(" ");
        let opponent = OpponentLabel::new(&name, checksum);
        if opponent.name != name {
            return Err(perr(ln, "opponent label is not in canonical form"));
        }

        let (ln, t) = self.fixed("hole", 2)?;
        let human_cards = [card(ln, t[0])?, card(ln, t[1])?];
        let (ln, t) = self.field("shown")?;
        let opponent_cards = match t.as_slice() {
            ["-"] => None,
            [a, b] => Some([card(ln, a)?, card(ln, b)?]),
            _ => return Err(perr(ln, "'shown' takes '-' or two cards")),
        };
        let (ln, t) = self.field("board")?;
        let board = match t.as_slice() {
            ["-"] => Vec::new(),
            cs if matches!(cs.len(), 3..=5) => {
                cs.iter().map(|c| card(ln, c)).collect::<Result<_, _>>()?
            }
            _ => return Err(perr(ln, "'board' takes '-' or 3 to 5 cards")),
        };

        let mut actions = Vec::new();
        loop {
            let Some(&l) = self.lines.get(self.pos) else {
                return Err(perr(0, "missing 'result' line (truncated file)"));
            };
            if !l.starts_with("action ") {
                break;
            }
            let (ln, t) = self.fixed("action", 5)?;
            if actions.len() >= MAX_ACTIONS {
                return Err(perr(ln, "too many actions"));
            }
            actions.push(ActionRecord {
                seat: seat(ln, t[0])?,
                street: parse_street(t[1]).ok_or_else(|| perr(ln, "unknown street"))?,
                kind: parse_kind(t[2]).ok_or_else(|| perr(ln, "unknown action kind"))?,
                raise_to: num(ln, t[3])?,
                chips_committed: num(ln, t[4])?,
            });
        }

        let (ln, t) = self.field("result")?;
        let reason = match t.as_slice() {
            ["showdown"] => TerminalReason::Showdown,
            ["fold", w] => TerminalReason::Fold {
                winner: seat(ln, w)?,
            },
            _ => return Err(perr(ln, "'result' takes 'showdown' or 'fold <seat>'")),
        };
        let (ln, t) = self.fixed("payoffs", 2)?;
        let payoffs = [num(ln, t[0])?, num(ln, t[1])?];
        let (ln, t) = self.fixed("final", 2)?;
        let final_stacks = [num(ln, t[0])?, num(ln, t[1])?];
        self.fixed("end", 0)?;

        let rec = HandRecord {
            hand_number,
            rules,
            button,
            human_seat,
            starting_stacks,
            opponent,
            human_cards,
            opponent_cards,
            board,
            actions,
            reason,
            payoffs,
            final_stacks,
        };
        rec.validate()?;
        Ok(rec)
    }
}

/// Strict decimal integer: optional leading `-` only for signed targets, no
/// `+`, no leading zeros (except `0` itself).
fn num<T: std::str::FromStr>(line: usize, s: &str) -> Result<T, HistoryError> {
    let digits = s.strip_prefix('-').unwrap_or(s);
    let canonical = !digits.is_empty()
        && digits.bytes().all(|b| b.is_ascii_digit())
        && (digits == "0" || !digits.starts_with('0'))
        && s != "-0";
    if !canonical {
        return Err(perr(line, &format!("invalid number {s:?}")));
    }
    s.parse()
        .map_err(|_| perr(line, &format!("number out of range {s:?}")))
}

fn seat(line: usize, s: &str) -> Result<Seat, HistoryError> {
    match s {
        "0" => Ok(Seat::Zero),
        "1" => Ok(Seat::One),
        _ => Err(perr(line, "seat must be 0 or 1")),
    }
}

/// Cards must be in canonical display form (e.g. `As`, `Td`).
fn card(line: usize, s: &str) -> Result<Card, HistoryError> {
    match s.parse::<Card>() {
        Ok(c) if c.to_string() == s => Ok(c),
        _ => Err(perr(line, &format!("invalid card {s:?}"))),
    }
}

fn street_name(s: Street) -> &'static str {
    match s {
        Street::Preflop => "preflop",
        Street::Flop => "flop",
        Street::Turn => "turn",
        Street::River => "river",
        Street::Showdown => "showdown",
    }
}

fn parse_street(s: &str) -> Option<Street> {
    [Street::Preflop, Street::Flop, Street::Turn, Street::River]
        .into_iter()
        .find(|&k| street_name(k) == s)
}

/// Stable lowercase name of an action class.
pub fn kind_name(k: ActionKind) -> &'static str {
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
