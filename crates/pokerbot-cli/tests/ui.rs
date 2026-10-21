//! M-15: the binary's UI behaviour (width, colour, glyphs, commands, flags),
//! always against the untrained baseline so no artifact is needed.

mod common;

use common::*;
use pokerbot_cli::game::{BOT, HUMAN, hand_deck};
use pokerbot_cli::ui::term::{strip_ansi, visible_width};
use pokerbot_cli::ui::{ColorMode, Glyphs, SHOWDOWN_MARKER, Ui};
use pokerbot_core::rules::{RulesConfig, StackDepth};
use pokerbot_core::state::{HandConfig, HandState};

fn base(extra: &[&'static str]) -> Vec<&'static str> {
    let mut a = vec!["--bot", "baseline", "--seed", "5", "--hands", "4"];
    a.extend_from_slice(extra);
    a
}

fn assert_fits(out: &str, width: usize) {
    for l in out.lines() {
        assert!(
            visible_width(l) <= width,
            "line wider than {width}: {:?}",
            strip_ansi(l)
        );
    }
}

#[test]
fn ui_width_flag_and_columns_env_are_respected() {
    for width in ["48", "80", "100", "133", "240"] {
        let o = pokerbot(
            &base(&["--width", width, "--auto", "random", "--glyphs", "unicode"]),
            "",
        );
        assert!(o.status.success());
        assert_fits(&stdout(&o), width.parse().unwrap());
    }
    for (cols, expect) in [("100", 100), ("60", 60), ("20", 48), ("junk", 80)] {
        let o = pokerbot_env(&base(&["--auto", "checkcall"]), "", &[("COLUMNS", cols)]);
        let out = stdout(&o);
        assert_fits(&out, expect);
        // The hand divider spans exactly the chosen width.
        let rule = out
            .lines()
            .find(|l| l.starts_with("-- Hand 1 of 4"))
            .unwrap();
        assert_eq!(visible_width(rule), expect, "COLUMNS={cols}");
    }
    let wide = stdout(&pokerbot_env(
        &base(&["--auto", "checkcall"]),
        "",
        &[("COLUMNS", "120")],
    ));
    assert!(wide.contains("Action log"), "wide layout has the log pane");
    let narrow = stdout(&pokerbot(&base(&["--auto", "checkcall"]), ""));
    assert!(!narrow.contains("Action log") && narrow.contains("  Log: "));
}

#[test]
fn ui_colour_modes() {
    let plain = stdout(&pokerbot(&base(&["--auto", "checkcall"]), ""));
    assert!(
        !plain.contains('\x1b'),
        "non-TTY output is plain by default"
    );
    let c256 = stdout(&pokerbot(
        &base(&["--auto", "checkcall", "--color", "256"]),
        "",
    ));
    assert!(c256.contains("\x1b[38;5;"));
    let tc = stdout(&pokerbot(
        &base(&["--auto", "checkcall", "--color", "truecolor"]),
        "",
    ));
    assert!(tc.contains("\x1b[38;2;"));
    let c16 = stdout(&pokerbot(
        &base(&["--auto", "checkcall", "--color", "16"]),
        "",
    ));
    assert!(c16.contains('\x1b') && !c16.contains("38;5;"));
    // Same game, only styling differs.
    assert_eq!(strip_ansi(&c256), plain);
    assert_eq!(strip_ansi(&tc), plain);
    let nc = stdout(&pokerbot_env(
        &base(&["--auto", "checkcall"]),
        "",
        &[("NO_COLOR", "1"), ("COLORTERM", "truecolor")],
    ));
    assert!(!nc.contains('\x1b'));
}

#[test]
fn ui_unicode_glyphs_never_leak_bot_cards() {
    let ui = Ui {
        color: ColorMode::None,
        glyphs: Glyphs::Unicode,
        width: 80,
    };
    let mut showdowns = 0;
    for seed in 0..10u64 {
        let s = seed.to_string();
        let o = pokerbot(
            &[
                "--bot", "baseline", "--seed", &s, "--hands", "5", "--auto", "random", "--glyphs",
                "unicode",
            ],
            "",
        );
        let out = stdout(&o);
        assert!(out.contains('\u{2593}'), "unicode card backs");
        for (i, section) in out.split("\n\u{2500}\u{2500} Hand ").skip(1).enumerate() {
            let config = HandConfig {
                rules: RulesConfig::standard(StackDepth::Bb20),
                button: if i % 2 == 0 { HUMAN } else { BOT },
            };
            let h = HandState::new(config, hand_deck(seed, i as u64)).unwrap();
            for c in h.hole_cards(HUMAN) {
                assert!(section.contains(&ui.card_text(c)));
            }
            let pre = match section.find(SHOWDOWN_MARKER) {
                Some(p) => {
                    showdowns += 1;
                    &section[..p]
                }
                None => section,
            };
            for c in h.hole_cards(BOT) {
                assert!(
                    !pre.contains(&ui.card_text(c)),
                    "seed {seed} hand {i}: leaked {c}"
                );
            }
        }
    }
    assert!(showdowns > 0);
}

#[test]
fn ui_commands_listed_in_help_work() {
    let o = pokerbot(
        &base(&[]),
        // Fold at once (the button faces the big blind), then exercise the
        // between-hands commands.
        "help\nstats\nf\nhelp\nstats\nbogus\n\nquit\n",
    );
    let out = stdout(&o);
    assert!(o.status.success());
    assert!(out.contains("Commands:"));
    for cmd in ["help", "stats", "quit"] {
        assert!(out.contains(&format!("    {cmd} ")), "help lists {cmd}");
    }
    assert!(
        out.contains("Commands between hands:"),
        "between-hands help"
    );
    assert!(out.contains("continue"));
    assert!(
        out.matches("Session: ").count() >= 3,
        "stats in hand, after hand, between"
    );
    assert!(out.contains("Invalid input: \"bogus\""));
    assert!(out.contains("You quit the session."));
    assert!(out.contains("Opponent modes (--bot)"));
}

#[test]
fn ui_replay_hint_includes_bot_auto_strategy() {
    let out = stdout(&pokerbot(
        &base(&["--auto", "checkcall", "--width", "200"]),
        "",
    ));
    assert!(
        out.contains(
            "Replay: pokerbot --seed 5 --hands 4 --depth 20 --bot baseline --auto checkcall"
        ),
        "{out}"
    );
    let out = stdout(&pokerbot(
        &[
            "--seed",
            "5",
            "--hands",
            "1",
            "--strategy",
            "/nowhere/x.pokerbot",
            "--width",
            "200",
            "--auto",
            "random",
        ],
        "",
    ));
    assert!(
        out.contains("Replay: pokerbot --seed 5 --hands 1 --depth 20 --bot auto --strategy /nowhere/x.pokerbot --difficulty challenging --auto random"),
        "{out}"
    );
}

#[test]
fn ui_repeated_flags_are_rejected() {
    for args in [
        &["--seed", "1", "--seed", "2"][..],
        &["--no-color", "--color", "256"],
        &["--width", "80", "--width=90"],
        &["--bot", "cfr", "--bot", "baseline"],
    ] {
        let o = pokerbot(args, "");
        assert_eq!(o.status.code(), Some(2), "{args:?}");
        assert!(stderr(&o).contains("more than once"), "{}", stderr(&o));
        assert!(o.stdout.is_empty());
    }
}

#[test]
fn ui_narrowest_width_fits_for_quit_and_eof_paths() {
    for script in ["quit\n", "", "x\nhelp\nstats\nf\nhelp\nbogus\n"] {
        let o = pokerbot(&base(&["--width", "48"]), script);
        assert!(o.status.success());
        assert_fits(&stdout(&o), 48);
    }
}

#[test]
fn ui_fast_and_non_tty_never_pause() {
    // 40 hands of piped/auto play finish quickly: no pacing off a TTY.
    let start = std::time::Instant::now();
    let o = pokerbot(
        &[
            "--bot", "baseline", "--seed", "1", "--hands", "40", "--auto", "random", "--fast",
        ],
        "",
    );
    assert!(o.status.success());
    let o = pokerbot(
        &[
            "--bot", "baseline", "--seed", "1", "--hands", "40", "--auto", "random",
        ],
        "",
    );
    assert!(o.status.success());
    assert!(start.elapsed().as_secs() < 20, "took {:?}", start.elapsed());
}
