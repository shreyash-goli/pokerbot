//! Promotion gate decisions.

use mako_core::rules::{RulesConfig, StackDepth};
use mako_eval::baselines::{AlwaysFold, AlwaysShove, RandomLegal};
use mako_eval::gate::{ChecksumStatus, GateConfig, GateInput, Verdict, decide};
use mako_eval::matchup::{DecisionCounts, MatchSpec, run_match};
use mako_eval::report::{MatchupReport, summarize_match};
use mako_eval::stats::Summary;

fn cfg(min_hands: u64) -> GateConfig {
    GateConfig {
        min_hands,
        require_positive_ci_vs: vec!["random".into(), "checkcall".into()],
        max_fallback_rate: 0.05,
        require_pinned_checksum: true,
    }
}

fn shove_vs_fold(id: &str, pairs: u64) -> MatchupReport {
    let spec = MatchSpec {
        rules: RulesConfig::standard(StackDepth::Bb20),
        base_seed: 3,
        pairs,
        max_actions: 512,
    };
    let m = run_match(&spec, &mut AlwaysShove, &mut AlwaysFold).unwrap();
    summarize_match(id, &m, None)
}

fn ok_counts() -> DecisionCounts {
    DecisionCounts {
        decisions: 1000,
        fallbacks: 0,
    }
}

#[test]
fn clear_win_promotes() {
    let ms = vec![
        shove_vs_fold("random", 100),
        shove_vs_fold("checkcall", 100),
    ];
    // Shove wins the BB's 2 chips on the button and the SB's 1 chip in the BB.
    assert!(ms[0].pairs == 100 && ms[0].a_net_chips == 300);
    assert!((ms[0].bb100.mean - 75.0).abs() < 1e-9);
    let d = decide(
        &cfg(200),
        &GateInput {
            matchups: &ms,
            candidate: ok_counts(),
            checksum: ChecksumStatus::Matches(7),
        },
    );
    assert_eq!(d.verdict, Verdict::Promote, "{:?}", d.failures());
}

#[test]
fn too_few_hands_holds() {
    let ms = vec![shove_vs_fold("random", 50), shove_vs_fold("checkcall", 50)];
    let d = decide(
        &cfg(200),
        &GateInput {
            matchups: &ms,
            candidate: ok_counts(),
            checksum: ChecksumStatus::Matches(7),
        },
    );
    assert_eq!(d.verdict, Verdict::Hold);
    assert!(d.failures().iter().all(|c| c.name.starts_with("hands")));
}

#[test]
fn ci_straddling_zero_holds() {
    let mut straddle = shove_vs_fold("checkcall", 200);
    straddle.bb100 = Summary {
        n: 200,
        mean: 3.0,
        sd: 100.0,
        se: 7.0,
        ci_low: -10.7,
        ci_high: 16.7,
    };
    let ms = vec![shove_vs_fold("random", 200), straddle];
    let d = decide(
        &cfg(200),
        &GateInput {
            matchups: &ms,
            candidate: ok_counts(),
            checksum: ChecksumStatus::Matches(7),
        },
    );
    assert_eq!(d.verdict, Verdict::Hold);
    let f = d.failures();
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].name, "CI vs checkcall");

    // A real mirror-like (zero) result also holds.
    let spec = MatchSpec {
        rules: RulesConfig::standard(StackDepth::Bb20),
        base_seed: 3,
        pairs: 200,
        max_actions: 512,
    };
    let m = run_match(&spec, &mut RandomLegal::new(1), &mut RandomLegal::new(2)).unwrap();
    let zero = summarize_match("checkcall", &m, None);
    let ms = vec![shove_vs_fold("random", 200), zero];
    let d = decide(
        &cfg(200),
        &GateInput {
            matchups: &ms,
            candidate: ok_counts(),
            checksum: ChecksumStatus::Matches(7),
        },
    );
    assert_eq!(d.verdict, Verdict::Hold);
}

#[test]
fn missing_matchup_fallbacks_and_checksum_hold() {
    let base = |ms: &[MatchupReport], c: DecisionCounts, s: ChecksumStatus| {
        decide(
            &cfg(200),
            &GateInput {
                matchups: ms,
                candidate: c,
                checksum: s,
            },
        )
        .verdict
    };
    let both = vec![
        shove_vs_fold("random", 100),
        shove_vs_fold("checkcall", 100),
    ];
    let good = ChecksumStatus::Matches(1);
    assert_eq!(base(&both[..1], ok_counts(), good), Verdict::Hold);
    let many_fallbacks = DecisionCounts {
        decisions: 1000,
        fallbacks: 51,
    };
    assert_eq!(base(&both, many_fallbacks, good), Verdict::Hold);
    assert_eq!(base(&both, DecisionCounts::default(), good), Verdict::Hold);
    assert_eq!(
        base(
            &both,
            ok_counts(),
            ChecksumStatus::Mismatch {
                expected: 1,
                found: 2
            }
        ),
        Verdict::Hold
    );
    assert_eq!(
        base(&both, ok_counts(), ChecksumStatus::NotPinned(2)),
        Verdict::Hold
    );
    let mut lax = cfg(200);
    lax.require_pinned_checksum = false;
    let d = decide(
        &lax,
        &GateInput {
            matchups: &both,
            candidate: ok_counts(),
            checksum: ChecksumStatus::NotPinned(2),
        },
    );
    assert_eq!(d.verdict, Verdict::Promote);
    // No required baselines is never a promotion.
    lax.require_positive_ci_vs.clear();
    let d = decide(
        &lax,
        &GateInput {
            matchups: &both,
            candidate: ok_counts(),
            checksum: good,
        },
    );
    assert_eq!(d.verdict, Verdict::Hold);
}

#[test]
fn disclaimer_makes_no_claims() {
    let d = mako_eval::gate::DISCLAIMER.to_lowercase();
    assert!(d.contains("no exploitability"));
    assert!(d.contains("not guarantees"));
    assert!(d.contains("none is claimed"));
}
