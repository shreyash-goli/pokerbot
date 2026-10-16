//! M-12: `.mako` strategy artifact format and policy reader.
//! Tests are prefixed `artifact_` so `cargo test -p mako-cfr artifact`
//! selects them.

use mako_cfr::abstraction::key::{InfoKey, fnv1a64};
use mako_cfr::artifact::{
    ARTIFACT_VERSION, ArtifactError, ArtifactMetadata, PROB_SCALE, Policy, StrategyArtifact,
    quantize,
};
use mako_cfr::regret::{FULL_MASK, NUM_SLOTS, mask_of};
use mako_cfr::traverse::{Trainer, TrainerConfig};
use mako_core::rules::{ActionKind, RulesConfig, StackDepth};

const LABEL: &str = "smoke: trained for plumbing, not competition";

fn trained(seed: u64, iters: u64) -> Trainer {
    let mut t = Trainer::new(TrainerConfig::tiny(seed)).unwrap();
    t.run_iterations(iters);
    t
}

fn artifact_of(t: &Trainer) -> StrategyArtifact {
    StrategyArtifact::from_table(t.table(), ArtifactMetadata::from_trainer(t, LABEL)).unwrap()
}

/// Replace the trailing checksum so structural checks are reached.
fn reseal(mut b: Vec<u8>) -> Vec<u8> {
    let n = b.len() - 8;
    let sum = fnv1a64(&b[..n]);
    b[n..].copy_from_slice(&sum.to_le_bytes());
    b
}

#[test]
fn artifact_quantization_is_exact_and_deterministic() {
    let all = FULL_MASK;
    let third = 1.0 / 3.0;
    let m3 = mask_of([ActionKind::Fold, ActionKind::Call, ActionKind::Pot]);
    assert_eq!(
        quantize(&[third, 0.0, third, 0.0, third, 0.0], m3),
        [21845, 0, 21845, 0, 21845, 0]
    );
    let m2 = mask_of([ActionKind::Check, ActionKind::AllIn]);
    // 32767.5 each: the leftover unit goes to the lower slot index.
    assert_eq!(
        quantize(&[0.0, 0.5, 0.0, 0.0, 0.0, 0.5], m2),
        [0, 32768, 0, 0, 0, 32767]
    );
    let u = [1.0 / 6.0; NUM_SLOTS];
    assert_eq!(
        quantize(&u, all),
        [10923, 10923, 10923, 10922, 10922, 10922]
    );
    // Zero mass → uniform over legal slots; illegal mass ignored.
    assert_eq!(
        quantize(&[0.9, 0.0, 0.0, 0.0, 0.0, 0.0], m2),
        [0, 32768, 0, 0, 0, 32767]
    );
    let mut rng = mako_core::rng::MakoRng::seed_from_u64(3);
    for _ in 0..10_000 {
        let mask = (1 + rng.gen_range(63)) as u8;
        let mut p = [0.0; NUM_SLOTS];
        let mut s = 0.0;
        for (i, x) in p.iter_mut().enumerate() {
            if mask & (1 << i) != 0 {
                *x = rng.gen_f64();
                s += *x;
            }
        }
        for x in p.iter_mut() {
            *x /= s.max(1e-300);
        }
        let q = quantize(&p, mask);
        assert_eq!(q.iter().map(|&v| u32::from(v)).sum::<u32>(), 65535);
        for i in 0..NUM_SLOTS {
            if mask & (1 << i) == 0 {
                assert_eq!(q[i], 0);
            } else {
                assert!((f64::from(q[i]) / 65535.0 - p[i]).abs() <= 1.0 / 65535.0 + 1e-12);
            }
        }
    }
}

#[test]
fn artifact_roundtrip_from_trained_table() {
    let t = trained(3, 800);
    let a = artifact_of(&t);
    let bytes = a.to_bytes();
    let back = StrategyArtifact::from_bytes(&bytes).unwrap();
    assert_eq!(back, a);
    assert_eq!(back.to_bytes(), bytes, "re-serialization is byte-identical");
    let policy = Policy::load(&bytes, &t.config().rules, &t.config().abstraction).unwrap();
    assert_eq!(policy.metadata().label, LABEL);
    assert_eq!(policy.metadata().iterations, 800);
    assert_eq!(policy.metadata().seed, 3);
    let mut stored = 0;
    let mut skipped = 0;
    for (k, e) in t.table().iter() {
        let key = InfoKey::from_bytes(k).unwrap();
        let got = policy.lookup(&key);
        if e.strategy_sum.iter().all(|&s| s == 0.0) {
            assert!(got.is_none(), "traverser-only infosets are omitted");
            skipped += 1;
            continue;
        }
        let got = got.expect("stored entry");
        let want = e.average_strategy();
        for i in 0..NUM_SLOTS {
            assert!((got[i] - want[i]).abs() <= 1.0 / f64::from(PROB_SCALE) + 1e-12);
        }
        stored += 1;
    }
    assert_eq!(stored, policy.len());
    eprintln!(
        "stored {stored}, omitted {skipped} (no average mass), {} bytes",
        bytes.len()
    );
    // Unseen key: a well-formed key for a decision that never occurs (the
    // big blind never acts first preflop), so it cannot be in the table.
    let unseen = InfoKey::from_bytes(&[1, 0, 1, 0, 0, 0]).unwrap();
    assert!(policy.lookup(&unseen).is_none());
    assert!(policy.lookup_bytes(b"not a key").is_none());
}

