//! M-09: CFR reference validation on Kuhn poker.
//! All tests are prefixed `toy_` so `cargo test -p pokerbot-cfr toy` selects them.

use pokerbot_cfr::toy::cfr::{AverageWeighting, ExternalSamplingMccfr, VanillaCfr};
use pokerbot_cfr::toy::eval::{
    GAME_VALUE_P0, KuhnStrategy, best_response_value, expected_value_p0, exploitability,
};
use pokerbot_cfr::toy::kuhn::{Card, Deal, History, InfoSetId, NUM_INFOSETS, info_key};
use pokerbot_cfr::toy::regret::{RegretNode, regret_matching, regret_matching_vec};
use pokerbot_cfr::toy::rng::SplitMix64;

const EPS: f64 = 1e-12;

fn assert_normalized(p: &[f64]) {
    let sum: f64 = p.iter().sum();
    assert!((sum - 1.0).abs() < EPS, "sum {sum} for {p:?}");
    for &x in p {
        assert!(x.is_finite() && (0.0..=1.0).contains(&x), "bad prob {x}");
    }
}

fn assert_table_valid(s: &KuhnStrategy) {
    for row in &s.probs {
        assert_normalized(row);
    }
}

fn bits(s: &KuhnStrategy) -> Vec<u64> {
    s.probs.iter().flatten().map(|x| x.to_bits()).collect()
}

// ---------------------------------------------------------------- regret matching

#[test]
fn toy_regret_matching_positive_part_normalization() {
    let p = regret_matching_vec(&[3.0, -2.0, 1.0, 0.0]);
    assert_normalized(&p);
    assert_eq!(p, vec![0.75, 0.0, 0.25, 0.0]);
}

#[test]
fn toy_regret_matching_uniform_fallback() {
    for r in [
        vec![0.0, 0.0, 0.0],
        vec![-1.0, -5.0, 0.0],
        vec![f64::NAN, -1.0, 0.0],
    ] {
        let p = regret_matching_vec(&r);
        assert_normalized(&p);
        for &x in &p {
            assert!((x - 1.0 / 3.0).abs() < EPS);
        }
    }
}

#[test]
fn toy_regret_matching_single_action_and_empty() {
    assert_eq!(regret_matching_vec(&[-4.0]), vec![1.0]);
    assert_eq!(regret_matching_vec(&[7.0]), vec![1.0]);
    assert!(regret_matching_vec(&[]).is_empty());
}

#[test]
fn toy_regret_matching_random_inputs_normalized() {
    let mut rng = SplitMix64::new(42);
    let mut out = [0.0; 6];
    for _ in 0..10_000 {
        let n = 1 + rng.below(6) as usize;
        let r: Vec<f64> = (0..n).map(|_| (rng.next_f64() - 0.5) * 1e6).collect();
        regret_matching(&r, &mut out[..n]);
        assert_normalized(&out[..n]);
        for i in 0..n {
            if r[i] <= 0.0 && r.iter().any(|&x| x > 0.0) {
                assert_eq!(out[i], 0.0);
            }
        }
    }
}

#[test]
fn toy_regret_node_average_accumulation() {
    let mut n = RegretNode::new(2);
    let mut avg = [0.0; 2];
    n.average_strategy(&mut avg);
    assert_eq!(avg, [0.5, 0.5], "unreached node is uniform");
    n.accumulate_strategy(&[1.0, 0.0], 1.0);
    n.accumulate_strategy(&[0.0, 1.0], 3.0);
    n.average_strategy(&mut avg);
    assert_eq!(avg, [0.25, 0.75]);
    n.add_regrets(&[-1.0, 2.0]);
    let mut cur = [0.0; 2];
    n.current_strategy(&mut cur);
    assert_eq!(cur, [0.0, 1.0]);
}

// ---------------------------------------------------------------- Kuhn rules / keys

#[test]
fn toy_kuhn_terminal_utilities() {
    use Card::*;
    let u = |h: &str, c0, c1| History::parse(h).terminal_utility_p0([c0, c1]);
    assert_eq!(u("pp", K, J), 1);
    assert_eq!(u("pp", J, Q), -1);
    assert_eq!(u("bp", J, K), 1);
    assert_eq!(u("pbp", K, J), -1);
    assert_eq!(u("bb", Q, K), -2);
    assert_eq!(u("pbb", K, Q), 2);
    for h in ["", "p", "b", "pb"] {
        assert!(!History::parse(h).is_terminal());
    }
    for h in ["pp", "bp", "bb", "pbp", "pbb"] {
        let hist = History::parse(h);
        assert!(hist.is_terminal());
        // Zero-sum and integer: swapping cards flips showdown sign; folds don't depend on cards.
        for d in Deal::all() {
            let v = hist.terminal_utility_p0(d.cards);
            assert!(v.abs() == 1 || v.abs() == 2);
        }
    }
}

