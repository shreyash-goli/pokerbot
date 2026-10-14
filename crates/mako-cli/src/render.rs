//! Minimal text rendering. Everything here is built from a
//! [`PlayerObservation`] for the human seat (plus public session data), so
//! the screen can only show what that player may legally know. The polished
//! retro UI is M-15.

use mako_core::action::{ActionKind, ActionRecord, LegalAction, Seat, Street, TerminalReason};
use mako_core::card::{Card, Suit};
use mako_core::eval;
use mako_core::observation::PlayerObservation;
use mako_core::rules::Chips;

use crate::input::key;

/// Marker line printed when a hand legitimately reaches showdown. Nothing
/// before it in a hand's output may contain the opponent's hole cards.
pub const SHOWDOWN_MARKER: &str = "*** SHOWDOWN ***";

/// ANSI styling, or plain text when disabled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Style {
    pub color: bool,
}

impl Style {
    fn paint(&self, code: &str, text: &str) -> String {
        if self.color {
            format!("\x1b[{code}m{text}\x1b[0m")
        } else {
            text.to_string()
        }
    }

    pub fn bold(&self, text: &str) -> String {
        self.paint("1", text)
    }

    pub fn dim(&self, text: &str) -> String {
        self.paint("2", text)
    }

    pub fn accent(&self, text: &str) -> String {
        self.paint("33", text)
    }

    /// A face-up card, e.g. `[Ah]`.
    pub fn card(&self, c: Card) -> String {
        let text = format!("[{c}]");
        match c.suit() {
            Suit::Hearts | Suit::Diamonds => self.paint("1;31", &text),
            Suit::Clubs | Suit::Spades => self.paint("1", &text),
        }
    }

    pub fn cards(&self, cs: &[Card]) -> String {
        cs.iter().map(|&c| self.card(c)).collect()
    }

    /// Two face-down cards.
    pub fn hidden_pair(&self) -> String {
        self.dim("[??][??]")
    }

    /// The board: revealed cards, then placeholders for the rest.
    pub fn board(&self, revealed: &[Card]) -> String {
        let mut s = self.cards(revealed);
        for _ in revealed.len()..5 {
            s.push_str(&self.dim("[--]"));
        }
        s
    }
}

/// Seat display names from the human's point of view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Names {
    pub human: Seat,
}

impl Names {
    pub fn of(&self, seat: Seat) -> &'static str {
        if seat == self.human { "You" } else { "Bot" }
    }
}

pub fn street_name(s: Street) -> &'static str {
    match s {
        Street::Preflop => "Preflop",
        Street::Flop => "Flop",
        Street::Turn => "Turn",
        Street::River => "River",
        Street::Showdown => "Hand over",
    }
}

fn signed(v: i64) -> String {
    if v > 0 {
        format!("+{v}")
    } else {
        v.to_string()
    }
}

/// Signed chip amount, e.g. `+3`, `-12`, `0`.
pub fn chips_delta(v: i64) -> String {
    signed(v)
}

/// Describes a recorded action. `street_max` is the largest street
/// contribution before the action (decides "bet" vs "raise to"). `you`
/// selects second-person wording ("call 1") over third ("calls 1").
pub fn describe_record(rec: &ActionRecord, street_max: Chips, you: bool) -> String {
    let v = |base: &str, third: &str| {
        if you {
            base.to_string()
        } else {
            third.to_string()
        }
    };
    let size = |tag: &str| {
        if street_max == 0 {
            format!("{} {}{tag}", v("bet", "bets"), rec.raise_to)
        } else {
            format!("{} to {}{tag}", v("raise", "raises"), rec.raise_to)
        }
    };
    match rec.kind {
        ActionKind::Fold => v("fold", "folds"),
        ActionKind::Check => v("check", "checks"),
        ActionKind::Call => format!("{} {}", v("call", "calls"), rec.chips_committed),
        ActionKind::HalfPot => size(" (half pot)"),
        ActionKind::Pot => size(" (pot)"),
        ActionKind::AllIn => size(" (all-in)"),
    }
}

