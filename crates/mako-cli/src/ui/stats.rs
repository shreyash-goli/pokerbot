//! Session statistics panel.
//!
//! Rendering takes a plain [`SessionStats`] value, so richer statistics
//! (e.g. from `mako-analysis`, M-17) can be added as fields without the
//! panel touching game state.

use super::term::wrap;
use super::{Role, Ui, chips_delta};

/// Where the opponent's decisions came from, over completed hands only:
/// counts for the hand in progress would hint at the bot's private spot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BotSources {
    /// A trained policy with explicit fallbacks.
    Policy {
        decisions: u64,
        policy: u64,
        fallback: u64,
    },
    /// The untrained baseline.
    Untrained { decisions: u64 },
}

/// Everything the stats panel shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionStats {
    pub hands_completed: u64,
    /// Net chips for the human and the bot (sum to zero).
    pub you: i64,
    pub bot: i64,
    /// Big blind, for the per-hand rate.
    pub big_blind: u32,
    /// The opponent's honest label.
    pub opponent: String,
    pub sources: BotSources,
}

/// The panel lines.
pub fn render(s: &SessionStats, ui: &Ui) -> Vec<String> {
    let mut out = Vec::new();
    let score = format!(
        "Session: {} hand(s) completed. You {} / Bot {}",
        s.hands_completed,
        ui.paint(Role::Strong, &chips_delta(s.you)),
        chips_delta(s.bot)
    );
    out.extend(wrap(&score, ui.width, "  ", "    "));
    if s.hands_completed > 0 && s.big_blind > 0 {
        // Display only; chip accounting stays integer.
        let rate = s.you as f64 / f64::from(s.big_blind) / s.hands_completed as f64;
        out.extend(wrap(
            &format!("You: {rate:+.2} big blinds per hand"),
            ui.width,
            "  ",
            "    ",
        ));
    }
    let src = match s.sources {
        BotSources::Policy {
            decisions,
            policy,
            fallback,
        } => format!(
            "Bot decisions in completed hands: {decisions} total, {policy} from policy, \
             {fallback} explicit fallback (uniform legal, unseen infoset)"
        ),
        BotSources::Untrained { decisions } => format!(
            "Bot: {} ({decisions} decisions; no trained policy in use)",
            s.opponent
        ),
    };
    out.extend(
        wrap(&src, ui.width, "  ", "    ")
            .into_iter()
            .map(|l| ui.paint(Role::Dim, &l)),
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::term::visible_width;

    #[test]
    fn panel_fits_and_reports_sources() {
        let s = SessionStats {
            hands_completed: 4,
            you: -6,
            bot: 6,
            big_blind: 2,
            opponent: "cfr 20BB (500k it): smoke".into(),
            sources: BotSources::Policy {
                decisions: 9,
                policy: 7,
                fallback: 2,
            },
        };
        for width in [48, 80, 160] {
            let ui = Ui {
                width,
                ..Ui::plain()
            };
            let lines = render(&s, &ui);
            assert!(lines.iter().all(|l| visible_width(l) <= width));
            // Collapse wrap indentation before matching phrases.
            let text = lines
                .join(" ")
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            assert!(text.contains("Session: 4 hand(s) completed. You -6 / Bot +6"));
            assert!(text.contains("-0.75 big blinds per hand"));
            assert!(text.contains("7 from policy"));
        }
    }
}