#[test]
fn artifact_same_seed_byte_identical() {
    let a = artifact_of(&trained(9, 300)).to_bytes();
    let b = artifact_of(&trained(9, 300)).to_bytes();
    let c = artifact_of(&trained(10, 300)).to_bytes();
    assert_eq!(a, b);
    assert_eq!(fnv1a64(&a), fnv1a64(&b));
    assert_ne!(a, c);
}

#[test]
fn artifact_rejects_malformed_bytes() {
    let good = artifact_of(&trained(4, 200)).to_bytes();
    assert!(StrategyArtifact::from_bytes(&good).is_ok());
    let err = |b: &[u8]| StrategyArtifact::from_bytes(b).unwrap_err();

    let mut b = good.clone();
    b[1] = b'X';
    assert_eq!(err(&b), ArtifactError::BadMagic);

    let mut b = good.clone();
    b[8..10].copy_from_slice(&(ARTIFACT_VERSION + 1).to_le_bytes());
    assert_eq!(
        err(&b),
        ArtifactError::UnsupportedVersion(ARTIFACT_VERSION + 1)
    );

    let mut b = good.clone();
    let mid = b.len() / 2;
    b[mid] ^= 1;
    assert_eq!(err(&b), ArtifactError::ChecksumMismatch);

    // Truncation, raw and with a recomputed checksum.
    assert_eq!(err(&good[..5]), ArtifactError::Truncated);
    assert!(StrategyArtifact::from_bytes(&good[..good.len() - 3]).is_err());
    let mut cut = good[..good.len() - 8 - 5].to_vec();
    cut.extend_from_slice(&[0; 8]);
    assert_eq!(err(&reseal(cut)), ArtifactError::Truncated);

    // Trailing garbage before the checksum.
    let mut long = good[..good.len() - 8].to_vec();
    long.extend_from_slice(&[7, 7, 7]);
    long.extend_from_slice(&[0; 8]);
    assert_eq!(err(&reseal(long)), ArtifactError::TrailingBytes);

    // Unsupported rules spec ("GAME_RULES v1" → "v2"), resealed.
    let mut b = good.clone();
    let pos = 11 + "GAME_RULES v".len();
    assert_eq!(b[pos], b'1');
    b[pos] = b'2';
    assert!(matches!(
        err(&reseal(b)),
        ArtifactError::UnsupportedRulesSpec(_)
    ));
}

#[test]
fn artifact_rejects_invalid_entries() {
    let base = artifact_of(&trained(4, 200));
    assert!(base.entries.len() > 10);
    let check = |a: &StrategyArtifact| StrategyArtifact::from_bytes(&a.to_bytes()).unwrap_err();

    let mut a = base.clone();
    let dup = a.entries[3].clone();
    a.entries.insert(4, dup);
    assert_eq!(check(&a), ArtifactError::DuplicateKey);

    let mut a = base.clone();
    a.entries.swap(2, 3);
    assert_eq!(check(&a), ArtifactError::UnsortedKeys);

    let mut a = base.clone();
    let i = a.entries[5].q.iter().position(|&v| v > 0).unwrap();
    a.entries[5].q[i] -= 1;
    assert!(matches!(check(&a), ArtifactError::Invalid(m) if m.contains("sum")));

    // Empty mask.
    let mut a = base.clone();
    a.entries[5].mask = 0;
    a.entries[5].q = [0; NUM_SLOTS];
    assert!(matches!(check(&a), ArtifactError::Invalid(m) if m.contains("mask")));

    // Mass on a slot that is illegal at this decision: add the slot to the
    // mask (so it is serialized) and move mass onto it.
    let mut a = base.clone();
    let e = &mut a.entries[5];
    let illegal = (0..NUM_SLOTS).find(|&s| e.mask & (1 << s) == 0).unwrap();
    let from = e.q.iter().position(|&v| v > 0).unwrap();
    e.mask |= 1 << illegal;
    e.q[from] -= 1;
    e.q[illegal] = 1;
    assert!(matches!(check(&a), ArtifactError::Invalid(m) if m.contains("legal set")));

    // In-memory illegal-slot mass is also rejected by validate().
    let mut a = base.clone();
    let e = &mut a.entries[5];
    let illegal = (0..NUM_SLOTS).find(|&s| e.mask & (1 << s) == 0).unwrap();
    e.q[illegal] = 1;
    assert!(matches!(a.validate(), Err(ArtifactError::Invalid(m)) if m.contains("illegal-slot")));

    // Key that is not a valid InfoKey.
    let mut a = base.clone();
    a.entries[0].key = vec![9, 9, 9];
    assert!(matches!(check(&a), ArtifactError::Invalid(_)));
}

