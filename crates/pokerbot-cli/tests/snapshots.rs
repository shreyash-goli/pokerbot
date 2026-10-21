//! Golden snapshots of the terminal UI.
//!
//! Fixed seeded states (preflop, flop, turn, river, showdown, fold) and the
//! session header are rendered at widths 80 and 120, in two styles:
//! `plain` (no colour, ASCII glyphs) and `256` (256 colours, Unicode
//! glyphs). Output is compared with `tests/snapshots/<name>.txt`; escape
//! bytes are stored as the two characters `\e`. Regenerate with
//! `UPDATE_SNAPSHOTS=1 cargo test -p pokerbot-cli --test snapshots`.

use std::path::PathBuf;

use pokerbot_cli::game::{BOT, HUMAN};
use pokerbot_cli::ui::screens::{self, Banner, BannerLevel};
use pokerbot_cli::ui::table::{self, TableInfo};
use pokerbot_cli::ui::term::{strip_ansi, visible_width};
use pokerbot_cli::ui::{ColorMode, Glyphs, Names, SHOWDOWN_MARKER, Ui};
use pokerbot_core::action::ActionKind::{self, *};
use pokerbot_core::rules::{RulesConfig, StackDepth};
use pokerbot_core::state::{HandConfig, HandState};

const SEED: u64 = 7;
const NAMES: Names = Names { human: HUMAN };
const LABEL: &str = "cfr 20BB (500k it): smoke: trained for plumbing, not competition";

fn states() -> Vec<(&'static str, Vec<ActionKind>)> {
    vec![
        ("preflop", vec![]),
        ("flop", vec![Call, Check, HalfPot]),
        ("turn", vec![Call, Check, HalfPot, Call, Check]),
        ("river", vec![Call, Check, HalfPot, Call, Check, Check, Pot]),
        (
            "showdown",
            vec![Call, Check, HalfPot, Call, Check, Check, Pot, Call],
        ),
        ("fold", vec![HalfPot, Fold]),
    ]
}

fn hand(actions: &[ActionKind]) -> HandState {
    let config = HandConfig {
        rules: RulesConfig::standard(StackDepth::Bb20),
        button: HUMAN,
    };
    let mut h = HandState::from_seed(config, SEED).unwrap();
    for &a in actions {
        h.apply(a).unwrap_or_else(|e| panic!("{a:?}: {e}"));
    }
    h
}

fn styles() -> [(&'static str, ColorMode, Glyphs); 2] {
    [
        ("plain", ColorMode::None, Glyphs::Ascii),
        ("256", ColorMode::Ansi256, Glyphs::Unicode),
    ]
}

/// The screen for a state: the table, then the menu (human to act) or the
/// result (terminal). Rendered from the human observation only.
fn render_state(h: &HandState, ui: &Ui) -> Vec<String> {
    let obs = h.observation(HUMAN);
    let info = TableInfo {
        hand: 2,
        hands: 5,
        opponent: LABEL,
        names: NAMES,
    };
    let mut lines = table::table_screen(&obs, &info, ui);
    if obs.terminal.is_some() {
        lines.extend(screens::result(ui, &obs, NAMES));
    } else {
        assert!(obs.is_my_turn(), "fixtures stop at the human's decision");
        lines.extend(table::menu(&obs, ui));
    }
    lines
}

fn header(ui: &Ui, level: BannerLevel) -> Vec<String> {
    let banner = match level {
        BannerLevel::Info => Banner {
            level,
            lines: vec![
                format!("Opponent: CFR policy -- {LABEL}"),
                "artifact artifacts/smoke_20bb.pokerbot | checksum 0xd2767c7dad569fdd | 39741 infosets"
                    .into(),
            ],
        },
        BannerLevel::Warning => Banner {
            level,
            lines: vec![
                "No trained policy for 50BB at artifacts/smoke_50bb.pokerbot (missing: No such file or directory (os error 2)).".into(),
                "Playing the UNTRAINED baseline: baseline (untrained, random legal).".into(),
            ],
        },
    };
    let setup = vec![
        "Session seed 7 | 5 hand(s) | 20 BB deep (40 chips) | blinds 1/2".to_string(),
        "Replay: pokerbot --seed 7 --hands 5 --depth 20 --bot auto --strategy artifacts/smoke_20bb.pokerbot".to_string(),
    ];
    screens::session_header(ui, "0.1.0", &banner, &setup)
}

fn snapshot_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots")
}

fn encode(lines: &[String]) -> String {
    let mut s = lines.join("\n").replace('\x1b', "\\e");
    s.push('\n');
    s
}

