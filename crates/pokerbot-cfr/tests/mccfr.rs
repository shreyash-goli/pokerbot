//! M-10: external-sampling MCCFR updates, table, determinism.
//! Tests are prefixed `mccfr_` so `cargo test -p pokerbot-cfr mccfr` selects them.

use std::time::Instant;

use pokerbot_cfr::abstraction::key::InfoKey;
use pokerbot_cfr::infoset::StrategyTable;
use pokerbot_cfr::regret::{
    FULL_MASK, NUM_SLOTS, UpdateConfig, accumulate_strategy, average_masked, is_legal, mask_of,
    regret_matching, regret_matching_masked, update_regrets,
};
use pokerbot_cfr::traverse::{Trainer, TrainerConfig, key_bytes};
use pokerbot_core::action::Seat;
use pokerbot_core::card::parse_cards;
use pokerbot_core::deck::Deck;
use pokerbot_core::rng::PokerbotRng;
use pokerbot_core::rules::{ActionKind, Position, RulesConfig, StackDepth, Street};

const EPS: f64 = 1e-12;

fn assert_dist(p: &[f64; NUM_SLOTS], mask: u8) {
    let s: f64 = p.iter().sum();
    assert!((s - 1.0).abs() < EPS, "sum {s}");
    for (i, &x) in p.iter().enumerate() {
        assert!(x.is_finite() && (0.0..=1.0).contains(&x));
        if !is_legal(mask, i) {
            assert_eq!(x, 0.0, "illegal slot {i} has mass");
        }
    }
}

// ------------------------------------------------------------------ regret rules

#[test]
fn mccfr_masked_regret_matching_never_uses_illegal_slots() {
    let mut rng = PokerbotRng::seed_from_u64(1);
    let mut out = [0.0; NUM_SLOTS];
    for _ in 0..20_000 {
        let mask = (1 + rng.gen_range(63)) as u8;
        let mut r = [0.0; NUM_SLOTS];
        for x in r.iter_mut() {
            *x = (rng.gen_f64() - 0.5) * 1e3;
        }
        regret_matching_masked(&r, mask, &mut out);
        assert_dist(&out, mask);
        average_masked(&r, mask, &mut out);
        assert_dist(&out, mask);
    }
    // Large positive regret on an illegal slot is ignored.
    let r = [1e9, 0.0, -1.0, 0.0, 0.0, 0.0];
    let mask = mask_of([ActionKind::Call, ActionKind::Pot]);
    regret_matching_masked(&r, mask, &mut out);
    assert_eq!(out, [0.0, 0.0, 0.5, 0.0, 0.5, 0.0]);
    // Full mask agrees with the generic Kuhn-validated helper.
    let r = [3.0, -1.0, 1.0, 0.0, 4.0, f64::NAN];
    regret_matching_masked(&r, FULL_MASK, &mut out);
    let mut generic = [0.0; NUM_SLOTS];
    regret_matching(&r, &mut generic);
    assert_eq!(out, generic);
}

