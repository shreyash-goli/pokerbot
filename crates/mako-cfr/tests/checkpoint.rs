//! M-11: linear weighting, bounded training, checkpoint/resume.
//! Tests are prefixed `checkpoint_` so `cargo test -p mako-cfr checkpoint`
//! selects them.

use std::sync::atomic::{AtomicBool, Ordering};

use mako_cfr::checkpoint::{
    CHECKPOINT_VERSION, Checkpoint, CheckpointError, load_file, save_atomic,
};
use mako_cfr::regret::{UpdateConfig, Weighting, flags_from_bits, flags_to_bits};
use mako_cfr::train::{MAX_ITERATIONS, StopReason, TrainConfig, TrainError, TrainEvent, train};
use mako_cfr::traverse::Trainer;

fn run(cfg: &TrainConfig, resume: Option<Checkpoint>) -> (Trainer, Vec<Checkpoint>) {
    let cancel = AtomicBool::new(false);
    let mut cps = Vec::new();
    let out = train(cfg, resume, &cancel, &mut |e| {
        if let TrainEvent::Checkpoint(c) = e {
            cps.push(c.clone());
        }
    })
    .unwrap();
    assert_eq!(out.reason, StopReason::Completed);
    (out.trainer, cps)
}

fn final_bytes(t: &Trainer) -> Vec<u8> {
    Checkpoint::from_trainer(t).to_bytes()
}

fn flag_sets() -> Vec<Weighting> {
    vec![
        Weighting::default(),
        Weighting {
            linear_averaging: true,
            linear_regrets: false,
        },
        Weighting {
            linear_averaging: true,
            linear_regrets: true,
        },
    ]
}

#[test]
fn checkpoint_weight_formula_and_flag_bits() {
    let w = Weighting {
        linear_averaging: true,
        linear_regrets: false,
    };
    assert_eq!(w.strategy_weight(1), 1.0);
    assert_eq!(w.strategy_weight(7), 7.0);
    assert_eq!(w.strategy_weight(0), 1.0);
    assert_eq!(w.regret_weight(7), 1.0);
    let both = Weighting {
        linear_averaging: true,
        linear_regrets: true,
    };
    assert_eq!(both.regret_weight(12), 12.0);
    assert_eq!(Weighting::default().strategy_weight(99), 1.0);
    for bits in 0..8 {
        let (u, w) = flags_from_bits(bits).unwrap();
        assert_eq!(flags_to_bits(u, w), bits);
    }
    assert!(flags_from_bits(8).is_none());
    let rm = UpdateConfig {
        regret_matching_plus: true,
    };
    assert_eq!(flags_to_bits(rm, Weighting::default()), 1);
}

#[test]
fn checkpoint_resume_matches_uninterrupted_at_several_splits() {
    const N: u64 = 160;
    for weighting in flag_sets() {
        let mut cfg = TrainConfig::tiny(21, N);
        cfg.weighting = weighting;
        let (straight, _) = run(&cfg, None);
        let want = final_bytes(&straight);
        for split in [1u64, 37, 80, 159] {
            let mut first = cfg;
            first.iterations = split;
            let (part, cps) = run(&first, None);
            let cp = cps.last().unwrap();
            assert_eq!(cp.iteration, split);
            // Through bytes, as a real resume would.
            let reloaded = Checkpoint::from_bytes(&cp.to_bytes()).unwrap();
            assert_eq!(reloaded.to_bytes(), final_bytes(&part));
            let (resumed, _) = run(&cfg, Some(reloaded));
            assert_eq!(resumed.iteration(), N);
            assert_eq!(resumed.rng_state(), straight.rng_state());
            assert_eq!(resumed.stats(), straight.stats());
            assert_eq!(resumed.table().digest(), straight.table().digest());
            assert_eq!(final_bytes(&resumed), want, "{weighting:?} split {split}");
        }
    }
}

