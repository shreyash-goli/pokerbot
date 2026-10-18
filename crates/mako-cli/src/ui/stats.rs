//! Session statistics panel.
//!
//! Rendering takes a plain [`SessionStats`] value and never touches game
//! state. The optional [`HistoryView`] carries `mako-analysis` statistics
//! (M-17), which are computed only from the public decision history of
//! completed hands.

use mako_analysis::stats::{PlayerStats, SessionStats as AnalysisStats, SplitStats};
use mako_analysis::stats::{fmt_num, fmt_pct};

use super::term::wrap;
use super::{Role, Ui, chips_delta};

/// Statistics from the saved hand history (M-17).
#[derive(Clone, Debug, PartialEq)]
pub struct HistoryView {
    /// Hands completed in this run of the program.
    pub session: AnalysisStats,
    /// Every hand in the history file, including this session's.
    pub lifetime: AnalysisStats,
    /// Where the history lives, e.g. `saved to /path` or `not saved`.
    pub storage: String,
}

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
#[derive(Clone, Debug, PartialEq)]
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
    /// Detailed statistics; `None` renders the score panel only.
    pub history: Option<HistoryView>,
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
    if let Some(h) = &s.history {
        let l = &h.lifetime.human.overall;
        out.extend(wrap(
            &format!(
                "Lifetime: {} hand(s), You {} chips, {} BB/100 ('stats' for details)",
                l.hands,
                chips_delta(l.net_chips),
                fmt_num(l.bb_per_100())
            ),
            ui.width,
            "  ",
            "    ",
        ));
    }
    out
}

/// The full panel for the `stats` command: the score panel, then session
/// and lifetime statistics for both players with positional splits.
pub fn render_full(s: &SessionStats, ui: &Ui) -> Vec<String> {
    let mut out = render(s, ui);
    let Some(h) = &s.history else {
        return out;
    };
    for (title, st) in [
        ("This session", &h.session),
        ("Lifetime (saved history)", &h.lifetime),
    ] {
        out.push(ui.paint(Role::Strong, &format!("  {title}: {} hand(s)", st.hands())));
        for (name, p) in [("You", &st.human), ("Bot", &st.opponent)] {
            out.extend(player_lines(name, p, ui));
        }
    }
    let note = format!(
        "From public actions only. VPIP: voluntarily put chips in preflop (call or \
         raise; the big blind checking its option is not VPIP; walks are excluded). \
         PFR: preflop bet/raise. AF = (bets+raises)/calls; AFq = (bets+raises)/\
         (bets+raises+calls+folds); W$SD = showdowns won. History {}.",
        h.storage
    );
    out.extend(
        wrap(&note, ui.width, "  ", "    ")
            .into_iter()
            .map(|l| ui.paint(Role::Dim, &l)),
    );
    out
}

fn split_text(label: &str, s: &SplitStats) -> String {
    format!(
        "{label} {} hands, net {}, VPIP {}, PFR {}",
        s.hands,
        chips_delta(s.net_chips),
        fmt_pct(s.vpip()),
        fmt_pct(s.pfr())
    )
}

fn player_lines(name: &str, p: &PlayerStats, ui: &Ui) -> Vec<String> {
    let o = &p.overall;
    let head = format!("    {name:<4}");
    let mut out = wrap(
        &format!(
            "net {} chips | {} BB/100 | VPIP {} | PFR {} | AF {} | AFq {} | \
             W$SD {} of {} showdown(s), {} split",
            chips_delta(o.net_chips),
            fmt_num(o.bb_per_100()),
            fmt_pct(o.vpip()),
            fmt_pct(o.pfr()),
            fmt_num(p.aggression_factor()),
            fmt_pct(p.aggression_frequency()),
            fmt_pct(p.showdown_win_rate()),
            p.showdowns,
            p.showdowns_split
        ),
        ui.width,
        &head,
        "         ",
    );
    out.extend(wrap(
        &format!(
            "{} | {}",
            split_text("button:", &p.button),
            split_text("big blind:", &p.big_blind)
        ),
        ui.width,
        "         ",
        "         ",
    ));
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
            history: None,
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
            assert_eq!(render_full(&s, &ui), lines, "no history: same panel");
        }
    }

    #[test]
    fn full_panel_fits_and_shows_na() {
        let s = SessionStats {
            hands_completed: 0,
            you: 0,
            bot: 0,
            big_blind: 2,
            opponent: "baseline".into(),
            sources: BotSources::Untrained { decisions: 0 },
            history: Some(HistoryView {
                session: AnalysisStats::new(),
                lifetime: AnalysisStats::new(),
                storage: "not saved (--no-history)".into(),
            }),
        };
        for width in [48, 80, 160] {
            let ui = Ui {
                width,
                ..Ui::plain()
            };
            let lines = render_full(&s, &ui);
            assert!(lines.iter().all(|l| visible_width(l) <= width));
            let text = lines.join(" ");
            assert!(text.contains("VPIP n/a"), "{text}");
            assert!(text.contains("Lifetime (saved history): 0 hand(s)"));
            assert!(!text.contains("NaN"));
        }
    }
}