#[test]
fn mccfr_hand_computed_single_updates() {
    // Mask {Fold, Call, Pot}, zero regrets → uniform 1/3.
    let mask = mask_of([ActionKind::Fold, ActionKind::Call, ActionKind::Pot]);
    let mut regrets = [0.0; NUM_SLOTS];
    let mut sigma = [0.0; NUM_SLOTS];
    regret_matching_masked(&regrets, mask, &mut sigma);
    let third = 1.0 / 3.0;
    assert_eq!(sigma, [third, 0.0, third, 0.0, third, 0.0]);
    // Utils -1, 2, 5 (illegal slots carry garbage that must be ignored).
    let utils = [-1.0, 99.0, 2.0, 99.0, 5.0, 99.0];
    let node = update_regrets(
        &mut regrets,
        mask,
        &sigma,
        &utils,
        1.0,
        UpdateConfig::default(),
    );
    assert!((node - 2.0).abs() < EPS);
    assert!((regrets[0] + 3.0).abs() < EPS && regrets[2].abs() < EPS);
    assert!((regrets[4] - 3.0).abs() < EPS);
    assert_eq!((regrets[1], regrets[3], regrets[5]), (0.0, 0.0, 0.0));
    regret_matching_masked(&regrets, mask, &mut sigma);
    // Call regret is 0 up to rounding (2 − 2.0000000000000001); Pot dominates.
    assert!(sigma[4] > 1.0 - 1e-12 && sigma[2] < 1e-12);
    assert_eq!(
        (sigma[0], sigma[1], sigma[3], sigma[5]),
        (0.0, 0.0, 0.0, 0.0)
    );
    // RM+ floors negatives.
    let mut rp = [0.0; NUM_SLOTS];
    let cfg = UpdateConfig {
        regret_matching_plus: true,
    };
    update_regrets(
        &mut rp,
        mask,
        &[third, 0.0, third, 0.0, third, 0.0],
        &utils,
        1.0,
        cfg,
    );
    assert!(rp[0] == 0.0 && rp[2].abs() < EPS && (rp[4] - 3.0).abs() < EPS);
    // Weighted strategy accumulation.
    let mut sums = [0.0; NUM_SLOTS];
    accumulate_strategy(&mut sums, mask, &[0.5, 0.0, 0.25, 0.0, 0.25, 0.0], 2.0);
    accumulate_strategy(&mut sums, mask, &[0.0, 0.0, 0.0, 0.0, 1.0, 0.0], 2.0);
    let mut avg = [0.0; NUM_SLOTS];
    average_masked(&sums, mask, &mut avg);
    assert_eq!(avg, [0.25, 0.0, 0.125, 0.0, 0.625, 0.0]);
}

/// Scripted subtree: 1/2 blinds, 2-chip stacks. The big blind is all-in from
/// posting, so the button's only decision is Fold (−1 chip) or Call (all-in
/// showdown). With AA vs 72o on a dry board the call wins +2 chips.
#[test]
fn mccfr_scripted_traversal_matches_hand_computation() {
    let mut cfg = TrainerConfig::tiny(0);
    cfg.rules = RulesConfig::new(1, 2, 2).unwrap();
    cfg.verify_keys = true;
    let mut t = Trainer::new(cfg).unwrap();
    // Deal order BB, BTN, BB, BTN, then board. Button = seat Zero.
    let deck = || Deck::from_order(parse_cards("7c As 2d Ah 3s 4s 9d Jc Kh").unwrap()).unwrap();
    let u = t.traverse_deck(deck(), Seat::Zero);
    // σ = (½, ½): u_fold = −0.5 BB, u_call = +1 BB, node = 0.25.
    assert!((u - 0.25).abs() < EPS);
    assert_eq!(t.table().len(), 1);
    let key = key_bytes(Street::Preflop, Position::Button, 0, &[]);
    let e = t
        .table()
        .get(&key)
        .expect("button root key (AA = bucket 0)");
    assert_eq!(e.mask, mask_of([ActionKind::Fold, ActionKind::Call]));
    assert!((e.regret[0] + 0.75).abs() < EPS && (e.regret[2] - 0.75).abs() < EPS);
    assert_eq!(e.strategy_sum, [0.0; NUM_SLOTS]);
    // Second traversal: σ = (0, 1), node = 1; regrets −2.25 / 0.75.
    let u = t.traverse_deck(deck(), Seat::Zero);
    assert!((u - 1.0).abs() < EPS);
    let e = t.table().get(&key).unwrap();
    assert!((e.regret[0] + 2.25).abs() < EPS && (e.regret[2] - 0.75).abs() < EPS);
    // Big blind as traverser: it never acts; the button's node is an
    // opponent node, so its current strategy (0, 1) is averaged.
    let u = t.traverse_deck(deck(), Seat::One);
    assert!((u + 1.0).abs() < EPS, "BB loses 2 chips = 1 BB");
    let e = t.table().get(&key).unwrap();
    assert_eq!(e.strategy_sum, [0.0, 0.0, 1.0, 0.0, 0.0, 0.0]);
    assert_eq!(e.average_strategy(), [0.0, 0.0, 1.0, 0.0, 0.0, 0.0]);
    assert_eq!(t.stats().keys_verified, 3);
}

