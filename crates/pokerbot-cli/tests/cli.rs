//! End-to-end tests of the `pokerbot` binary against the untrained baseline
//! (M-05). CFR-policy sessions are tested in `tests/cfr.rs`.

use std::io::Write;
use std::process::{Command, Output, Stdio};

use pokerbot_cli::game::{BOT, HUMAN, hand_deck};
use pokerbot_cli::ui::SHOWDOWN_MARKER;
use pokerbot_core::card::Card;
use pokerbot_core::rules::{RulesConfig, StackDepth};
use pokerbot_core::state::{HandConfig, HandState};

/// A fresh per-invocation directory used as the child's `POKERBOT_HOME` and
/// `HOME`, so a test that passes no history flag can never touch the real
/// home directory or `~/.pokerbot`. Set on the child `Command` only.
fn sandbox_home() -> std::path::PathBuf {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let d = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "pokerbot-home-{}-{}-{}",
        module_path!().replace("::", "-"),
        std::process::id(),
        N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&d).expect("sandbox home");
    d
}

/// Runs the binary. Unless the test picks a bot, `--bot baseline` is added
/// so these tests never depend on a local trained artifact.
fn pokerbot(args: &[&str], stdin: &str) -> Output {
    let mut argv: Vec<&str> = args.to_vec();
    if !args.iter().any(|a| a.starts_with("--bot")) && !args.contains(&"--help") {
        argv.extend(["--bot", "baseline"]);
    }
    // No test here chooses a history location: never write one.
    if !args
        .iter()
        .any(|a| matches!(*a, "--help" | "-h" | "--version" | "-V"))
    {
        argv.push("--no-history");
    }
    let mut child = Command::new(env!("CARGO_BIN_EXE_pokerbot"))
        .args(&argv)
        // A fixed environment: 80 columns, ASCII glyphs, no colour hints.
        .env_remove("NO_COLOR")
        .env_remove("COLUMNS")
        .env_remove("COLORTERM")
        .env("LC_ALL", "C")
        .env("TERM", "xterm")
        .env("POKERBOT_HOME", sandbox_home())
        .env("HOME", sandbox_home())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn pokerbot");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    // stdin is dropped here, so the child sees EOF after the script.
    child.wait_with_output().expect("wait for pokerbot")
}

fn stdout(o: &Output) -> String {
    String::from_utf8(o.stdout.clone()).expect("utf-8 output")
}

/// The output section of each hand, split on the hand header.
fn hand_sections(out: &str) -> Vec<&str> {
    out.split("\n-- Hand ").skip(1).collect()
}

/// Hole cards of (human, bot) for hand `i` of a session, recomputed with
/// the engine from the same seed (independently of the binary).
fn hole_cards(seed: u64, i: u64) -> ([String; 2], [String; 2]) {
    // The human holds the button on even-indexed hands (it alternates).
    let config = HandConfig {
        rules: RulesConfig::standard(StackDepth::Bb20),
        button: if i % 2 == 0 { HUMAN } else { BOT },
    };
    let h = HandState::new(config, hand_deck(seed, i)).unwrap();
    let fmt = |cs: [Card; 2]| cs.map(|c| format!("[{c}]"));
    (fmt(h.hole_cards(HUMAN)), fmt(h.hole_cards(BOT)))
}

/// Asserts the bot's hole cards never appear in a hand's output before a
/// showdown marker (or at all, if the hand had no showdown), and that the
/// human's cards do appear (sanity check of the recomputed deal).
fn assert_no_leak(out: &str, seed: u64) -> usize {
    let mut showdowns = 0;
    for (i, section) in hand_sections(out).iter().enumerate() {
        let (mine, bots) = hole_cards(seed, i as u64);
        for c in &mine {
            assert!(
                section.contains(c.as_str()),
                "hand {i}: my card {c} missing"
            );
        }
        let pre = match section.find(SHOWDOWN_MARKER) {
            Some(pos) => {
                showdowns += 1;
                let post = &section[pos..];
                for c in &bots {
                    assert!(post.contains(c.as_str()), "hand {i}: showdown hides {c}");
                }
                &section[..pos]
            }
            None => section,
        };
        for c in &bots {
            assert!(
                !pre.contains(c.as_str()),
                "hand {i}: bot card {c} leaked before showdown:\n{pre}"
            );
        }
    }
    showdowns
}

fn final_score(out: &str) -> (i64, i64) {
    let line = out
        .lines()
        .find(|l| l.starts_with("Final:"))
        .expect("final line");
    let nums: Vec<i64> = line
        .split_whitespace()
        .filter_map(|w| w.trim_start_matches('+').parse().ok())
        .collect();
    // "Final: N hand(s) played. You X / Bot Y chips ..."
    (nums[1], nums[2])
}