#[test]
fn artifact_wrong_depth_and_abstraction_are_rejected() {
    let t = trained(6, 150);
    let bytes = artifact_of(&t).to_bytes();
    let rules = t.config().rules;
    let abs = t.config().abstraction;
    assert!(Policy::load(&bytes, &rules, &abs).is_ok());
    for depth in [StackDepth::Bb50, StackDepth::Bb100] {
        let e = Policy::load(&bytes, &RulesConfig::standard(depth), &abs).unwrap_err();
        assert!(matches!(e, ArtifactError::RulesMismatch { .. }));
        eprintln!("{e}");
    }
    let mut other = abs;
    other.river_buckets += 1;
    assert!(matches!(
        Policy::load(&bytes, &rules, &other).unwrap_err(),
        ArtifactError::AbstractionMismatch { .. }
    ));
    let mut other = abs;
    other.seed ^= 1;
    assert!(Policy::load(&bytes, &rules, &other).is_err());
}

#[test]
#[ignore = "size report; run with --release --include-ignored"]
fn artifact_size_report_tiny_10k() {
    let t = trained(1, 10_000);
    let a = artifact_of(&t);
    let bytes = a.to_bytes();
    eprintln!(
        "tiny 20BB 10k iterations: table {} infosets, artifact {} entries, {} bytes ({:.1} KiB), checksum {:#018x}",
        t.table().len(),
        a.entries.len(),
        bytes.len(),
        bytes.len() as f64 / 1024.0,
        a.checksum()
    );
    let start = std::time::Instant::now();
    let p = Policy::load(&bytes, &t.config().rules, &t.config().abstraction).unwrap();
    eprintln!(
        "load+validate: {:?} for {} entries",
        start.elapsed(),
        p.len()
    );
}

#[test]
fn artifact_header_accessors_match_training_config() {
    use mako_cfr::artifact::{read_metadata, stored_abstraction, stored_rules};
    let t = trained(12, 120);
    let bytes = artifact_of(&t).to_bytes();
    let cfg = t.config();
    assert_eq!(stored_abstraction(&bytes).unwrap(), cfg.abstraction);
    assert_eq!(stored_rules(&bytes).unwrap(), cfg.rules);
    let m = read_metadata(&bytes).unwrap();
    assert_eq!(m, StrategyArtifact::from_bytes(&bytes).unwrap().metadata);
    assert_eq!((m.seed, m.iterations), (12, 120));
    assert_eq!(m.label, LABEL);
    assert!(m.trainer_id.starts_with("mako-cfr "));
    assert_eq!((m.update, m.weighting), (cfg.update, cfg.weighting));
    let a = StrategyArtifact::from_bytes(&bytes).unwrap();
    assert_eq!((a.rules(), a.abstraction()), (&cfg.rules, &cfg.abstraction));
    let p = Policy::load(&bytes, &cfg.rules, &cfg.abstraction).unwrap();
    assert_eq!((p.rules(), p.abstraction()), (&cfg.rules, &cfg.abstraction));
}

#[test]
fn artifact_header_accessors_reject_malformed_headers() {
    use mako_cfr::artifact::{read_metadata, stored_abstraction, stored_rules};
    let good = artifact_of(&trained(12, 60)).to_bytes();
    let mut magic = good.clone();
    magic[0] = b'Z';
    assert_eq!(
        stored_abstraction(&magic).unwrap_err(),
        ArtifactError::BadMagic
    );
    let mut ver = good.clone();
    ver[8] = 9;
    assert_eq!(
        stored_rules(&ver).unwrap_err(),
        ArtifactError::UnsupportedVersion(9)
    );
    let mut flipped = good.clone();
    flipped[30] ^= 0x10; // inside the rules/abstraction header
    assert_eq!(
        read_metadata(&flipped).unwrap_err(),
        ArtifactError::ChecksumMismatch
    );
    assert_eq!(
        read_metadata(&good[..6]).unwrap_err(),
        ArtifactError::Truncated
    );
    // Header cut short but resealed: structurally truncated.
    let mut short = good[..40].to_vec();
    short.extend_from_slice(&[0; 8]);
    assert_eq!(
        read_metadata(&reseal(short)).unwrap_err(),
        ArtifactError::Truncated
    );
    // Corrupted abstraction bytes with a valid checksum.
    let mut abs = good.clone();
    let off = 11 + "GAME_RULES v1".len() + 12; // start of AbstractionConfig bytes
    assert_eq!(&abs[off..off + 4], b"MKEQ");
    abs[off] = b'X';
    assert!(matches!(
        stored_abstraction(&reseal(abs)).unwrap_err(),
        ArtifactError::Invalid(_)
    ));
}