#[test]
fn checkpoint_periodic_checkpoints_and_atomic_file_roundtrip() {
    let mut cfg = TrainConfig::tiny(4, 120);
    cfg.checkpoint_every = Some(50);
    let (t, cps) = run(&cfg, None);
    let its: Vec<u64> = cps.iter().map(|c| c.iteration).collect();
    assert_eq!(its, vec![50, 100, 120]);
    let dir = std::env::temp_dir().join(format!("mako-ckpt-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("run.mkck");
    save_atomic(&path, &cps[0].to_bytes()).unwrap();
    save_atomic(&path, &cps[2].to_bytes()).unwrap(); // overwrite atomically
    assert!(!dir.join("run.mkck.tmp").exists());
    let loaded = load_file(&path).unwrap();
    assert_eq!(loaded.to_bytes(), final_bytes(&t));
    // Resume from the 50-iteration checkpoint reaches the same final state.
    let (resumed, _) = run(&cfg, Some(cps[0].clone()));
    assert_eq!(final_bytes(&resumed), final_bytes(&t));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn checkpoint_cancellation_stops_early_with_usable_checkpoint() {
    let mut cfg = TrainConfig::tiny(8, 100_000);
    cfg.checkpoint_every = Some(25);
    let cancel = AtomicBool::new(false);
    let mut last = None;
    let out = train(&cfg, None, &cancel, &mut |e| {
        if let TrainEvent::Checkpoint(c) = e {
            if c.iteration >= 50 {
                cancel.store(true, Ordering::Relaxed);
            }
            last = Some(c.clone());
        }
    })
    .unwrap();
    assert_eq!(out.reason, StopReason::Cancelled);
    assert_eq!(out.iterations_run, 50);
    let cp = last.unwrap();
    assert_eq!(cp.iteration, 50);
    // Resume to 90 equals a straight 90-iteration run.
    let mut to90 = cfg;
    to90.iterations = 90;
    to90.checkpoint_every = None;
    let (resumed, _) = run(&to90, Some(Checkpoint::from_bytes(&cp.to_bytes()).unwrap()));
    let (straight, _) = run(&to90, None);
    assert_eq!(final_bytes(&resumed), final_bytes(&straight));
}

#[test]
fn checkpoint_wall_clock_and_bounds() {
    let mut cfg = TrainConfig::tiny(1, 1_000);
    cfg.max_wall_secs = Some(0);
    let cancel = AtomicBool::new(false);
    let out = train(&cfg, None, &cancel, &mut |_| {}).unwrap();
    assert_eq!(out.reason, StopReason::WallClock);
    assert_eq!(out.iterations_run, 0);
    // No unbounded or degenerate runs.
    for bad in [0, MAX_ITERATIONS + 1] {
        let c = TrainConfig::tiny(1, bad);
        assert!(matches!(
            train(&c, None, &cancel, &mut |_| {}),
            Err(TrainError::Config(_))
        ));
    }
    let mut c = TrainConfig::tiny(1, 10);
    c.checkpoint_every = Some(0);
    assert!(train(&c, None, &cancel, &mut |_| {}).is_err());
    // Pre-set cancel flag: nothing runs, but a checkpoint is still emitted.
    let stop = AtomicBool::new(true);
    let mut n = 0;
    let out = train(&TrainConfig::tiny(1, 10), None, &stop, &mut |e| {
        if matches!(e, TrainEvent::Checkpoint(_)) {
            n += 1;
        }
    })
    .unwrap();
    assert_eq!(
        (out.reason, out.iterations_run, n),
        (StopReason::Cancelled, 0, 1)
    );
}

#[test]
fn checkpoint_rejects_corruption_truncation_version_and_mismatch() {
    let (t, _) = run(&TrainConfig::tiny(5, 40), None);
    let good = final_bytes(&t);
    assert!(Checkpoint::from_bytes(&good).is_ok());

    let mut bad = good.clone();
    bad[0] = b'X';
    assert_eq!(
        Checkpoint::from_bytes(&bad).unwrap_err(),
        CheckpointError::BadMagic
    );

    let mut bad = good.clone();
    bad[8..12].copy_from_slice(&(CHECKPOINT_VERSION + 1).to_le_bytes());
    assert_eq!(
        Checkpoint::from_bytes(&bad).unwrap_err(),
        CheckpointError::UnsupportedVersion(CHECKPOINT_VERSION + 1)
    );

    let mut bad = good.clone();
    let mid = bad.len() / 2;
    bad[mid] ^= 0x40;
    assert_eq!(
        Checkpoint::from_bytes(&bad).unwrap_err(),
        CheckpointError::ChecksumMismatch
    );

    for cut in [3usize, 15, 100, good.len() - 1] {
        assert!(Checkpoint::from_bytes(&good[..cut]).is_err(), "cut {cut}");
    }
    let mut longer = good.clone();
    longer.push(0);
    assert!(Checkpoint::from_bytes(&longer).is_err());

    // Garbage before a *recomputed* checksum: structurally rejected.
    let mut body = good[..good.len() - 8].to_vec();
    body.extend_from_slice(&[1, 2, 3]);
    let sum = mako_cfr::abstraction::key::fnv1a64(&body);
    body.extend_from_slice(&sum.to_le_bytes());
    assert_eq!(
        Checkpoint::from_bytes(&body).unwrap_err(),
        CheckpointError::TrailingBytes
    );

    // Resume with a different config.
    let cp = Checkpoint::from_bytes(&good).unwrap();
    let base = TrainConfig::tiny(5, 80);
    let mut variants = Vec::new();
    let mut c = base;
    c.seed = 6;
    variants.push(c);
    let mut c = base;
    c.rules = mako_core::rules::RulesConfig::standard(mako_core::rules::StackDepth::Bb50);
    variants.push(c);
    let mut c = base;
    c.abstraction.flop_buckets = 4;
    variants.push(c);
    let mut c = base;
    c.weighting.linear_averaging = true;
    variants.push(c);
    let cancel = AtomicBool::new(false);
    for v in variants {
        match train(&v, Some(cp.clone()), &cancel, &mut |_| {}) {
            Err(TrainError::Checkpoint(CheckpointError::ConfigMismatch(_))) => {}
            other => panic!(
                "expected config mismatch, got {:?}",
                other.map(|o| o.reason)
            ),
        }
    }
    // Cache capacity is not part of compatibility.
    let mut c = base;
    c.cache_capacity = 0;
    assert!(train(&c, Some(cp.clone()), &cancel, &mut |_| {}).is_ok());
    // Target already passed.
    let c = TrainConfig::tiny(5, 10);
    assert!(matches!(
        train(&c, Some(cp), &cancel, &mut |_| {}),
        Err(TrainError::Config(_))
    ));
}

#[test]
fn checkpoint_linear_averaging_changes_average_not_regrets() {
    let plain = run(&TrainConfig::tiny(13, 300), None).0;
    let mut cfg = TrainConfig::tiny(13, 300);
    cfg.weighting.linear_averaging = true;
    let lin = run(&cfg, None).0;
    assert_eq!(plain.table().len(), lin.table().len());
    let mut avg_differs = 0;
    for ((ka, ea), (kb, eb)) in plain.table().iter().zip(lin.table().iter()) {
        assert_eq!(ka, kb);
        assert_eq!(ea.regret.map(f64::to_bits), eb.regret.map(f64::to_bits));
        if ea.average_strategy() != eb.average_strategy() {
            avg_differs += 1;
        }
    }
    assert!(avg_differs > 0);
    // Linear regrets change regrets (and therefore the sampled path).
    let mut cfg = TrainConfig::tiny(13, 300);
    cfg.weighting.linear_regrets = true;
    let lr = run(&cfg, None).0;
    assert_ne!(lr.table().digest(), plain.table().digest());
    for (_, e) in lr.table().iter() {
        assert!(
            e.regret
                .iter()
                .chain(&e.strategy_sum)
                .all(|v| v.is_finite())
        );
    }
    eprintln!(
        "300 it: {} infosets, {} average strategies changed by linear averaging",
        plain.table().len(),
        avg_differs
    );
}

/// Linear averaging still converges on Kuhn (reference implementation).
#[test]
fn checkpoint_linear_kuhn_still_converges() {
    use mako_cfr::toy::cfr::{AverageWeighting, ExternalSamplingMccfr};
    use mako_cfr::toy::eval::exploitability;
    let mut m = ExternalSamplingMccfr::new(9, AverageWeighting::Linear);
    m.run(50_000);
    let e = exploitability(&m.average_strategy());
    eprintln!("Kuhn ES-MCCFR linear 50k: exploitability {e:.5}");
    // Observed 0.0160 (seed 9); uniform play is 0.458.
    assert!(e < 0.03);
}
