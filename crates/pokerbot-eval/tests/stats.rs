//! Statistics against hand-computed values.

use pokerbot_core::rules::{RulesConfig, StackDepth};
use pokerbot_eval::matchup::{DecisionCounts, MatchResult, PairResult};
use pokerbot_eval::report::summarize_match;
use pokerbot_eval::stats::{Z95, bb_per_100, bootstrap_ci, summarize};

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

#[test]
fn mean_variance_ci_hand_computed() {
    // x = 1..5: mean 3, Σ(x-m)² = 10, s² = 10/4 = 2.5, s = 1.58113883,
    // se = s/√5 = 0.70710678, CI = 3 ± 1.959964 · 0.70710678 = 3 ± 1.38590382.
    let s = summarize(&[1.0, 2.0, 3.0, 4.0, 5.0], Z95);
    assert_eq!(s.n, 5);
    assert!(close(s.mean, 3.0));
    assert!(close(s.sd, 2.5f64.sqrt()));
    assert!(close(s.se, 0.5f64.sqrt()));
    assert!((s.ci_low - (3.0 - 1.385_903_82)).abs() < 1e-7);
    assert!((s.ci_high - (3.0 + 1.385_903_82)).abs() < 1e-7);
}

#[test]
fn degenerate_samples() {
    let one = summarize(&[4.0], Z95);
    assert_eq!(one.n, 1);
    assert!(one.ci_low.is_infinite() && one.ci_high.is_infinite());
    assert!(!one.significantly_positive());
    let empty = summarize(&[], Z95);
    assert_eq!(empty.n, 0);
    assert!(!empty.significantly_positive());
    let constant = summarize(&[3.0; 10], Z95);
    assert!(close(constant.sd, 0.0) && close(constant.ci_low, 3.0));
    assert!(constant.significantly_positive());
}

#[test]
fn bb_per_100_scaling() {
    assert!(close(bb_per_100(1.0, 2), 50.0));
    assert!(close(bb_per_100(-0.5, 2), -25.0));
    assert!(close(bb_per_100(10.0, 10), 100.0));
}

#[test]
fn matchup_report_scales_pairs_to_hands() {
    // Pair totals 4, -2, 6, 0 chips over 8 hands: 8 chips / 8 hands =
    // 1 chip/hand = 0.5 BB/hand = 50 BB/100.
    let pairs = [(3, 1), (-1, -1), (2, 4), (5, -5)]
        .into_iter()
        .enumerate()
        .map(|(i, (b, bb))| PairResult {
            deal_seed: i as u64,
            a_button: b,
            a_big_blind: bb,
        })
        .collect::<Vec<_>>();
    let m = MatchResult {
        label_a: "a".into(),
        label_b: "b".into(),
        rules: RulesConfig::standard(StackDepth::Bb20),
        base_seed: 0,
        pairs,
        counts_a: DecisionCounts::default(),
        counts_b: DecisionCounts::default(),
        infer_a: None,
        infer_b: None,
        adapt_a: None,
        adapt_b: None,
        delivered_a: 0,
        delivered_b: 0,
        showdowns: 0,
    };
    let r = summarize_match("x", &m, None);
    assert_eq!(r.hands, 8);
    assert_eq!(r.a_net_chips, 8);
    assert_eq!(r.a_net_chips + r.b_net_chips, 0);
    assert!(close(r.bb100.mean, 50.0));
    // Pair totals sd: mean 2, deviations 2,-4,4,-2 → Σ=40, s²=40/3.
    let sd_pairs = (40.0f64 / 3.0).sqrt();
    assert!(close(r.bb100.sd, sd_pairs * 25.0));
    assert!(close(r.bb100.se, sd_pairs / 2.0 * 25.0));
    // Button: 3,-1,2,5 → mean 2.25 chips/hand = 112.5 BB/100.
    assert!(close(r.button_bb100.mean, 112.5));
    // Big blind: 1,-1,4,-5 → mean -0.25 chips/hand = -12.5 BB/100.
    assert!(close(r.big_blind_bb100.mean, -12.5));
}

#[test]
fn bootstrap_is_seeded_and_sane() {
    let xs: Vec<f64> = (0..200).map(|i| ((i * 37) % 11) as f64 - 5.0).collect();
    let a = bootstrap_ci(&xs, 1000, 5).unwrap();
    let b = bootstrap_ci(&xs, 1000, 5).unwrap();
    assert_eq!(a, b);
    let s = summarize(&xs, Z95);
    assert!(a.0 < s.mean && s.mean < a.1);
    // Roughly agrees with the normal CI.
    assert!((a.0 - s.ci_low).abs() < 0.5 * s.half_width());
    assert_eq!(bootstrap_ci(&[2.0; 5], 100, 1), Some((2.0, 2.0)));
    assert_eq!(bootstrap_ci(&[], 100, 1), None);
}