/// The public action log of the hand, one line per street.
pub fn history_lines(obs: &PlayerObservation, names: Names) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut current: Option<Street> = None;
    let mut street_max: Chips = 0;
    let mut line = String::new();
    for rec in &obs.history {
        if current != Some(rec.street) {
            if !line.is_empty() {
                lines.push(std::mem::take(&mut line));
            }
            current = Some(rec.street);
            street_max = if rec.street == Street::Preflop {
                obs.big_blind
            } else {
                0
            };
            line = format!("{:>8}:", street_name(rec.street));
        }
        line.push_str(&format!(
            " {} {};",
            names.of(rec.seat),
            describe_record(rec, street_max, rec.seat == names.human)
        ));
        street_max = street_max.max(rec.raise_to);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// Menu label for a legal action, with its exact chip amount.
pub fn action_label(a: &LegalAction, obs: &PlayerObservation) -> String {
    let me = obs.seat.index();
    let facing = obs.street_contributions.iter().copied().max().unwrap_or(0) > 0;
    let verb = if facing { "raise to" } else { "bet" };
    match a.kind {
        ActionKind::Fold => "fold".to_string(),
        ActionKind::Check => "check".to_string(),
        ActionKind::Call if a.chips_to_commit == obs.stacks[me] => {
            format!("call {} (all-in)", a.chips_to_commit)
        }
        ActionKind::Call => format!("call {}", a.chips_to_commit),
        ActionKind::HalfPot => format!("{verb} {} (half pot)", a.raise_to),
        ActionKind::Pot => format!("{verb} {} (pot)", a.raise_to),
        ActionKind::AllIn => format!("all-in to {}", a.raise_to),
    }
}

/// The action menu, e.g. `c) call 1   h) raise to 4 (half pot)   ...`.
pub fn action_menu(obs: &PlayerObservation, style: Style) -> String {
    obs.legal_actions
        .iter()
        .map(|a| {
            format!(
                "{}) {}",
                style.accent(&key(a.kind).to_string()),
                action_label(a, obs)
            )
        })
        .collect::<Vec<_>>()
        .join("   ")
}

fn position_tag(obs: &PlayerObservation, seat: Seat) -> &'static str {
    if seat == obs.button { "BTN/SB" } else { "BB" }
}

/// The table as seen by `obs.seat`. The opponent's cards are face down
/// unless the observation itself reveals them (showdown only).
pub fn table(obs: &PlayerObservation, names: Names, style: Style) -> String {
    let me = obs.seat;
    let opp = me.other();
    let opp_cards = match obs.opponent_hole_cards {
        Some(c) => style.cards(&c),
        None => style.hidden_pair(),
    };
    let row = |seat: Seat, cards: String| {
        format!(
            "  {:<3} {}  stack {:>4}  in pot this street {:>4}  {}",
            names.of(seat),
            cards,
            obs.stacks[seat.index()],
            obs.street_contributions[seat.index()],
            style.dim(position_tag(obs, seat)),
        )
    };
    let mut out = String::new();
    out.push_str(&row(opp, opp_cards));
    out.push('\n');
    out.push_str(&format!(
        "      Board {}   Pot {}   ({})\n",
        style.board(&obs.board),
        style.bold(&obs.pot.to_string()),
        street_name(obs.street)
    ));
    out.push_str(&row(me, style.cards(&obs.hole_cards)));
    out.push('\n');
    let log = history_lines(obs, names);
    if !log.is_empty() {
        out.push_str(&style.dim("  Action log:"));
        out.push('\n');
        for l in log {
            out.push_str(&format!("  {l}\n"));
        }
    }
    out
}