#[test]
fn auto_checkcall_session_completes() {
    let o = pokerbot(
        &[
            "--seed",
            "7",
            "--hands",
            "3",
            "--bot",
            "baseline",
            "--no-color",
            "--auto",
            "checkcall",
        ],
        "",
    );
    let out = stdout(&o);
    assert!(o.status.success(), "status {:?}\n{out}", o.status);
    assert!(out.contains("baseline (untrained, random legal)"));
    assert!(out.contains("Session seed 7"));
    assert!(out.contains("Session complete."));
    assert!(out.contains("Final: 3 hand(s) played."));
    assert_eq!(hand_sections(&out).len(), 3);
    assert!(!out.contains('\x1b'), "--no-color output has escapes");
    assert_no_leak(&out, 7);
    let (you, bot) = final_score(&out);
    assert_eq!(you + bot, 0, "session score is zero-sum");
}

#[test]
fn many_seeds_never_leak_and_conserve_chips() {
    let mut showdowns = 0;
    for seed in 0..25u64 {
        let s = seed.to_string();
        let policy = if seed % 2 == 0 { "random" } else { "checkcall" };
        let o = pokerbot(
            &["--seed", &s, "--hands", "6", "--no-color", "--auto", policy],
            "",
        );
        let out = stdout(&o);
        assert!(o.status.success());
        showdowns += assert_no_leak(&out, seed);
        let (you, bot) = final_score(&out);
        assert_eq!(you + bot, 0);
    }
    assert!(showdowns > 0, "test should exercise legitimate reveals");
}

#[test]
fn deterministic_transcripts() {
    let args = [
        "--seed",
        "7",
        "--hands",
        "3",
        "--no-color",
        "--auto",
        "random",
    ];
    assert_eq!(stdout(&pokerbot(&args, "")), stdout(&pokerbot(&args, "")));
    let script = "x\nc\nc\nc\nc\n\nc\nc\nc\nc\n\nc\nc\nc\nc\n";
    let piped = ["--seed", "7", "--hands", "3", "--no-color"];
    assert_eq!(
        stdout(&pokerbot(&piped, script)),
        stdout(&pokerbot(&piped, script))
    );
}

#[test]
fn piped_script_with_garbage_help_and_quit() {
    let o = pokerbot(
        &[
            "--seed",
            "7",
            "--hands",
            "3",
            "--bot",
            "baseline",
            "--no-color",
        ],
        "x\nhelp\nstats\n???\nc\nc\nc\nc\nquit\n",
    );
    let out = stdout(&o);
    assert!(o.status.success(), "{out}");
    assert!(out.contains("Invalid input"));
    assert!(out.contains("Commands:"), "help text shown");
    assert!(out.contains("Session: 0 hand(s) completed"), "stats shown");
    assert!(out.contains("You quit the session."), "{out}");
    assert!(out.contains("untrained"));
    assert_no_leak(&out, 7);
    let (you, bot) = final_score(&out);
    assert_eq!(you + bot, 0);
}

#[test]
fn quit_on_first_prompt_abandons_hand() {
    let o = pokerbot(&["--seed", "7", "--hands", "3", "--no-color"], "quit\n");
    let out = stdout(&o);
    assert!(o.status.success());
    assert!(out.contains("abandoned"));
    assert!(out.contains("Final: 0 hand(s) played. You 0 / Bot 0"));
    assert_no_leak(&out, 7);
}

#[test]
fn eof_ends_gracefully() {
    for script in ["", "c\n", "garbage\nmore garbage\n"] {
        let o = pokerbot(&["--seed", "7", "--hands", "3", "--no-color"], script);
        let out = stdout(&o);
        assert!(o.status.success(), "script {script:?}: {:?}", o.status);
        assert!(out.contains("Input closed; ending the session."), "{out}");
        assert!(String::from_utf8_lossy(&o.stderr).is_empty());
    }
}

#[test]
fn cfr_bot_with_missing_artifact_is_refused_explicitly() {
    let o = pokerbot(
        &[
            "--bot",
            "cfr",
            "--strategy",
            "/nonexistent/pokerbot/none.pokerbot",
            "--seed",
            "1",
            "--auto",
            "checkcall",
        ],
        "",
    );
    assert_eq!(o.status.code(), Some(2));
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(err.contains("no trained policy for 20BB"), "{err}");
    assert!(err.contains("missing"), "{err}");
    assert!(o.stdout.is_empty(), "no game played with a substitute bot");
}

#[test]
fn bad_flags_exit_2_and_help_exits_0() {
    for args in [
        &["--depth", "30"][..],
        &["--hands", "0"],
        &["--wat"],
        &["--bot", "x"],
    ] {
        let o = pokerbot(args, "");
        assert_eq!(o.status.code(), Some(2), "{args:?}");
        assert!(o.stdout.is_empty());
    }
    let o = pokerbot(&["--help"], "");
    assert!(o.status.success());
    assert!(stdout(&o).contains("--seed"));
}

#[test]
fn deeper_stacks_and_default_seed() {
    let o = pokerbot(
        &[
            "--depth",
            "100",
            "--hands",
            "2",
            "--no-color",
            "--auto",
            "random",
        ],
        "",
    );
    let out = stdout(&o);
    assert!(o.status.success());
    assert!(out.contains("100 BB deep (200 chips)"));
    assert!(
        out.contains("Replay: pokerbot --seed "),
        "clock seed is printed"
    );
}
