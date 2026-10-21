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

/// `(command, aliases, description)` accepted between hands. An empty
/// alias means none.
pub const BETWEEN_COMMANDS: [(&str, &str, &str); 6] = [
    ("continue", "Enter, c", "deal the next hand"),
    (
        "analysis",
        "a",
        "review the last hand: pot odds, your options, the fixed trained policy's \
         mix in your seat, and hindsight after a showdown ('analysis <n>' for saved \
         hand n)",
    ),
    (
        "replay",
        "r",
        "step through the last hand (Enter = next, q = stop); 'replay <n>' for \
         saved hand n",
    ),
    (
        "stats",
        "s",
        "session and lifetime stats and bot decision sources",
    ),
    ("help", "?", "show this help"),
    ("quit", "q", "end the session"),
];

/// Between-hands commands of the experimental adaptive opponent; listed
/// only when `--adapt` is active.
pub const ADAPT_COMMANDS: [(&str, &str, &str); 4] = [
    (
        "profile",
        "adapt",
        "what the adaptive bot has learned from your public actions, with \
         sample sizes",
    ),
    (
        "adapt off",
        "",
        "switch adaptation off: the fixed policy plays",
    ),
    ("adapt on", "", "switch adaptation back on"),
    ("reset-profile", "", "forget the learned profile"),
];

fn command_lines(ui: &Ui, cmds: &[(&str, &str, &str)]) -> Vec<String> {
    // Column widths from the table itself, so long names stay aligned.
    let cmd_w = cmds.iter().map(|c| c.0.len()).max().unwrap_or(0);
    let alias_w = cmds
        .iter()
        .map(|c| if c.1.is_empty() { 0 } else { c.1.len() + 2 })
        .max()
        .unwrap_or(0);
    let mut out = Vec::new();
    for (cmd, alias, desc) in cmds {
        let alias = if alias.is_empty() {
            String::new()
        } else {
            format!("({alias})")
        };
        // Pad the plain text, then colour it, so escapes do not skew columns.
        let head = format!(
            "    {}{} {:<alias_w$} ",
            ui.paint(Role::Accent, cmd),
            " ".repeat(cmd_w - cmd.len()),
            alias
        );
        let cont = " ".repeat(6 + cmd_w + alias_w);
        out.extend(wrap(desc, ui.width, &head, &cont));
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

/// Help shown between hands. The adaptive commands are listed only when
/// `adapt` (`--adapt`) is active.
pub fn between_hands(ui: &Ui, opponent: &str, adapt: bool) -> Vec<String> {
    let mut out = vec![ui.paint(Role::Strong, "  Commands between hands:")];
    out.extend(command_lines(ui, &BETWEEN_COMMANDS));
    if adapt {
        out.push(ui.paint(
            Role::Strong,
            "  --adapt only (experimental adaptive opponent):",
        ));
        out.extend(command_lines(ui, &ADAPT_COMMANDS));
    }
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
             Difficulty (--difficulty, CFR only): challenging = the trained policy \
             as-is (default); balanced = 75% policy + 25% uniform-random legal; \
             training = 40% policy + 60% uniform-random legal. These are blends of a \
             plumbing-grade smoke policy; relative strength is not benchmarked. \
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
        for (cmd, alias, _) in BETWEEN_COMMANDS.iter().chain(&ADAPT_COMMANDS) {
            let (cmd, alias) = (*cmd, *alias);
            for word in std::iter::once(cmd).chain(alias.split(", ").filter(|w| !w.is_empty())) {
                let word = if word == "Enter" { "" } else { word };
                assert_ne!(parse_between(word), BetweenHands::Invalid, "{word:?}");
            }
        }
    }

    #[test]
    fn adapt_commands_only_with_adapt_and_aligned() {
        let ui = Ui {
            width: 120,
            ..Ui::plain()
        };
        let off = between_hands(&ui, "x", false).join("\n");
        assert!(
            !off.contains("reset-profile") && !off.contains("profile "),
            "{off}"
        );
        let on = between_hands(&ui, "x", true);
        let text = on.join("\n");
        assert!(text.contains("--adapt only"));
        assert!(!text.contains("(-)"), "no placeholder alias");
        // Descriptions of one table start in the same column.
        let starts: Vec<usize> = ["deal the next", "show this help", "end the session"]
            .iter()
            .map(|d| on.iter().find_map(|l| l.find(d)).unwrap())
            .collect();
        assert!(starts.windows(2).all(|w| w[0] == w[1]), "{starts:?}");
        let adapt_starts: Vec<usize> = ["forget the learned", "switch adaptation back"]
            .iter()
            .map(|d| on.iter().find_map(|l| l.find(d)).unwrap())
            .collect();
        assert_eq!(adapt_starts[0], adapt_starts[1]);
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
                .chain(&between_hands(&ui, "x", false))
                .chain(&between_hands(&ui, "x", true))
            {
                assert!(visible_width(l) <= width, "{l:?}");
            }
        }
    }
}
