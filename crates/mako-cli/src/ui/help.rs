//! Help text. It lists exactly the commands the input parser accepts
//! (see `crate::input`); a test keeps the two in sync.

use super::term::wrap;
use super::{Role, Ui};

/// `(command, aliases, description)` accepted during the player's turn,
/// besides the action keys.
pub const IN_HAND_COMMANDS: [(&str, &str, &str); 3] = [
    ("help", "?", "show this help"),
    (
        "stats",
        "s",
        "session and lifetime stats (VPIP, PFR, aggression, showdowns, button/BB) \
         and where the bot's decisions came from",
    ),
    (
        "quit",
        "q",
        "end the session (an unfinished hand is not scored or saved)",
    ),
];

/// `(command, aliases, description)` accepted between hands.
pub const BETWEEN_COMMANDS: [(&str, &str, &str); 4] = [
    ("continue", "Enter, c", "deal the next hand"),
    (
        "stats",
        "s",
        "session and lifetime stats and bot decision sources",
    ),
    ("help", "?", "show this help"),
    ("quit", "q", "end the session"),
];

fn command_lines(ui: &Ui, cmds: &[(&str, &str, &str)]) -> Vec<String> {
    let mut out = Vec::new();
    for (cmd, alias, desc) in cmds {
        // Pad the plain text, then colour it, so escapes do not skew columns.
        let head = format!(
            "    {}{} {:<10} ",
            ui.paint(Role::Accent, cmd),
            " ".repeat(9usize.saturating_sub(cmd.len())),
            format!("({alias})")
        );
        out.extend(wrap(desc, ui.width, &head, &" ".repeat(25)));
    }
    out
}

/// Help shown during the player's turn. `opponent` is the current label.
pub fn in_hand(ui: &Ui, opponent: &str) -> Vec<String> {
    let mut out = vec![ui.paint(Role::Strong, "  Commands:")];
    out.extend(wrap(
        "Action keys: f fold, c check or call, k check, h half-pot bet/raise, \
         p pot bet/raise, a all-in; or the action's number in the Options list; \
         or the words fold, check, call, half, pot, allin (all-in). \
         Only the actions listed under Options are legal right now; anything \
         else is rejected and nothing changes.",
        ui.width,
        "    ",
        "    ",
    ));
    out.extend(command_lines(ui, &IN_HAND_COMMANDS));
    out.extend(stat_terms(ui));
    out.extend(opponent_modes(ui, opponent));
    out
}

/// Help shown between hands.
pub fn between_hands(ui: &Ui, opponent: &str) -> Vec<String> {
    let mut out = vec![ui.paint(Role::Strong, "  Commands between hands:")];
    out.extend(command_lines(ui, &BETWEEN_COMMANDS));
    out.extend(stat_terms(ui));
    out.extend(opponent_modes(ui, opponent));
    out
}

/// Short definitions of the statistics shown by `stats`.
fn stat_terms(ui: &Ui) -> Vec<String> {
    wrap(
        "Stats use public actions of completed hands only. VPIP = share of hands \
         where a player voluntarily put chips in preflop (a call or raise; the big \
         blind checking its option is not VPIP; walks, where the big blind never \
         acts, are excluded). PFR = share with a preflop bet or raise (a limp is \
         not PFR). n/a means there is nothing to divide by yet. Completed hands \
         are saved to the history file (see --history, --no-history, --reset-stats).",
        ui.width,
        "  ",
        "    ",
    )
}

fn opponent_modes(ui: &Ui, opponent: &str) -> Vec<String> {
    wrap(
        &format!(
            "Opponent modes (--bot): cfr = a trained CFR artifact (approximate, \
             not GTO); baseline = untrained, random legal; auto = cfr if a valid \
             artifact for this depth loads, otherwise baseline with a warning. \
             Playing now: {opponent}."
        ),
        ui.width,
        "  ",
        "    ",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::{BetweenHands, InHand, parse_between, parse_in_hand};
    use crate::ui::term::visible_width;

    #[test]
    fn listed_commands_all_work() {
        for (cmd, alias, _) in IN_HAND_COMMANDS {
            for word in std::iter::once(cmd).chain(alias.split(", ")) {
                let parsed = parse_in_hand(word, &[]);
                assert!(
                    matches!(parsed, InHand::Help | InHand::Stats | InHand::Quit),
                    "{word:?} -> {parsed:?}"
                );
            }
        }
        for (cmd, alias, _) in BETWEEN_COMMANDS {
            for word in std::iter::once(cmd).chain(alias.split(", ")) {
                let word = if word == "Enter" { "" } else { word };
                assert_ne!(parse_between(word), BetweenHands::Invalid, "{word:?}");
            }
        }
    }

    #[test]
    fn help_fits() {
        for width in [48, 80, 120] {
            let ui = Ui {
                width,
                ..Ui::plain()
            };
            for l in in_hand(&ui, "baseline")
                .iter()
                .chain(&between_hands(&ui, "x"))
            {
                assert!(visible_width(l) <= width, "{l:?}");
            }
        }
    }
}
