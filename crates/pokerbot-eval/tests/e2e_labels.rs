//! The evaluation report labels the policy as plumbing-only / not for
//! competition and disclaims any GTO, Nash or exploitability claim; a
//! non-pinned artifact never promotes. Uses the real
//! `artifacts/smoke_20bb.pokerbot` when it was trained locally.

mod common;

use std::process::Command;

fn eval(args: &[&str], artifact: Option<&std::path::Path>) -> (i32, String) {
    let mut c = Command::new(env!("CARGO_BIN_EXE_pokerbot-eval"));
    c.current_dir(common::workspace_root()).args(args);
    if let Some(a) = artifact {
        c.arg("--artifact").arg(a);
    }
    let o = c.output().unwrap();
    let out = String::from_utf8_lossy(&o.stdout).into_owned();
    (o.status.code().unwrap_or(-1), out)
}

fn assert_disclaimed(out: &str) {
    assert!(out.contains("DISCLAIMER:"), "{out}");
    assert!(
        out.contains("No exploitability, best-response, Nash-distance or GTO measurement"),
        "{out}"
    );
}

#[test]
fn e2e_labels_eval_tiny_artifact_disclaimed_and_held() {
    let dir = common::scratch("e2e-labels");
    let art = common::write_tiny(&dir);
    let (code, out) = eval(
        &["--smoke", "--pairs", "5", "--require-promote"],
        Some(&art),
    );
    assert_eq!(code, 3, "an unpinned artifact must HOLD:\n{out}");
    assert!(out.contains("promotion gate: HOLD"));
    assert_disclaimed(&out);
}

#[test]
fn e2e_labels_eval_real_smoke_artifact() {
    let p = common::workspace_root().join("artifacts/smoke_20bb.pokerbot");
    if !p.exists() {
        eprintln!(
            "SKIPPED (loud): artifacts/smoke_20bb.pokerbot absent; run scripts/train_smoke.sh"
        );
        return;
    }
    let (code, out) = eval(&["--smoke", "--pairs", "10"], None);
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("label      smoke: trained for plumbing, not competition"),
        "{out}"
    );
    assert!(out.contains("matches pinned value"), "{out}");
    // 20 hands per matchup is far below the locked minimum: HOLD.
    assert!(out.contains("promotion gate: HOLD"), "{out}");
    assert_disclaimed(&out);
}
