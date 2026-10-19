//! End-to-end: the `mako-eval` binary and library orchestration with a tiny
//! in-process-trained artifact.

mod common;

use std::process::Command;

use mako_core::rules::{RulesConfig, StackDepth};
use mako_eval::config::EvalConfig;
use mako_eval::evaluate::{from_bytes, run_evaluation};
use mako_eval::gate::Verdict;
use mako_eval::output::render_json;

fn bin() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_mako-eval"));
    c.current_dir(common::workspace_root());
    c
}

fn tiny_config(artifact: &std::path::Path, base_seed: u64) -> String {
    format!(
        r#"
[policy]
artifact = "{}"
[rules]
depth_bb = 20
[eval]
base_seed = {base_seed}
deal_pairs = 30
opponents = ["random", "checkcall", "tight", "mirror"]
bootstrap_resamples = 100
bootstrap_seed = 3
[gate]
min_hands = 60
require_positive_ci_vs = ["random", "checkcall"]
max_fallback_rate_bp = 500
"#,
        artifact.display()
    )
}

#[test]
fn smoke_flag_runs_with_artifact_override() {
    let dir = common::scratch("smoke");
    let art = common::write_tiny(&dir);
    let json = dir.join("report.json");
    let out = bin()
        .args(["--smoke", "--pairs", "20", "--artifact"])
        .arg(&art)
        .arg("--json")
        .arg(&json)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(stdout.contains("BB/100 (A, paired)"));
    assert!(stdout.contains("A on button"));
    assert!(stdout.contains("A on big blind"));
    assert!(stdout.contains("fallback"));
    // Tiny artifact: checksum differs from the pinned smoke artifact and the
    // sample is below the locked minimum, so the gate must hold.
    assert!(stdout.contains("promotion gate: HOLD"));
    assert!(stdout.contains("does not match pinned"));
    assert!(stdout.contains("No exploitability"));
    let j = std::fs::read_to_string(&json).unwrap();
    assert!(j.starts_with("{\"format\":\"mako-eval-report v1\""));
    assert!(j.contains("\"verdict\":\"HOLD\""));
    assert!(!j.to_lowercase().contains("gto:"));

    let held = bin()
        .args(["--smoke", "--pairs", "5", "--require-promote", "--artifact"])
        .arg(&art)
        .output()
        .unwrap();
    assert_eq!(held.status.code(), Some(3));
}

#[test]
fn missing_and_invalid_artifacts_are_explicit_errors() {
    let dir = common::scratch("missing");
    let out = bin()
        .args(["--smoke", "--artifact"])
        .arg(dir.join("nope.mako"))
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("cannot read artifact"), "{err}");
    assert!(out.stdout.is_empty(), "no report without an artifact");

    let junk = dir.join("junk.mako");
    std::fs::write(&junk, b"MAKOSTRT not really an artifact").unwrap();
    let out = bin()
        .args(["--smoke", "--artifact"])
        .arg(&junk)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("rejected"));

    for args in [
        vec!["--pairs", "5"],
        vec!["--smoke", "--pairs", "0"],
        vec!["--smoke", "--pairs", "999999999"],
        vec!["--smoke", "--bogus"],
    ] {
        let out = bin().args(&args).output().unwrap();
        assert_eq!(out.status.code(), Some(2), "{args:?}");
    }
}

#[test]
fn same_config_gives_identical_report() {
    let dir = common::scratch("determinism");
    let art = common::write_tiny(&dir);
    let cfg_path = dir.join("eval.toml");
    std::fs::write(&cfg_path, tiny_config(&art, 424_242)).unwrap();
    let run = |name: &str| {
        let j = dir.join(name);
        let out = bin()
            .arg("--config")
            .arg(&cfg_path)
            .arg("--json")
            .arg(&j)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        std::fs::read_to_string(j).unwrap()
    };
    let a = run("a.json");
    let b = run("b.json");
    assert_eq!(a, b);

    // Library path gives the same JSON.
    let cfg = EvalConfig::parse(&tiny_config(&art, 424_242)).unwrap();
    let loaded = from_bytes(
        art.display().to_string(),
        common::tiny_artifact().to_vec(),
        &cfg.rules,
    )
    .unwrap();
    let r = run_evaluation(&cfg, &loaded, None).unwrap();
    assert_eq!(render_json(&r), a);
    // Mirror is exactly zero; every matchup is zero-sum.
    let mirror = r
        .matchups
        .iter()
        .find(|m| m.opponent_id == "mirror")
        .unwrap();
    assert_eq!(mirror.a_net_chips, 0);
    for m in r.matchups.iter().chain(&r.sanity) {
        assert_eq!(m.a_net_chips + m.b_net_chips, 0);
        assert_eq!(m.hands, 60);
    }
    // Candidate harness counts agree with the agents' own InferStats.
    assert_eq!(r.candidate_counts.decisions, r.candidate_infer.decisions);
    assert_eq!(r.candidate_counts.fallbacks, r.candidate_infer.fallbacks);
    assert_eq!(
        r.gate.as_ref().unwrap().verdict,
        Verdict::Hold,
        "unpinned tiny artifact never promotes"
    );
}

#[test]
fn eval_seed_equal_to_training_seed_is_refused() {
    let dir = common::scratch("overlap");
    let art = common::write_tiny(&dir);
    let cfg = EvalConfig::parse(&tiny_config(&art, common::TINY_TRAIN_SEED)).unwrap();
    let loaded = from_bytes("tiny".into(), common::tiny_artifact().to_vec(), &cfg.rules).unwrap();
    let e = run_evaluation(&cfg, &loaded, None).unwrap_err().to_string();
    assert!(e.contains("training"), "{e}");
}

#[test]
fn wrong_depth_artifact_is_rejected() {
    let r50 = RulesConfig::standard(StackDepth::Bb50);
    assert!(from_bytes("tiny".into(), common::tiny_artifact().to_vec(), &r50).is_err());
}