/// End-of-hand summary from the human's observation of a terminal state.
pub fn result(obs: &PlayerObservation, names: Names, style: Style) -> String {
    let me = obs.seat;
    let opp = me.other();
    let mut out = String::new();
    match obs.terminal {
        Some(TerminalReason::Fold { winner }) => {
            out.push_str(&format!(
                "  {} folded. {} wins the pot. Board: {}\n",
                names.of(winner.other()),
                names.of(winner),
                style.board(&obs.board)
            ));
            out.push_str(&format!(
                "  Bot's cards stay hidden: {}\n",
                style.hidden_pair()
            ));
        }
        Some(TerminalReason::Showdown) => {
            out.push_str(&style.bold(SHOWDOWN_MARKER));
            out.push('\n');
            out.push_str(&format!("  Board {}\n", style.cards(&obs.board)));
            let describe = |hole: [Card; 2]| {
                let name = if obs.board.len() == 5 {
                    let seven: Vec<Card> = hole.iter().chain(&obs.board).copied().collect();
                    eval::evaluate(&seven).category().name()
                } else {
                    "?"
                };
                format!("{} {}", style.cards(&hole), name)
            };
            out.push_str(&format!(
                "  {:<3} {}\n",
                names.of(me),
                describe(obs.hole_cards)
            ));
            if let Some(oc) = obs.opponent_hole_cards {
                out.push_str(&format!("  {:<3} {}\n", names.of(opp), describe(oc)));
            }
        }
        None => out.push_str("  (hand not finished)\n"),
    }
    if let Some(p) = obs.payoffs {
        let mine = p[me.index()];
        let verdict = match mine.cmp(&0) {
            std::cmp::Ordering::Greater => "You win",
            std::cmp::Ordering::Less => "Bot wins",
            std::cmp::Ordering::Equal => "Split pot",
        };
        out.push_str(&format!(
            "  {verdict}. You {} / Bot {}\n",
            style.bold(&signed(mine)),
            signed(p[opp.index()])
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use mako_core::rules::{RulesConfig, StackDepth};
    use mako_core::state::{HandConfig, HandState};

    fn hand(seed: u64) -> HandState {
        HandState::from_seed(
            HandConfig {
                rules: RulesConfig::standard(StackDepth::Bb20),
                button: Seat::Zero,
            },
            seed,
        )
        .unwrap()
    }

    const PLAIN: Style = Style { color: false };
    const NAMES: Names = Names { human: Seat::Zero };

    #[test]
    fn table_hides_opponent_cards_preflop() {
        let h = hand(7);
        let obs = h.observation(Seat::Zero);
        let t = table(&obs, NAMES, PLAIN);
        assert!(t.contains("[??][??]"));
        for c in h.hole_cards(Seat::One) {
            assert!(!t.contains(&format!("[{c}]")), "bot card {c} leaked: {t}");
        }
        for c in h.hole_cards(Seat::Zero) {
            assert!(t.contains(&format!("[{c}]")));
        }
    }

    #[test]
    fn preflop_menu_has_exact_amounts() {
        let h = hand(7);
        let obs = h.observation(Seat::Zero); // button acts first preflop
        let menu = action_menu(&obs, PLAIN);
        assert!(menu.contains("f) fold"), "{menu}");
        assert!(menu.contains("c) call 1"), "{menu}");
        assert!(menu.contains("a) all-in to 40"), "{menu}");
        assert!(menu.contains("raise to"), "{menu}");
    }

    #[test]
    fn fold_result_keeps_cards_hidden() {
        let mut h = hand(9);
        h.apply(ActionKind::Fold).unwrap();
        let obs = h.observation(Seat::Zero);
        let r = result(&obs, NAMES, PLAIN);
        assert!(!r.contains(SHOWDOWN_MARKER));
        assert!(r.contains("You folded"), "{r}");
        for c in h.hole_cards(Seat::One) {
            assert!(!r.contains(&format!("[{c}]")));
        }
    }

    #[test]
    fn showdown_result_reveals() {
        let mut h = hand(3);
        while h.to_act().is_some() {
            let k = if h
                .legal_actions()
                .iter()
                .any(|a| a.kind == ActionKind::Check)
            {
                ActionKind::Check
            } else {
                ActionKind::Call
            };
            h.apply(k).unwrap();
        }
        let obs = h.observation(Seat::Zero);
        let r = result(&obs, NAMES, PLAIN);
        assert!(r.contains(SHOWDOWN_MARKER));
        for c in h.hole_cards(Seat::One) {
            assert!(r.contains(&format!("[{c}]")));
        }
    }

    #[test]
    fn color_toggle() {
        let c = Card::new(mako_core::card::Rank::Ace, Suit::Hearts);
        assert_eq!(PLAIN.card(c), "[Ah]");
        assert!(Style { color: true }.card(c).contains("\x1b["));
    }
}