// ------------------------------------------------------------------ table

#[test]
fn mccfr_table_insertion_order_and_mask_consistency() {
    let mut t = StrategyTable::new();
    let m = mask_of([ActionKind::Check, ActionKind::Pot]);
    assert_eq!(t.index_or_insert(b"b", m), 0);
    assert_eq!(t.index_or_insert(b"a", m), 1);
    assert_eq!(t.index_or_insert(b"b", m), 0);
    let keys: Vec<&[u8]> = t.iter().map(|(k, _)| k).collect();
    assert_eq!(keys, vec![&b"b"[..], &b"a"[..]]);
    let r = std::panic::catch_unwind(move || {
        let mut t = t;
        t.index_or_insert(b"a", FULL_MASK);
    });
    assert!(r.is_err(), "mask change must be rejected");
}

// ------------------------------------------------------------------ training

fn assert_table_sane(t: &Trainer) {
    for (k, e) in t.table().iter() {
        assert!(e.mask != 0 && e.mask & !FULL_MASK == 0);
        for i in 0..NUM_SLOTS {
            assert!(e.regret[i].is_finite() && e.strategy_sum[i].is_finite());
            assert!(e.strategy_sum[i] >= 0.0);
            if !is_legal(e.mask, i) {
                assert_eq!((e.regret[i], e.strategy_sum[i]), (0.0, 0.0));
            }
        }
        assert_dist(&e.current_strategy(), e.mask);
        assert_dist(&e.average_strategy(), e.mask);
        let key = InfoKey::from_bytes(k).expect("table keys are valid InfoKeys");
        assert!(key.bucket() < t.abstraction_buckets(key.street()));
    }
}

trait Buckets {
    fn abstraction_buckets(&self, s: Street) -> u16;
}

impl Buckets for Trainer {
    fn abstraction_buckets(&self, s: Street) -> u16 {
        use pokerbot_cfr::abstraction::key::CardAbstraction;
        self.abstraction().num_buckets(s)
    }
}

#[test]
fn mccfr_training_is_finite_normalized_and_grows() {
    let mut t = Trainer::new(TrainerConfig::tiny(7)).unwrap();
    t.run_iterations(50);
    let n50 = t.table().len();
    t.run_iterations(250);
    let n300 = t.table().len();
    assert!(n300 > n50 && n50 > 0, "{n50} -> {n300}");
    assert_eq!(t.iteration(), 300);
    let s = t.stats();
    assert_eq!(s.traversals, 600);
    assert!(s.opponent_samples > 0 && s.traverser_nodes > 0);
    assert!(s.nodes == s.traverser_nodes + s.opponent_samples);
    assert_table_sane(&t);
    eprintln!("tiny 300 iterations: {} infosets, stats {s:?}", n300);
}

#[test]
fn mccfr_same_seed_identical_table_different_seed_differs() {
    let run = |seed| {
        let mut t = Trainer::new(TrainerConfig::tiny(seed)).unwrap();
        t.run_iterations(150);
        t
    };
    let (a, b, c) = (run(42), run(42), run(43));
    assert_eq!(a.table().digest(), b.table().digest());
    assert_eq!(a.table().len(), b.table().len());
    for ((ka, ea), (kb, eb)) in a.table().iter().zip(b.table().iter()) {
        assert_eq!(ka, kb);
        assert_eq!(ea.mask, eb.mask);
        let bits = |x: &[f64; NUM_SLOTS]| x.map(f64::to_bits);
        assert_eq!(bits(&ea.regret), bits(&eb.regret));
        assert_eq!(bits(&ea.strategy_sum), bits(&eb.strategy_sum));
    }
    assert_ne!(a.table().digest(), c.table().digest());
    // Cache on/off does not change training.
    let mut cfg = TrainerConfig::tiny(42);
    cfg.cache_capacity = 0;
    let mut d = Trainer::new(cfg).unwrap();
    d.run_iterations(150);
    assert_eq!(d.table().digest(), a.table().digest());
}

