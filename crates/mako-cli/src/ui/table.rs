//! The table screen: seats, board, pot, action log and legal-action menu.
//!
//! Wording of aggressive actions: an action is a **bet** when the actor
//! faces no unmatched wager (nothing to call), and a **raise** when it
//! faces one. Preflop the small blind faces the big blind, so an open is
//! "raise to 4"; after a limp the big blind faces nothing, so its
//! half-pot action is "bet 2 (to 4)": 2 more chips, 4 in total.

use mako_core::action::{ActionKind, ActionRecord, LegalAction, Seat, Street};
use mako_core::observation::PlayerObservation;
use mako_core::rules::Chips;

use super::term::{fit, flow, pad, visible_width, wrap};
use super::{Names, Role, Ui};
use crate::input::key;

/// Public per-hand context shown around the table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TableInfo<'a> {
    /// 1-based hand number.
    pub hand: u64,
    pub hands: u64,
    /// The opponent agent's honest label.
    pub opponent: &'a str,
    pub names: Names,
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

fn verb(you: bool, base: &'static str, third: &'static str) -> &'static str {
    if you { base } else { third }
}

/// Phrase for an action. `before`: the actor's street contribution before
/// it; `to_call`: the unmatched amount it faced. `you` picks "call" over
/// "calls".
pub fn phrase(
    kind: ActionKind,
    raise_to: Chips,
    committed: Chips,
    before: Chips,
    to_call: Chips,
    you: bool,
) -> String {
    let tag = match kind {
        ActionKind::HalfPot => Some("half pot"),
        ActionKind::Pot => Some("pot"),
        ActionKind::AllIn => Some("all-in"),
        _ => None,
    };
    match kind {
        ActionKind::Fold => verb(you, "fold", "folds").to_string(),
        ActionKind::Check => verb(you, "check", "checks").to_string(),
        ActionKind::Call => format!("{} {committed}", verb(you, "call", "calls")),
        _ if to_call == 0 => {
            let mut notes = Vec::new();
            if before > 0 {
                notes.push(format!("to {raise_to}"));
            }
            notes.extend(tag.map(str::to_string));
            let amount = if before > 0 { committed } else { raise_to };
            let notes = if notes.is_empty() {
                String::new()
            } else {
                format!(" ({})", notes.join(", "))
            };
            format!("{} {amount}{notes}", verb(you, "bet", "bets"))
        }
        _ => {
            let notes = tag.map_or(String::new(), |t| format!(" ({t})"));
            format!("{} to {raise_to}{notes}", verb(you, "raise", "raises"))
        }
    }
}

/// Phrase for an action just taken, from the actor's observation *before*
/// it (public contributions only).
pub fn describe_live(rec: &ActionRecord, before: &PlayerObservation, you: bool) -> String {
    let me = before.street_contributions[rec.seat.index()];
    let opp = before.street_contributions[rec.seat.other().index()];
    phrase(
        rec.kind,
        rec.raise_to,
        rec.chips_committed,
        me,
        opp.saturating_sub(me),
        you,
    )
}

/// The public action log: one entry per street that has actions, as
/// `(street, "You call 1, Bot raises to 6 (pot)")`.
pub fn history(obs: &PlayerObservation, names: Names) -> Vec<(Street, String)> {
    let mut out: Vec<(Street, Vec<String>)> = Vec::new();
    let mut contrib = [0 as Chips; 2];
    for rec in &obs.history {
        if out.last().map(|(s, _)| *s) != Some(rec.street) {
            contrib = if rec.street == Street::Preflop {
                let mut c = [0; 2];
                c[obs.button.index()] = obs.small_blind;
                c[obs.button.other().index()] = obs.big_blind;
                c
            } else {
                [0; 2]
            };
            out.push((rec.street, Vec::new()));
        }
        let me = contrib[rec.seat.index()];
        let to_call = contrib[rec.seat.other().index()].saturating_sub(me);
        let you = rec.seat == names.human;
        let text = phrase(
            rec.kind,
            rec.raise_to,
            rec.chips_committed,
            me,
            to_call,
            you,
        );
        contrib[rec.seat.index()] = rec.raise_to;
        out.last_mut()
            .expect("pushed")
            .1
            .push(format!("{} {text}", names.of(rec.seat)));
    }
    out.into_iter().map(|(s, v)| (s, v.join(", "))).collect()
}

/// Menu label for a legal action, with its exact chip amounts.
pub fn action_label(a: &LegalAction, obs: &PlayerObservation) -> String {
    let me = obs.street_contributions[obs.seat.index()];
    let opp = obs.street_contributions[obs.seat.other().index()];
    let to_call = opp.saturating_sub(me);
    match a.kind {
        ActionKind::Call if a.chips_to_commit == obs.stacks[obs.seat.index()] => {
            format!("call {} (all-in)", a.chips_to_commit)
        }
        ActionKind::AllIn => format!("all-in to {}", a.raise_to),
        k => phrase(k, a.raise_to, a.chips_to_commit, me, to_call, true),
    }
}

