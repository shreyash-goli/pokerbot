//! Rendering of the post-hand coach and the replay viewer (M-18).
//!
//! Text comes from `pokerbot_analysis::coach`; this module only wraps it to the
//! width and applies the theme. Replay frames are rendered from the human's
//! rebuilt observations, so the opponent's cards appear only in the final
//! showdown result, exactly as during play.

use pokerbot_analysis::coach::{CoachLine, LineKind, ReplayFrame, Section};
use pokerbot_core::observation::PlayerObservation;

use super::table::{phrase, street_name};
use super::term::wrap;
use super::{Names, Role, Ui, screens};

/// Wraps and styles coach lines.
pub fn analysis(ui: &Ui, lines: &[CoachLine]) -> Vec<String> {
    let mut out = vec![ui.rule("")];
    for l in lines {
        let indent = if l.depth == 0 { "  " } else { "    " };
        let cont = if l.depth == 0 { "    " } else { "      " };
        let role = match (l.kind, l.section) {
            (LineKind::Heading, Section::Hindsight) => Some(Role::Accent),
            (LineKind::Heading, _) => Some(Role::Strong),
            (LineKind::Warn, _) => Some(Role::Warn),
            (LineKind::Note, _) => Some(Role::Dim),
            (LineKind::Text, _) => None,
        };
        for w in wrap(&l.text, ui.width, indent, cont) {
            out.push(match role {
                Some(r) => ui.paint(r, &w),
                None => w,
            });
        }
    }
    out.push(ui.rule(""));
    out
}

/// Title line of a replay.
pub fn replay_title(ui: &Ui, hand_number: u64, steps: usize) -> Vec<String> {
    let mut out = vec![ui.rule("")];
    out.extend(
        wrap(
            &format!("Replay of hand {hand_number}: {steps} action(s). Enter = next, q = stop."),
            ui.width,
            "  ",
            "    ",
        )
        .into_iter()
        .map(|l| ui.paint(Role::Strong, &l)),
    );
    out
}

/// One replay step.
pub fn replay_frame(ui: &Ui, f: &ReplayFrame, total: usize, names: Names) -> Vec<String> {
    let a = &f.action;
    let actor = a.seat.index();
    let you = a.seat == names.human;
    let text = phrase(
        a.kind,
        a.raise_to,
        a.chips_committed,
        f.street_contributions[actor],
        f.to_call,
        you,
    );
    let me = names.human.index();
    let line = format!(
        "Step {}/{total} | {} {} | pot {} | stacks You {} / Bot {} | {} {text}",
        f.step,
        street_name(f.street),
        ui.board(&f.board),
        f.pot,
        f.stacks[me],
        f.stacks[1 - me],
        names.of(a.seat)
    );
    wrap(&line, ui.width, "  ", "    ")
}

/// Final lines of a replay: the result as the human saw it.
pub fn replay_end(ui: &Ui, last: &PlayerObservation, names: Names) -> Vec<String> {
    let mut out = screens::result(ui, last, names);
    out.push(ui.rule(""));
    out
}
