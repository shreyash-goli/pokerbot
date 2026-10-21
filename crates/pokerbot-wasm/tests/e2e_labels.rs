//! M-24: the browser bridge's snapshot and strategyInfo identify a CFR
//! policy as weak and not GTO/Nash (tiny fixture and, if trained locally,
//! the real `artifacts/smoke_20bb.pokerbot`).

mod common;

use std::path::Path;

use common::policy_20bb;
use pokerbot_wasm::GameSession;

fn check(bytes: Vec<u8>) {
    let s = GameSession::new(20, 7, Some(bytes), "cfr").unwrap();
    for json in [s.strategy_info_json(), s.snapshot_json()] {
        assert!(json.contains("\"notGto\":true"), "{json}");
        assert!(json.contains("smoke-trained policy is weak"), "{json}");
        assert!(json.contains("it is NOT GTO or Nash"), "{json}");
    }
}

#[test]
fn e2e_labels_wasm_strategy_info_marks_policy_weak() {
    check(policy_20bb().1);
}

#[test]
fn e2e_labels_wasm_real_smoke_artifact() {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../artifacts/smoke_20bb.pokerbot");
    if !p.exists() {
        eprintln!(
            "SKIPPED (loud): artifacts/smoke_20bb.pokerbot absent; run scripts/train_smoke.sh"
        );
        return;
    }
    let bytes = std::fs::read(&p).unwrap();
    check(bytes.clone());
    let s = GameSession::new(20, 7, Some(bytes), "cfr").unwrap();
    assert!(
        s.strategy_info_json()
            .contains("smoke: trained for plumbing, not competition")
    );
}