/// Menu items, e.g. `[c] call 1`.
pub fn menu_items(obs: &PlayerObservation, ui: &Ui) -> Vec<String> {
    obs.legal_actions
        .iter()
        .map(|a| {
            let k = ui.paint(Role::Accent, &format!("[{}]", key(a.kind)));
            format!("{k} {}", action_label(a, obs))
        })
        .collect()
}

/// The legal-action menu, wrapped to the width.
pub fn menu(obs: &PlayerObservation, ui: &Ui) -> Vec<String> {
    let head = format!("  {} ", ui.paint(Role::Strong, "Options:"));
    let cont = " ".repeat(visible_width(&head));
    flow(&menu_items(obs, ui), ui.width, &head, &cont, "   ")
}

fn position(obs: &PlayerObservation, seat: Seat) -> &'static str {
    if seat == obs.button { "BTN/SB" } else { "BB" }
}

fn seat_row(obs: &PlayerObservation, seat: Seat, names: Names, ui: &Ui) -> String {
    let f = ui.frame();
    let marker = if obs.to_act == Some(seat) {
        ui.paint(Role::Accent, &f.arrow.to_string())
    } else {
        " ".to_string()
    };
    let cards = if seat == obs.seat {
        ui.cards(&obs.hole_cards)
    } else {
        match obs.opponent_hole_cards {
            Some(c) => ui.cards(&c),
            None => ui.backs(),
        }
    };
    let stack = obs.stacks[seat.index()];
    let stack_text = if stack == 0 && obs.terminal.is_none() {
        ui.paint(Role::Warn, "ALL-IN")
    } else {
        format!("{stack:>4}")
    };
    format!(
        "{marker} {:<3}  {:<6} {cards}  stack {stack_text}  bet {:>4}",
        names.of(seat),
        position(obs, seat),
        obs.street_contributions[seat.index()],
    )
}

fn board_row(obs: &PlayerObservation, ui: &Ui) -> String {
    format!(
        "        Board {}   Pot {}",
        ui.board(&obs.board),
        ui.paint(Role::Strong, &obs.pot.to_string())
    )
}

/// `╔═ Mako ══…═ right ═╗` sized to `width`.
fn top_border(ui: &Ui, right: &str) -> String {
    let f = ui.frame();
    let left = format!("{}{} ", f.tl, f.h);
    let title = ui.paint(Role::Title, "Mako");
    let right = format!(" {right} {}{}", f.h, f.tr);
    let used = visible_width(&left) + 4 + 1 + visible_width(&right);
    let fill: String = std::iter::repeat_n(f.h, ui.width.saturating_sub(used)).collect();
    let line = format!(
        "{}{title}{}{}",
        ui.paint(Role::Frame, &left),
        ui.paint(Role::Frame, &format!(" {fill}")),
        ui.paint(Role::Frame, &right)
    );
    fit(&line, ui.width)
}

/// `╚═ vs label ═══╝` sized to `width`.
fn bottom_border(ui: &Ui, label: &str) -> String {
    let f = ui.frame();
    let max_label = ui.width.saturating_sub(12);
    let label = fit(label, max_label);
    let left = format!("{}{} vs ", f.bl, f.h);
    let used = visible_width(&left) + visible_width(&label) + 2;
    let fill: String = std::iter::repeat_n(f.h, ui.width.saturating_sub(used)).collect();
    fit(
        &format!(
            "{}{}{}",
            ui.paint(Role::Frame, &left),
            ui.paint(Role::Label, &label),
            ui.paint(Role::Frame, &format!(" {fill}{}", f.br))
        ),
        ui.width,
    )
}

/// A framed content line: `║ content… ║`.
fn boxed(ui: &Ui, content: &str) -> String {
    let f = ui.frame();
    let inner = ui.width.saturating_sub(4);
    let v = ui.paint(Role::Frame, &f.v.to_string());
    format!("{v} {} {v}", pad(&fit(content, inner), inner))
}

/// A framed line with a left and a right pane.
fn boxed2(ui: &Ui, left: &str, right: &str, left_w: usize) -> String {
    let f = ui.frame();
    let right_w = ui.width.saturating_sub(left_w + 7);
    let v = ui.paint(Role::Frame, &f.v.to_string());
    let s = ui.paint(Role::Frame, &f.split.to_string());
    format!(
        "{v} {} {s} {} {v}",
        pad(&fit(left, left_w), left_w),
        pad(&fit(right, right_w), right_w)
    )
}

/// Width of the left pane in the wide layout.
pub const WIDE_LEFT: usize = 56;