#[test]
fn toy_info_key_ignores_opponent_card() {
    // info_key takes only an observation (own card + public history); verify
    // that across all deals the key depends on nothing else.
    for h in ["", "p", "b", "pb"] {
        let hist = History::parse(h);
        let p = hist.player_to_act();
        for own in Card::ALL {
            let keys: Vec<InfoSetId> = Deal::all()
                .into_iter()
                .filter(|d| d.cards[p] == own)
                .map(|d| info_key(&d.observation(p, hist)))
                .collect();
            assert_eq!(keys.len(), 2, "two opponent cards per own card");
            assert_eq!(keys[0], keys[1], "key varied with opponent card at {h}");
            assert_eq!(keys[0].card(), own);
            assert_eq!(keys[0].history(), hist);
        }
    }
    // 12 distinct information sets, round-trip through display.
    let names: std::collections::BTreeSet<String> =
        InfoSetId::all().map(|i| i.to_string()).collect();
    assert_eq!(names.len(), NUM_INFOSETS);
    assert!(names.contains("Q:pb"));
}

// ---------------------------------------------------------------- exact evaluation

#[test]
fn toy_equilibrium_family_is_exact() {
    for alpha in [0.0, 1.0 / 6.0, 0.25, 1.0 / 3.0] {
        let s = KuhnStrategy::equilibrium(alpha);
        assert!((expected_value_p0(&s) - GAME_VALUE_P0).abs() < EPS);
        assert!(exploitability(&s).abs() < EPS, "alpha {alpha}");
        assert!((best_response_value(&s, 0) - GAME_VALUE_P0).abs() < EPS);
        assert!((best_response_value(&s, 1) + GAME_VALUE_P0).abs() < EPS);
    }
}

/// Cross-check the recursive best response against brute force over all
/// 2^6 pure strategies of the responding player.
#[test]
fn toy_best_response_matches_brute_force() {
    let mut rng = SplitMix64::new(7);
    for _ in 0..5 {
        let mut s = KuhnStrategy::uniform();
        for row in s.probs.iter_mut() {
            let b = rng.next_f64();
            *row = [1.0 - b, b];
        }
        for br in 0..2 {
            let ids: Vec<InfoSetId> = InfoSetId::all().filter(|i| i.player() == br).collect();
            assert_eq!(ids.len(), 6);
            let mut best = f64::NEG_INFINITY;
            for mask in 0u32..64 {
                let mut t = s.clone();
                for (k, id) in ids.iter().enumerate() {
                    let bet = f64::from((mask >> k) & 1);
                    t.probs[id.0] = [1.0 - bet, bet];
                }
                let v = expected_value_p0(&t);
                best = best.max(if br == 0 { v } else { -v });
            }
            let fast = best_response_value(&s, br);
            assert!((fast - best).abs() < 1e-12, "br{br}: {fast} vs {best}");
        }
        assert!(exploitability(&s) >= -EPS);
    }
}

// ---------------------------------------------------------------- vanilla CFR

#[test]
fn toy_vanilla_cfr_converges_to_known_equilibrium() {
    let mut cfr = VanillaCfr::new(AverageWeighting::Uniform);
    let mut expl = Vec::new();
    let mut done = 0;
    for n in [100u64, 1_000, 10_000] {
        cfr.run(n - done);
        done = n;
        let s = cfr.average_strategy();
        assert_table_valid(&s);
        assert_table_valid(&cfr.current_strategy());
        let e = exploitability(&s);
        eprintln!(
            "vanilla t={n}: exploitability={e:.6} ev_p0={:.6}",
            expected_value_p0(&s)
        );
        expl.push(e);
    }
    assert_eq!(cfr.iterations(), 10_000);
    // Observed: ~0.0257, ~0.0073, ~0.0023.
    assert!(expl[0] > expl[1] && expl[1] > expl[2], "{expl:?}");
    assert!(expl[2] < 0.005, "{expl:?}");

    let s = cfr.average_strategy();
    assert!((expected_value_p0(&s) - GAME_VALUE_P0).abs() < 0.005);
    // Equilibrium-family structure.
    use Card::*;
    let alpha = s.bet_prob(J, "");
    assert!((0.0..=1.0 / 3.0 + 0.02).contains(&alpha), "alpha {alpha}");
    assert!(s.bet_prob(Q, "") < 0.01);
    assert!((s.bet_prob(K, "") - 3.0 * alpha).abs() < 0.03);
    assert!(s.bet_prob(J, "pb") < 0.01);
    assert!((s.bet_prob(Q, "pb") - (alpha + 1.0 / 3.0)).abs() < 0.03);
    assert!(s.bet_prob(K, "pb") > 0.99);
    assert!((s.bet_prob(J, "p") - 1.0 / 3.0).abs() < 0.03);
    assert!(s.bet_prob(Q, "p") < 0.01);
    assert!(s.bet_prob(K, "p") > 0.99);
    assert!(s.bet_prob(J, "b") < 0.01);
    assert!((s.bet_prob(Q, "b") - 1.0 / 3.0).abs() < 0.03);
    assert!(s.bet_prob(K, "b") > 0.99);
}

