//! Helpers shared by the CFR integration tests.
#![allow(dead_code)]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::OnceLock;
use std::sync::atomic::AtomicBool;

use mako_cfr::artifact::{ArtifactMetadata, StrategyArtifact};
use mako_cfr::train::{TrainConfig, train};
use mako_cli::game::{BOT, HUMAN, hand_deck};
use mako_cli::ui::SHOWDOWN_MARKER;
use mako_core::card::Card;
use mako_core::rules::{RulesConfig, StackDepth};
use mako_core::state::{HandConfig, HandState};

pub const TINY_LABEL: &str = "test: tiny cli fixture, no strength claim";

/// A fresh per-invocation directory used as the child's `MAKO_HOME` and
/// `HOME`, so a test that passes no history flag can never touch the real
/// home directory or `~/.mako`. Set on the child `Command` only.
fn sandbox_home() -> std::path::PathBuf {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let d = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "mako-home-{}-{}-{}",
        module_path!().replace("::", "-"),
        std::process::id(),
        N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&d).expect("sandbox home");
    d
}

/// Runs the binary with `args`, feeding `stdin` then EOF.
pub fn mako(args: &[&str], stdin: &str) -> Output {
    mako_env(args, stdin, &[])
}

/// [`mako`] with extra environment variables set after the fixed ones.
pub fn mako_env(args: &[&str], stdin: &str, env: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_mako"));
    // Tests that do not choose a history location get `--no-history`, so
    // transcripts stay deterministic and nothing is written anywhere.
    let picks_history = args
        .iter()
        .any(|a| a.starts_with("--history") || *a == "--no-history" || *a == "--reset-stats")
        || env.iter().any(|(k, _)| *k == "MAKO_HOME");
    cmd.args(args);
    if !picks_history
        && !args
            .iter()
            .any(|a| matches!(*a, "--help" | "-h" | "--version" | "-V"))
    {
        cmd.arg("--no-history");
    }
    cmd
        // A fixed environment: 80 columns, ASCII glyphs, no colour hints.
        .env_remove("NO_COLOR")
        .env_remove("COLUMNS")
        .env_remove("COLORTERM")
        .env("LC_ALL", "C")
        .env("TERM", "xterm");
    let home = sandbox_home();
    cmd.env("MAKO_HOME", &home).env("HOME", &home);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn mako");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    child.wait_with_output().expect("wait for mako")
}

pub fn stdout(o: &Output) -> String {
    String::from_utf8(o.stdout.clone()).expect("utf-8 output")
}

pub fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn tmp(name: &str) -> PathBuf {
    Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("mako-cli-{}-{name}", std::process::id()))
}

/// Bytes of a tiny 20BB artifact trained in-process (once per test binary).
pub fn tiny_artifact_bytes() -> &'static [u8] {
    static BYTES: OnceLock<Vec<u8>> = OnceLock::new();
    BYTES.get_or_init(|| {
        let cfg = TrainConfig::tiny(31, 400);
        let out = train(&cfg, None, &AtomicBool::new(false), &mut |_| {}).unwrap();
        let meta = ArtifactMetadata::from_trainer(&out.trainer, TINY_LABEL);
        StrategyArtifact::from_table(out.trainer.table(), meta)
            .unwrap()
            .to_bytes()
    })
}

/// Path of the tiny artifact written to the cargo test temp dir.
pub fn tiny_artifact_path() -> &'static Path {
    static PATH: OnceLock<PathBuf> = OnceLock::new();
    PATH.get_or_init(|| {
        let p = tmp("tiny20.mako");
        std::fs::write(&p, tiny_artifact_bytes()).unwrap();
        p
    })
}

/// Writes arbitrary bytes to a temp file and returns its path.
pub fn write_tmp(name: &str, bytes: &[u8]) -> PathBuf {
    let p = tmp(name);
    std::fs::write(&p, bytes).unwrap();
    p
}

/// The output section of each hand, split on the hand header.
pub fn hand_sections(out: &str) -> Vec<&str> {
    out.split("\n-- Hand ").skip(1).collect()
}

/// Hole cards of (human, bot) for hand `i` at 20BB, recomputed with the
/// engine from the session seed.
pub fn hole_cards(seed: u64, i: u64) -> ([String; 2], [String; 2]) {
    let config = HandConfig {
        rules: RulesConfig::standard(StackDepth::Bb20),
        button: if i % 2 == 0 { HUMAN } else { BOT },
    };
    let h = HandState::new(config, hand_deck(seed, i)).unwrap();
    let fmt = |cs: [Card; 2]| cs.map(|c| format!("[{c}]"));
    (fmt(h.hole_cards(HUMAN)), fmt(h.hole_cards(BOT)))
}

/// Bot hole cards never appear before a hand's showdown marker; returns the
/// number of showdowns.
pub fn assert_no_leak(out: &str, seed: u64) -> usize {
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
                for c in &bots {
                    assert!(
                        section[pos..].contains(c.as_str()),
                        "hand {i}: showdown hides {c}"
                    );
                }
                &section[..pos]
            }
            None => section,
        };
        for c in &bots {
            assert!(
                !pre.contains(c.as_str()),
                "hand {i}: bot card {c} leaked:\n{pre}"
            );
        }
    }
    showdowns
}

/// `(you, bot)` from the `Final:` line.
pub fn final_score(out: &str) -> (i64, i64) {
    let line = out
        .lines()
        .find(|l| l.starts_with("Final:"))
        .expect("final line");
    let nums: Vec<i64> = line
        .split_whitespace()
        .filter_map(|w| w.trim_start_matches('+').parse().ok())
        .collect();
    (nums[1], nums[2])
}