/// The table as seen by `obs.seat`, followed by the action log (compact
/// layout) and nothing else; the menu is rendered by [`menu`].
pub fn table_screen(obs: &PlayerObservation, info: &TableInfo<'_>, ui: &Ui) -> Vec<String> {
    let f = ui.frame();
    let me = obs.seat;
    let right = format!(
        "Hand {}/{} {} {}",
        info.hand,
        info.hands,
        f.dot,
        street_name(obs.street)
    );
    let rows = [
        seat_row(obs, me.other(), info.names, ui),
        String::new(),
        board_row(obs, ui),
        String::new(),
        seat_row(obs, me, info.names, ui),
    ];
    let log = history(obs, info.names);
    let mut out = vec![top_border(ui, &right)];
    if ui.is_wide() {
        let right_w = ui.width.saturating_sub(WIDE_LEFT + 7);
        let mut log_lines: Vec<String> = Vec::new();
        for (street, text) in &log {
            let head = format!("{:<7} ", street_name(*street));
            log_lines.extend(wrap(text, right_w, &head, "        "));
        }
        let shown = rows.len() - 1;
        let skip = log_lines.len().saturating_sub(shown);
        let mut pane = vec![ui.paint(Role::Dim, "Action log")];
        if log_lines.is_empty() {
            pane.push(ui.paint(Role::Dim, "(no actions yet)"));
        }
        pane.extend(log_lines.into_iter().skip(skip));
        for (i, row) in rows.iter().enumerate() {
            let r = pane.get(i).map_or("", String::as_str);
            out.push(boxed2(ui, row, r, WIDE_LEFT));
        }
    } else {
        for row in &rows {
            out.push(boxed(ui, row));
        }
    }
    out.push(bottom_border(ui, info.opponent));
    if !ui.is_wide() && !log.is_empty() {
        let joined: Vec<String> = log
            .iter()
            .map(|(s, t)| format!("{}: {t}", street_name(*s)))
            .collect();
        let text = joined.join(" | ");
        let mut lines = wrap(&text, ui.width, "  Log: ", "       ");
        // Keep the screen within 24 lines: the newest three log lines.
        let skip = lines.len().saturating_sub(3);
        lines.drain(..skip);
        out.extend(lines.into_iter().map(|l| ui.paint(Role::Dim, &l)));
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

    const NAMES: Names = Names { human: Seat::Zero };

    #[test]
    fn bet_vs_raise_wording() {
        // SB open faces the big blind: a raise.
        assert_eq!(
            phrase(ActionKind::HalfPot, 4, 3, 1, 1, true),
            "raise to 4 (half pot)"
        );
        // BB after a limp faces nothing: a bet of 2 more, 4 in total.
        assert_eq!(
            phrase(ActionKind::HalfPot, 4, 2, 2, 0, false),
            "bets 2 (to 4, half pot)"
        );
        // Postflop opening bet.
        assert_eq!(
            phrase(ActionKind::Pot, 12, 12, 0, 0, false),
            "bets 12 (pot)"
        );
        assert_eq!(
            phrase(ActionKind::AllIn, 30, 30, 0, 0, true),
            "bet 30 (all-in)"
        );
        assert_eq!(
            phrase(ActionKind::AllIn, 40, 34, 6, 6, false),
            "raises to 40 (all-in)"
        );
        assert_eq!(phrase(ActionKind::Call, 6, 4, 2, 4, true), "call 4");
        assert_eq!(phrase(ActionKind::Check, 2, 0, 2, 0, false), "checks");
    }

    #[test]
    fn limp_then_bb_bet_in_history_and_menu() {
        let mut h = hand(7);
        h.apply(ActionKind::Call).unwrap(); // button limps
        let bb = h.observation(Seat::One);
        let labels: Vec<String> = bb
            .legal_actions
            .iter()
            .map(|a| action_label(a, &bb))
            .collect();
        assert!(labels.contains(&"check".to_string()), "{labels:?}");
        assert!(
            labels.iter().any(|l| l.starts_with("bet 2 (to 4")),
            "{labels:?}"
        );
        h.apply(ActionKind::HalfPot).unwrap();
        let obs = h.observation(Seat::Zero);
        let log = history(&obs, NAMES);
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].1, "You call 1, Bot bets 2 (to 4, half pot)");
        let menu: Vec<String> = obs
            .legal_actions
            .iter()
            .map(|a| action_label(a, &obs))
            .collect();
        assert!(menu.contains(&"call 2".to_string()), "{menu:?}");
        assert!(menu.iter().any(|l| l.starts_with("raise to")), "{menu:?}");
    }

    #[test]
    fn table_fits_every_width_and_hides_backs() {
        let h = hand(11);
        let obs = h.observation(Seat::Zero);
        for glyphs in [super::super::Glyphs::Ascii, super::super::Glyphs::Unicode] {
            for width in (super::super::term::MIN_WIDTH..=200).step_by(7) {
                let ui = Ui {
                    color: super::super::ColorMode::Ansi256,
                    glyphs,
                    width,
                };
                let info = TableInfo {
                    hand: 1,
                    hands: 3,
                    opponent: "a very long opponent label that must be cut to fit the frame",
                    names: NAMES,
                };
                let mut lines = table_screen(&obs, &info, &ui);
                lines.extend(menu(&obs, &ui));
                for l in &lines {
                    assert!(visible_width(l) <= width, "{width}: {l:?}");
                }
                let text = lines.join("\n");
                for c in h.hole_cards(Seat::One) {
                    assert!(!text.contains(&ui.card_text(c)), "bot card leaked");
                }
                assert!(text.contains(ui.back_text()));
            }
        }
    }
}