#[test]
fn toy_vanilla_cfr_is_deterministic_and_linear_option_converges() {
    let mut a = VanillaCfr::new(AverageWeighting::Uniform);
    let mut b = VanillaCfr::new(AverageWeighting::Uniform);
    a.run(500);
    b.run(500);
    assert_eq!(bits(&a.average_strategy()), bits(&b.average_strategy()));

    let mut lin = VanillaCfr::new(AverageWeighting::Linear);
    lin.run(10_000);
    let s = lin.average_strategy();
    assert_table_valid(&s);
    let e = exploitability(&s);
    eprintln!("vanilla linear t=10000: exploitability={e:.6}");
    // Observed ~0.0042.
    assert!(e < 0.01);
}

// ---------------------------------------------------------------- external-sampling MCCFR

#[test]
fn toy_mccfr_same_seed_bit_identical() {
    let mut a = ExternalSamplingMccfr::new(12345, AverageWeighting::Uniform);
    let mut b = ExternalSamplingMccfr::new(12345, AverageWeighting::Uniform);
    a.run(5_000);
    b.run(5_000);
    assert_eq!(bits(&a.average_strategy()), bits(&b.average_strategy()));
    for (x, y) in a.nodes().iter().zip(b.nodes()) {
        let xb: Vec<u64> = x.regret_sum.iter().map(|v| v.to_bits()).collect();
        let yb: Vec<u64> = y.regret_sum.iter().map(|v| v.to_bits()).collect();
        assert_eq!(xb, yb);
    }
}

#[test]
fn toy_mccfr_different_seeds_differ() {
    let mut a = ExternalSamplingMccfr::new(1, AverageWeighting::Uniform);
    let mut b = ExternalSamplingMccfr::new(2, AverageWeighting::Uniform);
    a.run(5_000);
    b.run(5_000);
    assert_ne!(bits(&a.average_strategy()), bits(&b.average_strategy()));
}

#[test]
fn toy_mccfr_converges() {
    for (seed, w) in [
        (1u64, AverageWeighting::Uniform),
        (2, AverageWeighting::Uniform),
        (3, AverageWeighting::Linear),
    ] {
        let mut m = ExternalSamplingMccfr::new(seed, w);
        let mut expl = Vec::new();
        let mut done = 0;
        for n in [1_000u64, 10_000, 100_000] {
            m.run(n - done);
            done = n;
            let s = m.average_strategy();
            assert_table_valid(&s);
            for node in m.nodes() {
                assert!(node.regret_sum.iter().all(|r| r.is_finite()));
            }
            let e = exploitability(&s);
            eprintln!(
                "es-mccfr seed={seed} {w:?} t={n}: exploitability={e:.6} ev_p0={:.6}",
                expected_value_p0(&s)
            );
            expl.push(e);
        }
        // Observed at t=1k/10k/100k: 0.027-0.094 / 0.005-0.016 / 0.002-0.005.
        assert!(expl[0] > expl[2], "seed {seed}: {expl:?}");
        assert!(expl[2] < 0.012, "seed {seed}: {expl:?}");
        let ev = expected_value_p0(&m.average_strategy());
        assert!((ev - GAME_VALUE_P0).abs() < 0.005, "ev {ev}");
    }
}

#[test]
fn toy_strategies_far_from_equilibrium_are_exploitable() {
    let e = exploitability(&KuhnStrategy::uniform());
    assert!(e > 0.1, "uniform exploitability {e}");
}
