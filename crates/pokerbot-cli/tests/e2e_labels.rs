//! M-24: the terminal game identifies a CFR policy as a weak, plumbing-only,
//! not-GTO smoke policy, for the tiny fixture and (if trained locally) the
//! real `artifacts/smoke_20bb.pokerbot`. Children run sandboxed (tests/common).

mod common;

use std::path::Path;

use common::*;

const WEAK_NOTE: &str = "NOTE: plumbing-only smoke policy -- not competitive, not GTO / Nash.";

fn banner(strategy: &str) -> String {
    let o = pokerbot(
        &[
            "--bot",
            "cfr",
            "--strategy",
            strategy,
            "--seed",
            "1",
            "--hands",
            "1",
            "--no-color",
            "--fast",
            "--auto",
            "checkcall",
        ],
        "",
    );
    let out = stdout(&o);
    assert!(o.status.success(), "{}\n{out}", stderr(&o));
    out
}

#[test]
fn e2e_labels_cli_banner_marks_policy_weak() {
    let out = banner(tiny_artifact_path().to_str().unwrap());
    assert!(out.contains(WEAK_NOTE), "{out}");
    assert!(out.contains(TINY_LABEL), "{out}");
}

#[test]
fn e2e_labels_cli_banner_real_smoke_artifact() {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../artifacts/smoke_20bb.pokerbot");
    if !p.exists() {
        eprintln!(
            "SKIPPED (loud): artifacts/smoke_20bb.pokerbot is not present; run \
             scripts/train_smoke.sh to cover the real smoke policy's banner"
        );
        return;
    }
    let out = banner(p.to_str().unwrap());
    assert!(out.contains(WEAK_NOTE), "{out}");
    assert!(
        out.contains("smoke: trained for plumbing, not competition"),
        "{out}"
    );
    assert!(out.contains("trained 500000 iterations"), "{out}");
}