fn check(name: &str, lines: &[String], failures: &mut Vec<String>) {
    let path = snapshot_dir().join(format!("{name}.txt"));
    let got = encode(lines);
    if std::env::var_os("UPDATE_SNAPSHOTS").is_some() {
        std::fs::create_dir_all(snapshot_dir()).unwrap();
        std::fs::write(&path, &got).unwrap();
        return;
    }
    match std::fs::read_to_string(&path) {
        Ok(want) if want == got => {}
        Ok(want) => {
            let line = want
                .lines()
                .zip(got.lines())
                .position(|(a, b)| a != b)
                .unwrap_or(want.lines().count().min(got.lines().count()));
            failures.push(format!(
                "{name}: differs at line {}\n  want: {:?}\n  got:  {:?}",
                line + 1,
                want.lines().nth(line).unwrap_or(""),
                got.lines().nth(line).unwrap_or("")
            ));
        }
        Err(_) => failures.push(format!(
            "{name}: missing {} (run with UPDATE_SNAPSHOTS=1)",
            path.display()
        )),
    }
}

/// Width, colour and hidden-card invariants of one rendering.
fn check_invariants(name: &str, lines: &[String], ui: &Ui, h: Option<&HandState>) {
    for l in lines {
        assert!(
            visible_width(l) <= ui.width,
            "{name}: line wider than {}: {:?}",
            ui.width,
            strip_ansi(l)
        );
    }
    let text = lines.join("\n");
    match ui.color {
        ColorMode::None => assert!(!text.contains('\x1b'), "{name}: escapes in plain mode"),
        ColorMode::Ansi256 => assert!(text.contains("\x1b[38;5;"), "{name}: no 256-colour codes"),
        ColorMode::TrueColor => assert!(text.contains("\x1b[38;2;"), "{name}: no truecolor codes"),
        ColorMode::Ansi16 => {
            assert!(text.contains('\x1b'), "{name}: no colour codes");
            assert!(
                !text.contains("38;5;") && !text.contains("38;2;"),
                "{name}: deep colour in 16 mode"
            );
        }
    }
    if let Some(h) = h {
        let plain = strip_ansi(&text);
        let bot: Vec<String> = h.hole_cards(BOT).iter().map(|&c| ui.card_text(c)).collect();
        match plain.find(SHOWDOWN_MARKER) {
            Some(pos) => {
                for c in &bot {
                    // The showdown table row reveals them too, legitimately:
                    // the observation itself carries them only at showdown.
                    assert!(h.observation(HUMAN).opponent_hole_cards.is_some());
                    assert!(
                        plain[pos..].contains(c.as_str()),
                        "{name}: {c} not revealed"
                    );
                }
            }
            None => {
                for c in &bot {
                    assert!(!plain.contains(c.as_str()), "{name}: bot card {c} leaked");
                }
                assert!(plain.contains(ui.back_text()), "{name}: no card backs");
            }
        }
        for c in h.hole_cards(HUMAN) {
            assert!(plain.contains(&ui.card_text(c)), "{name}: own card missing");
        }
    }
}

#[test]
fn golden_snapshots() {
    let mut failures = Vec::new();
    for width in [80, 120] {
        for (style, color, glyphs) in styles() {
            let ui = Ui {
                color,
                glyphs,
                width,
            };
            for (state, actions) in states() {
                let h = hand(&actions);
                let lines = render_state(&h, &ui);
                let name = format!("{state}_w{width}_{style}");
                check_invariants(&name, &lines, &ui, Some(&h));
                check(&name, &lines, &mut failures);
            }
            for (tag, level) in [
                ("header_info", BannerLevel::Info),
                ("header_warning", BannerLevel::Warning),
            ] {
                let lines = header(&ui, level);
                let name = format!("{tag}_w{width}_{style}");
                check_invariants(&name, &lines, &ui, None);
                check(&name, &lines, &mut failures);
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn every_width_fits_in_every_mode() {
    for width in (48..=240).step_by(5) {
        for color in [
            ColorMode::None,
            ColorMode::Ansi16,
            ColorMode::Ansi256,
            ColorMode::TrueColor,
        ] {
            for glyphs in [Glyphs::Ascii, Glyphs::Unicode] {
                let ui = Ui {
                    color,
                    glyphs,
                    width,
                };
                for (state, actions) in states() {
                    let h = hand(&actions);
                    let name = format!("{state}@{width}/{color:?}/{glyphs:?}");
                    check_invariants(&name, &render_state(&h, &ui), &ui, Some(&h));
                }
                for level in [BannerLevel::Info, BannerLevel::Warning] {
                    let lines = header(&ui, level);
                    for l in &lines {
                        assert!(visible_width(l) <= width, "header@{width}: {l:?}");
                    }
                }
            }
        }
    }
}

#[test]
fn compact_screen_fits_24_lines() {
    let ui = Ui {
        color: ColorMode::None,
        glyphs: Glyphs::Ascii,
        width: 80,
    };
    for (state, actions) in states() {
        let lines = render_state(&hand(&actions), &ui);
        // Leaves room for the hand divider, bot actions and the prompt.
        assert!(lines.len() <= 16, "{state}: {} lines", lines.len());
    }
}
