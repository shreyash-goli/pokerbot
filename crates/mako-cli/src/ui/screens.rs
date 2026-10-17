//! Session and hand screens other than the table: the title header,
//! opponent banners, hand and street dividers, action events, results and
//! prompts. Everything is built from public information only.

use mako_core::action::{ActionRecord, Seat, TerminalReason};
use mako_core::card::Card;
use mako_core::eval;
use mako_core::observation::PlayerObservation;

use super::table::{describe_live, street_name};
use super::term::{fit, visible_width, wrap};
use super::{Glyphs, Names, Role, SHOWDOWN_MARKER, Ui, chips_delta};

/// Severity of an opponent banner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BannerLevel {
    /// Plain information (which policy is playing).
    Info,
    /// A loud warning (e.g. falling back to the untrained baseline).
    Warning,
}

/// Lines announcing the opponent; rendered wrapped to the width.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Banner {
    pub level: BannerLevel,
    pub lines: Vec<String>,
}

/// A 3-row "pixel" logo drawn with half-block characters (original art).
const LOGO: [&str; 3] = [
    "\u{2588}\u{2584} \u{2584}\u{2588} \u{2584}\u{2580}\u{2580}\u{2584} \u{2588} \u{2584}\u{2580} \u{2584}\u{2580}\u{2580}\u{2584}",
    "\u{2588} \u{2580} \u{2588} \u{2588}\u{2580}\u{2580}\u{2588} \u{2588}\u{2580}\u{2584}  \u{2588}  \u{2588}",
    "\u{2580}   \u{2580} \u{2580}  \u{2580} \u{2580}  \u{2580}  \u{2580}\u{2580} ",
];

/// Title block, banner and setup lines at the start of a session.
pub fn session_header(ui: &Ui, version: &str, banner: &Banner, setup: &[String]) -> Vec<String> {
    let tagline = format!(
        "Mako {version} {} heads-up no-limit hold'em",
        ui.frame().dot
    );
    let mut out = Vec::new();
    match ui.glyphs {
        Glyphs::Unicode => {
            for (i, row) in LOGO.iter().enumerate() {
                let mut line = format!("  {}", ui.paint(Role::Title, row));
                if i == 1 {
                    line.push_str("   ");
                    line.push_str(&ui.paint(Role::Label, &tagline));
                }
                out.push(fit(&line, ui.width));
            }
        }
        Glyphs::Ascii => {
            out.push(fit(
                &format!(
                    "  {}   {}",
                    ui.paint(Role::Title, "[ M A K O ]"),
                    ui.paint(Role::Label, &tagline)
                ),
                ui.width,
            ));
        }
    }
    out.push(ui.rule(""));
    out.extend(banner_lines(ui, banner));
    for s in setup {
        if s.starts_with("Replay: ") {
            out.extend(shell_wrap(s, ui.width));
        } else {
            out.extend(wrap(s, ui.width, "", "    "));
        }
    }
    out.push(ui.paint(
        Role::Dim,
        &fit("Type 'help' at any prompt for the commands.", ui.width),
    ));
    out
}

/// Wraps a command line so it still pastes into a shell: every line but
/// the last ends with ` \`.
pub fn shell_wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = wrap(text, width.saturating_sub(2), "", "    ");
    let n = lines.len();
    for l in lines.iter_mut().take(n.saturating_sub(1)) {
        l.push_str(" \\");
    }
    lines
}

/// The opponent banner; warnings are framed in `!!` bars (text markers,
/// so they are unmistakable without colour).
pub fn banner_lines(ui: &Ui, banner: &Banner) -> Vec<String> {
    let mut out = Vec::new();
    match banner.level {
        BannerLevel::Info => {
            for (i, l) in banner.lines.iter().enumerate() {
                let indent = if i == 0 { "" } else { "  " };
                out.extend(wrap(l, ui.width, indent, "    "));
            }
        }
        BannerLevel::Warning => {
            let bar = ui.paint(Role::Warn, &"!".repeat(ui.width));
            out.push(bar.clone());
            for l in &banner.lines {
                for w in wrap(l, ui.width, "!! ", "!!    ") {
                    out.push(ui.paint(Role::Warn, &w));
                }
            }
            out.push(bar);
        }
    }
    out
}

/// Divider and blinds line at the start of a hand.
pub fn hand_header(
    ui: &Ui,
    obs: &PlayerObservation,
    hand: u64,
    hands: u64,
    names: Names,
) -> Vec<String> {
    let b = obs.button;
    let dot = ui.frame().dot;
    vec![
        ui.rule(&format!(
            "Hand {hand} of {hands} {dot} button: {}",
            names.of(b)
        )),
        fit(
            &format!(
                "  Blinds: {} {} (SB) {dot} {} {} (BB) {dot} your cards {}",
                names.of(b),
                obs.street_contributions[b.index()],
                names.of(b.other()),
                obs.street_contributions[b.other().index()],
                ui.cards(&obs.hole_cards)
            ),
            ui.width,
        ),
    ]
}

/// One action as it happens: `  ▸ Bot raises to 6 (pot)`.
pub fn action_event(
    ui: &Ui,
    rec: &ActionRecord,
    before: &PlayerObservation,
    names: Names,
) -> String {
    let who = names.of(rec.seat);
    let text = describe_live(rec, before, rec.seat == names.human);
    let who = if rec.seat == names.human {
        who.to_string()
    } else {
        ui.paint(Role::Label, who)
    };
    fit(&format!("  {} {who} {text}", ui.frame().arrow), ui.width)
}

