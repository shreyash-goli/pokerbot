//! Evaluation config parsing.

mod common;

use pokerbot_eval::config::{EvalConfig, parse_checksum};

const GOOD: &str = r#"
[policy]
artifact = "a.pokerbot"
[rules]
depth_bb = 20
[eval]
base_seed = 5
deal_pairs = 10
opponents = ["random", "checkcall"]
[gate]
min_hands = 20
require_positive_ci_vs = ["random"]
max_fallback_rate_bp = 100
"#;

#[test]
fn smoke_config_parses_and_is_locked_sensibly() {
    let text = std::fs::read_to_string(common::workspace_root().join("configs/eval_smoke.toml"))
        .expect("configs/eval_smoke.toml");
    let c = EvalConfig::parse(&text).unwrap();
    assert_eq!(c.depth_bb, 20);
    assert_eq!(c.rules.starting_stack, 40);
    assert_eq!(c.artifact, "artifacts/smoke_20bb.pokerbot");
    assert!(c.expected_checksum.is_some());
    // Disjoint from the smoke training seed and abstraction seed.
    assert_ne!(c.base_seed, 20_261_115);
    assert_ne!(c.base_seed, 1_296_126_799);
    for o in ["random", "checkcall", "mirror"] {
        assert!(c.opponents.iter().any(|x| x == o));
    }
    assert!(
        c.gate
            .as_ref()
            .unwrap()
            .require_positive_ci_vs
            .contains(&"random".to_string())
    );
    assert!(
        c.gate
            .as_ref()
            .unwrap()
            .require_positive_ci_vs
            .contains(&"checkcall".to_string())
    );
    assert!(c.gate.as_ref().unwrap().min_hands <= 2 * c.deal_pairs);
}

#[test]
fn minimal_config_defaults() {
    let c = EvalConfig::parse(GOOD).unwrap();
    assert_eq!(c.rules.big_blind, 2);
    assert_eq!(c.expected_checksum, None);
    assert!(c.sanity_random_vs_checkcall);
    assert!(c.gate.as_ref().unwrap().require_pinned_checksum);
    assert!((c.gate.as_ref().unwrap().max_fallback_rate - 0.01).abs() < 1e-12);
}

#[test]
fn strictness() {
    let bad = [
        GOOD.replace("deal_pairs = 10", "deal_pairs = 10\ndeal_pairs = 11"),
        GOOD.replace("[gate]", "[gate]\nbogus = 1"),
        GOOD.replace("[rules]", "[ruleset]"),
        GOOD.replace("\"checkcall\"]", "\"nash-bot\"]"),
        GOOD.replace("\"checkcall\"]", "\"random\"]"),
        GOOD.replace(
            "require_positive_ci_vs = [\"random\"]",
            "require_positive_ci_vs = [\"tight\"]",
        ),
        GOOD.replace("deal_pairs = 10", "deal_pairs = 0"),
        GOOD.replace("deal_pairs = 10", "deal_pairs = \"10\""),
        GOOD.replace("max_fallback_rate_bp = 100", "max_fallback_rate_bp = 10001"),
        GOOD.replace("base_seed = 5\n", ""),
        GOOD.replace("opponents = [\"random\", \"checkcall\"]", "opponents = []"),
        GOOD.replace(
            "artifact = \"a.pokerbot\"",
            "artifact = \"a.pokerbot\" extra",
        ),
    ];
    for (i, t) in bad.iter().enumerate() {
        assert!(EvalConfig::parse(t).is_err(), "case {i} should fail:\n{t}");
    }
    let mirror = GOOD
        .replace("[\"random\", \"checkcall\"]", "[\"random\", \"mirror\"]")
        .replace("ci_vs = [\"random\"]", "ci_vs = [\"mirror\"]");
    assert!(
        EvalConfig::parse(&mirror).is_err(),
        "mirror cannot be a gate requirement"
    );
}

#[test]
fn checksum_parsing() {
    assert_eq!(
        parse_checksum("0xd2767c7dad569fdd"),
        Some(0xd276_7c7d_ad56_9fdd)
    );
    assert_eq!(parse_checksum("17"), Some(17));
    assert_eq!(parse_checksum("0x"), None);
    assert_eq!(parse_checksum("0x1ffffffffffffffff"), None);
}