#[test]
fn mccfr_rm_plus_flag_keeps_regrets_nonnegative() {
    let mut cfg = TrainerConfig::tiny(3);
    cfg.update = UpdateConfig {
        regret_matching_plus: true,
    };
    let mut t = Trainer::new(cfg).unwrap();
    t.run_iterations(200);
    assert!(
        t.table()
            .iter()
            .all(|(_, e)| e.regret.iter().all(|&r| r >= 0.0))
    );
    assert_table_sane(&t);
}

/// Sanity only (no strength claim): after a modest run, the button's
/// first-action average strategy is non-uniform, and the strongest preflop
/// classes fold less often than uniform play would.
#[test]
fn mccfr_average_strategy_moves_away_from_uniform() {
    let mut t = Trainer::new(TrainerConfig::tiny(11)).unwrap();
    t.run_iterations(3_000);
    assert_table_sane(&t);
    let mut max_dev: f64 = 0.0;
    let mut strong_fold = Vec::new();
    for (k, e) in t.table().iter() {
        let key = InfoKey::from_bytes(k).unwrap();
        if key.street() != Street::Preflop || !key.history().is_empty() {
            continue;
        }
        let avg = e.average_strategy();
        let n = f64::from((e.mask).count_ones());
        for (i, &p) in avg.iter().enumerate() {
            if is_legal(e.mask, i) {
                max_dev = max_dev.max((p - 1.0 / n).abs());
            }
        }
        if key.bucket() < 10 && e.strategy_sum.iter().sum::<f64>() > 0.0 {
            strong_fold.push(avg[ActionKind::Fold.index()]);
        }
    }
    let mean_fold = strong_fold.iter().sum::<f64>() / strong_fold.len().max(1) as f64;
    eprintln!(
        "button root: max deviation from uniform {max_dev:.3}; mean fold prob of top-10 classes {mean_fold:.3} over {} classes",
        strong_fold.len()
    );
    assert!(max_dev > 0.1);
    assert!(
        !strong_fold.is_empty() && mean_fold < 0.2,
        "uniform fold would be 0.2"
    );
}

// ------------------------------------------------------------------ performance

#[test]
#[ignore = "timing report; run with --release --include-ignored"]
fn mccfr_performance_report() {
    for (name, cfg, iters) in [
        ("tiny 20BB", TrainerConfig::tiny(1), 10_000u64),
        (
            "default 20BB",
            TrainerConfig::standard(StackDepth::Bb20, 1),
            10_000,
        ),
        (
            "default 20BB, turn_samples=1000",
            {
                let mut c = TrainerConfig::standard(StackDepth::Bb20, 1);
                c.abstraction.turn_samples = 1_000;
                c
            },
            10_000,
        ),
    ] {
        let mut t = Trainer::new(cfg).unwrap();
        let start = Instant::now();
        t.run_iterations(iters);
        let dt = start.elapsed().as_secs_f64();
        let s = t.stats();
        eprintln!(
            "{name}: {iters} it in {dt:.2}s = {:.0} it/s; {} infosets, ~{:.1} MB table; nodes/it {:.1}; cache {:?}",
            iters as f64 / dt,
            t.table().len(),
            t.table().approx_bytes() as f64 / 1e6,
            s.nodes as f64 / iters as f64,
            t.abstraction().cache_stats()
        );
    }
}