/// Divider when a new street is dealt: `── Flop [7c][Jd][2c] · pot 12 ──…`.
pub fn street_banner(ui: &Ui, obs: &PlayerObservation) -> String {
    let f = ui.frame();
    let r2: String = std::iter::repeat_n(f.rule, 2).collect();
    let head = format!(
        "{} {} {} {}",
        ui.paint(Role::Frame, &r2),
        ui.paint(Role::Title, street_name(obs.street)),
        ui.cards(&obs.board),
        ui.paint(Role::Frame, &format!("{} pot {} ", f.dot, obs.pot))
    );
    let rest = ui.width.saturating_sub(visible_width(&head));
    let fill: String = std::iter::repeat_n(f.rule, rest).collect();
    fit(&format!("{head}{}", ui.paint(Role::Frame, &fill)), ui.width)
}

fn category(hole: [Card; 2], board: &[Card]) -> &'static str {
    if board.len() != 5 {
        return "?";
    }
    let seven: Vec<Card> = hole.iter().chain(board).copied().collect();
    eval::evaluate(&seven).category().name()
}

/// End-of-hand result from the human's observation of a terminal state.
pub fn result(ui: &Ui, obs: &PlayerObservation, names: Names) -> Vec<String> {
    let me = obs.seat;
    let opp: Seat = me.other();
    let mut out = Vec::new();
    match obs.terminal {
        Some(TerminalReason::Fold { winner }) => {
            let loser = winner.other();
            let (folds, wins) = if loser == names.human {
                ("fold", "wins")
            } else {
                ("folds", "win")
            };
            out.push(format!(
                "  {} {folds}. {} {wins} the pot.",
                names.of(loser),
                names.of(winner)
            ));
            out.push(format!(
                "  Board {}   Bot's cards stay hidden: {}",
                ui.board(&obs.board),
                ui.backs()
            ));
        }
        Some(TerminalReason::Showdown) => {
            out.push(ui.paint(Role::Title, SHOWDOWN_MARKER));
            out.push(format!("  Board {}", ui.cards(&obs.board)));
            out.push(format!(
                "  {:<3}   {}  {}",
                names.of(me),
                ui.cards(&obs.hole_cards),
                category(obs.hole_cards, &obs.board)
            ));
            if let Some(oc) = obs.opponent_hole_cards {
                out.push(format!(
                    "  {:<3}   {}  {}",
                    names.of(opp),
                    ui.cards(&oc),
                    category(oc, &obs.board)
                ));
            }
        }
        None => out.push("  (hand not finished)".to_string()),
    }
    if let Some(p) = obs.payoffs {
        let mine = p[me.index()];
        let (verdict, role) = match mine.cmp(&0) {
            std::cmp::Ordering::Greater => ("You win.", Role::Win),
            std::cmp::Ordering::Less => ("Bot wins.", Role::Lose),
            std::cmp::Ordering::Equal => ("Split pot.", Role::Strong),
        };
        out.push(format!(
            "  {} You {} / Bot {}",
            ui.paint(role, verdict),
            ui.paint(Role::Strong, &chips_delta(mine)),
            chips_delta(p[opp.index()])
        ));
    }
    out.into_iter().map(|l| fit(&l, ui.width)).collect()
}

/// The in-hand prompt (no newline).
pub fn action_prompt(ui: &Ui) -> String {
    format!("  {} ", ui.paint(Role::Accent, "Your action>"))
}

/// The between-hands prompt (no newline).
pub fn between_prompt(ui: &Ui) -> String {
    let dot = ui.frame().dot;
    format!(
        "  {} next hand {dot} stats {dot} help {dot} quit > ",
        ui.paint(Role::Accent, "[Enter]")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::ColorMode;

    #[test]
    fn replay_hint_wraps_as_shell_continuations() {
        let cmd = "Replay: mako --seed 7 --hands 5 --depth 20 --bot auto --strategy artifacts/smoke_20bb.mako --auto checkcall";
        for width in [48, 80] {
            let lines = shell_wrap(cmd, width);
            assert!(lines.len() > 1);
            assert!(lines.iter().all(|l| visible_width(l) <= width));
            let n = lines.len();
            assert!(lines[..n - 1].iter().all(|l| l.ends_with(" \\")));
            let rejoined: Vec<&str> = lines
                .iter()
                .map(|l| l.trim_end_matches('\\').trim())
                .collect();
            assert_eq!(rejoined.join(" "), cmd);
        }
        assert_eq!(
            shell_wrap("Replay: mako", 80),
            vec!["Replay: mako".to_string()]
        );
    }

    #[test]
    fn logo_rows_are_equal_width() {
        let w = LOGO[0].chars().count();
        assert!(LOGO.iter().all(|r| r.chars().count() == w), "{w}");
    }

    #[test]
    fn warning_banner_has_text_markers_and_fits() {
        let b = Banner {
            level: BannerLevel::Warning,
            lines: vec![
                "No trained policy for 20BB at some/really/long/path/to/an/artifact.mako (missing: No such file or directory).".into(),
                "Playing the UNTRAINED baseline.".into(),
            ],
        };
        for width in [48, 80, 140] {
            for color in [ColorMode::None, ColorMode::Ansi16] {
                let ui = Ui {
                    color,
                    glyphs: Glyphs::Ascii,
                    width,
                };
                let lines = banner_lines(&ui, &b);
                assert!(lines.iter().all(|l| visible_width(l) <= width));
                let plain: Vec<String> = lines
                    .iter()
                    .map(|l| crate::ui::term::strip_ansi(l))
                    .collect();
                assert!(plain[1].starts_with("!! No trained policy"));
                assert!(plain.iter().skip(1).all(|l| l.starts_with("!!")));
            }
        }
    }
}
