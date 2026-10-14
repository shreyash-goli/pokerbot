//! M-08: canonicalization, equity, postflop buckets, cache.
//! Tests are prefixed `equity_` so `cargo test -p mako-cfr equity` selects them.

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use mako_cfr::abstraction::key::{CardAbstraction, InfoKey};
use mako_cfr::abstraction::postflop::{
    AbstractionConfig, CONFIG_BYTES, ConfigError, EquityAbstraction, postflop_equity,
};
use mako_cfr::abstraction::preflop::PreflopClass;
use mako_cfr::equity::{
    Equity, canonical_form, canonical_key, exact_equity, permute_suit, sampled_equity,
};
use mako_core::action::Seat;
use mako_core::card::{Card, parse_cards};
use mako_core::deck::Deck;
use mako_core::eval::reference_evaluate;
use mako_core::rng::MakoRng;
use mako_core::rules::{ActionKind, RulesConfig, StackDepth, Street};
use mako_core::state::{HandConfig, HandState};

fn cards(s: &str) -> Vec<Card> {
    parse_cards(s).unwrap()
}

fn hole(s: &str) -> [Card; 2] {
    let v = cards(s);
    [v[0], v[1]]
}

/// Random distinct `(hole, board)` with `board_len` board cards.
fn random_situation(rng: &mut MakoRng, board_len: usize) -> ([Card; 2], Vec<Card>) {
    let d = Deck::from_seed(rng.next_u64());
    let o = d.order();
    ([o[0], o[1]], o[2..2 + board_len].to_vec())
}

fn all_perms() -> Vec<[u8; 4]> {
    let mut v = Vec::new();
    for a in 0..4u8 {
        for b in 0..4u8 {
            for c in 0..4u8 {
                for d in 0..4u8 {
                    let p = [a, b, c, d];
                    let mut s = p;
                    s.sort_unstable();
                    if s == [0, 1, 2, 3] {
                        v.push(p);
                    }
                }
            }
        }
    }
    assert_eq!(v.len(), 24);
    v
}

/// Independent oracle: slow reference evaluator, full enumeration.
fn oracle_equity(h: [Card; 2], board: &[Card]) -> Equity {
    let used: HashSet<Card> = h.iter().chain(board).copied().collect();
    let rest: Vec<Card> = Card::all().filter(|c| !used.contains(c)).collect();
    let mut points = 0;
    let mut trials = 0;
    let mut tally = |b: &[Card], excl: &[Card]| {
        let mut hero = h.to_vec();
        hero.extend_from_slice(b);
        let hr = reference_evaluate(&hero);
        let r: Vec<Card> = rest.iter().copied().filter(|c| !excl.contains(c)).collect();
        for i in 0..r.len() {
            for j in i + 1..r.len() {
                let mut opp = vec![r[i], r[j]];
                opp.extend_from_slice(b);
                let or = reference_evaluate(&opp);
                points += match hr.cmp(&or) {
                    std::cmp::Ordering::Greater => 2,
                    std::cmp::Ordering::Equal => 1,
                    std::cmp::Ordering::Less => 0,
                };
                trials += 1;
            }
        }
    };
    if board.len() == 5 {
        tally(board, &[]);
    } else {
        assert_eq!(board.len(), 4);
        for &rv in &rest {
            let mut b = board.to_vec();
            b.push(rv);
            tally(&b, &[rv]);
        }
    }
    Equity { points, trials }
}

// ------------------------------------------------------------------ oracle

#[test]
fn equity_river_oracle_hand_picked() {
    // The nuts: royal flush in hand.
    let e = exact_equity(hole("As Ks"), &cards("Qs Js Ts 2c 3d"));
    assert_eq!(
        e,
        Equity {
            points: 1980,
            trials: 990
        }
    );
    assert_eq!(e.value(), 1.0);
    // Board plays (royal on board): every opponent ties.
    let e = exact_equity(hole("2c 3d"), &cards("As Ks Qs Js Ts"));
    assert_eq!(
        e,
        Equity {
            points: 990,
            trials: 990
        }
    );
    assert_eq!(e.value(), 0.5);
    // Quad aces on board, hero K kicker: opponents holding a K tie
    // (990 − C(42,2) = 129 hands), all others lose.
    let e = exact_equity(hole("Kd 3h"), &cards("Ac Ad Ah As 2c"));
    assert_eq!(
        e,
        Equity {
            points: 2 * (990 - 129) + 129,
            trials: 990
        }
    );
    // Straight vs any pair: an opponent pair can never beat a made straight on
    // a rainbow, unpaired board unless it makes a higher straight.
    let h = hole("9h 8d");
    let b = cards("Tc Jd Qs 2h 3c");
    let e = exact_equity(h, &b);
    assert_eq!(e, oracle_equity(h, &b));
    // Losers: AK (16 combos, broadway), K9 (4 kings × 3 nines = 12, K-high).
    // Ties: 98 (3 nines × 3 eights = 9, same straight). Everything else loses.
    let lose = 16 + 12;
    let tie = 9;
    assert_eq!(e.points, 2 * (990 - lose - tie) + tie);
}

#[test]
fn equity_river_matches_reference_oracle_random() {
    let mut rng = MakoRng::seed_from_u64(11);
    for _ in 0..20 {
        let (h, b) = random_situation(&mut rng, 5);
        assert_eq!(exact_equity(h, &b), oracle_equity(h, &b));
    }
}

#[test]
fn equity_turn_matches_brute_force() {
    let cases = [("Ah Kh", "Qh 7h 2c 9d"), ("7c 2d", "Ks 8h 3d 3c")];
    for (h, b) in cases {
        let (h, b) = (hole(h), cards(b));
        let e = exact_equity(h, &b);
        assert_eq!(e.trials, 46 * 990);
        assert_eq!(e, oracle_equity(h, &b), "{h:?} {b:?}");
    }
    // Structural: exact turn equals the sum of exact rivers.
    let mut rng = MakoRng::seed_from_u64(3);
    for _ in 0..3 {
        let (h, b) = random_situation(&mut rng, 4);
        let used: HashSet<Card> = h.iter().chain(&b).copied().collect();
        let mut sum = Equity {
            points: 0,
            trials: 0,
        };
        for r in Card::all().filter(|c| !used.contains(c)) {
            let mut b5 = b.clone();
            b5.push(r);
            let e = exact_equity(h, &b5);
            sum.points += e.points;
            sum.trials += e.trials;
        }
        assert_eq!(exact_equity(h, &b), sum);
    }
}

#[test]
fn equity_sampled_turn_within_tolerance_of_exact() {
    let mut rng = MakoRng::seed_from_u64(5);
    for _ in 0..4 {
        let (h, b) = random_situation(&mut rng, 4);
        let exact = exact_equity(h, &b).value();
        let mc = sampled_equity(h, &b, 4_000, &mut rng).value();
        // sigma <= 0.5/sqrt(4000) ≈ 0.0079; allow 5 sigma.
        assert!((exact - mc).abs() < 0.04, "exact {exact} mc {mc}");
    }
}

/// Exact flop (~1.07M showdowns per case) vs seeded MC. Slow in debug.
#[test]
#[ignore = "exact flop enumeration; run with --release --include-ignored"]
fn equity_flop_mc_within_tolerance_of_exact() {
    let cases = [
        ("Ah Ad", "Kc 8d 3s"),
        ("7c 2d", "Kc 8d 3s"),
        ("9h 8h", "Th 7h 2c"),
    ];
    let cfg = AbstractionConfig {
        flop_samples: 4_000,
        ..AbstractionConfig::default()
    };
    for (h, b) in cases {
        let (h, b) = (hole(h), cards(b));
        let t = Instant::now();
        let exact = exact_equity(h, &b);
        let dt = t.elapsed();
        assert_eq!(exact.trials, 1081 * 990);
        let mc = postflop_equity(&cfg, h, &b);
        eprintln!(
            "flop {h:?} {b:?}: exact {:.5} ({dt:?}), mc4000 {:.5}",
            exact.value(),
            mc.value()
        );
        assert!((exact.value() - mc.value()).abs() < 0.04);
    }
}

// ------------------------------------------------------------------ canonicalization

#[test]
fn equity_suit_isomorphism_identical_key_equity_bucket() {
    let mut rng = MakoRng::seed_from_u64(21);
    let abs = EquityAbstraction::new(AbstractionConfig::default()).unwrap();
    for board_len in [0usize, 3, 4, 5] {
        for _ in 0..4 {
            let (h, b) = random_situation(&mut rng, board_len);
            let street = Street::ALL[[0, 0, 0, 1, 2, 3][board_len]];
            let k0 = canonical_key(h, &b);
            let b0 = abs.bucket(street, h, &b).unwrap();
            let e0 = (board_len > 0).then(|| abs.equity(h, &b));
            for p in all_perms() {
                let ph = h.map(|c| permute_suit(c, p));
                let pb: Vec<Card> = b.iter().map(|&c| permute_suit(c, p)).collect();
                assert_eq!(canonical_key(ph, &pb), k0);
                assert_eq!(abs.bucket(street, ph, &pb).unwrap(), b0);
                if let Some(e0) = e0 {
                    assert_eq!(abs.equity(ph, &pb), e0);
                }
            }
        }
    }
}

#[test]
fn equity_board_and_hole_order_invariance() {
    let mut rng = MakoRng::seed_from_u64(8);
    for _ in 0..200 {
        let (h, mut b) = random_situation(&mut rng, 5);
        let k = canonical_key(h, &b);
        b.reverse();
        assert_eq!(canonical_key([h[1], h[0]], &b), k);
        b.swap(0, 3);
        assert_eq!(canonical_key(h, &b), k);
    }
    // Hole vs board is distinguished.
    let a = canonical_key(hole("Ah Kd"), &cards("2c 7s 9h"));
    let b = canonical_key(hole("2c Kd"), &cards("Ah 7s 9h"));
    assert_ne!(a, b);
}

#[test]
fn equity_preflop_canonical_classes_are_169() {
    let all: Vec<Card> = Card::all().collect();
    let mut keys: HashMap<u64, PreflopClass> = HashMap::new();
    for i in 0..52 {
        for j in i + 1..52 {
            let h = [all[i], all[j]];
            let class = PreflopClass::of(h);
            if let Some(prev) = keys.insert(canonical_key(h, &[]), class) {
                assert_eq!(prev, class);
            }
        }
    }
    assert_eq!(keys.len(), 169);
}

/// Different non-isomorphic situations never share a key: the key is an
/// injective packing of the canonical form (no hashing, so zero collisions).
#[test]
fn equity_canonical_keys_do_not_collide() {
    let mut rng = MakoRng::seed_from_u64(99);
    let mut by_key = HashMap::new();
    for i in 0..100_000 {
        let (h, b) = random_situation(&mut rng, [3, 4, 5][i % 3]);
        let f = canonical_form(h, &b);
        // The canonical form round-trips through its own cards.
        assert_eq!(canonical_form(f.hole(), &f.board()), f);
        if let Some(prev) = by_key.insert(f.key(), f) {
            assert_eq!(prev, f, "key collision");
        }
    }
    eprintln!("distinct canonical forms in sample: {}", by_key.len());
}

/// Exhaustive check against the published count of suit-isomorphic flop
/// situations (hole + flop, each unordered): 1,286,792 (Waugh, "A Fast and
/// Optimal Hand Isomorphism Algorithm", 2013).
#[test]
#[ignore = "25.9M canonicalizations; run with --release --include-ignored"]
fn equity_flop_isomorphism_count_matches_literature() {
    let all: Vec<Card> = Card::all().collect();
    let mut keys = HashSet::new();
    let t = Instant::now();
    for i in 0..52 {
        for j in i + 1..52 {
            let rest: Vec<Card> = all
                .iter()
                .copied()
                .filter(|&c| c != all[i] && c != all[j])
                .collect();
            for a in 0..50 {
                for b in a + 1..50 {
                    for c in b + 1..50 {
                        keys.insert(canonical_key(
                            [all[i], all[j]],
                            &[rest[a], rest[b], rest[c]],
                        ));
                    }
                }
            }
        }
    }
    eprintln!(
        "flop canonical classes: {} in {:?}",
        keys.len(),
        t.elapsed()
    );
    assert_eq!(keys.len(), 1_286_792);
}

// ------------------------------------------------------------------ buckets / config

#[test]
fn equity_buckets_in_range_all_streets() {
    let cfg = AbstractionConfig {
        flop_samples: 200,
        flop_buckets: 5,
        turn_buckets: 7,
        river_buckets: 11,
        ..AbstractionConfig::default()
    };
    let abs = EquityAbstraction::new(cfg).unwrap();
    let mut rng = MakoRng::seed_from_u64(4);
    let mut seen: HashMap<Street, HashSet<u16>> = HashMap::new();
    for i in 0..300 {
        let (street, len) = [
            (Street::Preflop, 0),
            (Street::Flop, 3),
            (Street::Turn, 4),
            (Street::River, 5),
        ][i % 4];
        if street == Street::Turn && i % 8 != 3 {
            continue; // exact turn is slower in debug
        }
        let (h, b) = random_situation(&mut rng, len);
        let bucket = abs.bucket(street, h, &b).unwrap();
        assert!(bucket < abs.num_buckets(street), "{street:?} {bucket}");
        seen.entry(street).or_default().insert(bucket);
    }
    assert_eq!(abs.num_buckets(Street::Preflop), 169);
    assert_eq!(abs.num_buckets(Street::River), 11);
    assert!(
        seen[&Street::River].len() >= 6,
        "river buckets poorly spread"
    );
    // Wrong board length is an error, not a panic.
    assert!(
        abs.bucket(Street::Flop, hole("Ah Kh"), &cards("2c 3c"))
            .is_err()
    );
    // Bucket edge cases.
    assert_eq!(
        Equity {
            points: 2,
            trials: 1
        }
        .bucket(8),
        7
    );
    assert_eq!(
        Equity {
            points: 0,
            trials: 1
        }
        .bucket(8),
        0
    );
    assert_eq!(
        Equity {
            points: 1,
            trials: 1
        }
        .bucket(8),
        4
    );
}

#[test]
fn equity_bucket_ordering_sanity() {
    let abs = EquityAbstraction::new(AbstractionConfig::default()).unwrap();
    let flop = cards("Kc 8d 3s");
    let aa = abs.bucket(Street::Flop, hole("Ah Ad"), &flop).unwrap();
    let junk = abs.bucket(Street::Flop, hole("7c 2d"), &flop).unwrap();
    assert!(aa > junk, "AA {aa} vs 72o {junk}");
    let river = cards("Kc 8d 3s 5h Jc");
    assert!(
        abs.bucket(Street::River, hole("Ah Ad"), &river).unwrap()
            >= abs.bucket(Street::River, hole("7c 2d"), &river).unwrap()
    );
}

#[test]
fn equity_determinism_and_seed_affects_only_sampled_streets() {
    let a = EquityAbstraction::new(AbstractionConfig::default()).unwrap();
    let b = EquityAbstraction::new(AbstractionConfig::default()).unwrap();
    let c = EquityAbstraction::new(AbstractionConfig {
        seed: 12345,
        ..AbstractionConfig::default()
    })
    .unwrap();
    let mut rng = MakoRng::seed_from_u64(31);
    let mut flop_equity_differs = 0;
    for _ in 0..60 {
        let (h, f) = random_situation(&mut rng, 3);
        assert_eq!(a.equity(h, &f), b.equity(h, &f));
        if a.equity(h, &f) != c.equity(h, &f) {
            flop_equity_differs += 1;
        }
        let (h, r) = random_situation(&mut rng, 5);
        assert_eq!(a.equity(h, &r), c.equity(h, &r), "river must be exact");
    }
    let (h, t) = random_situation(&mut rng, 4);
    assert_eq!(a.equity(h, &t), c.equity(h, &t), "turn is exact by default");
    assert!(
        flop_equity_differs > 30,
        "seed should change MC flop equity"
    );
    // Evaluation order does not matter (no shared RNG stream).
    let (h, f) = random_situation(&mut rng, 3);
    let first = a.equity(h, &f);
    for _ in 0..5 {
        let (h2, f2) = random_situation(&mut rng, 3);
        a.equity(h2, &f2);
    }
    assert_eq!(a.equity(h, &f), first);
}

#[test]
fn equity_config_serialization_and_validation() {
    let cfg = AbstractionConfig::default();
    let bytes = cfg.to_bytes();
    assert_eq!(bytes.len(), CONFIG_BYTES);
    assert_eq!(AbstractionConfig::from_bytes(&bytes).unwrap(), cfg);
    eprintln!("default fingerprint {:#018x}", cfg.fingerprint());
    assert_eq!(cfg.fingerprint(), DEFAULT_FINGERPRINT);
    let other = AbstractionConfig {
        flop_samples: 2_000,
        ..cfg
    };
    assert_ne!(other.fingerprint(), cfg.fingerprint());
    let mut bad = bytes;
    bad[0] = b'X';
    assert!(AbstractionConfig::from_bytes(&bad).is_err());
    assert!(AbstractionConfig::from_bytes(&bytes[..27]).is_err());
    assert_eq!(
        EquityAbstraction::new(AbstractionConfig {
            river_buckets: 0,
            ..cfg
        })
        .err(),
        Some(ConfigError("bucket counts must be at least 1"))
    );
}

// Pinned: changing defaults or the byte layout must be deliberate.
const DEFAULT_FINGERPRINT: u64 = 0x4a92_1145_2c5b_609b;

// ------------------------------------------------------------------ cache

#[test]
fn equity_cache_equivalent_and_bounded() {
    let cfg = AbstractionConfig {
        flop_samples: 300,
        ..AbstractionConfig::default()
    };
    let plain = EquityAbstraction::new(cfg).unwrap();
    let cached = EquityAbstraction::with_cache(cfg, 40).unwrap();
    let mut rng = MakoRng::seed_from_u64(17);
    let pool: Vec<([Card; 2], Vec<Card>)> = (0..80)
        .map(|i| random_situation(&mut rng, if i % 2 == 0 { 3 } else { 5 }))
        .collect();
    for i in 0..600 {
        let (h, b) = &pool[rng.gen_range(pool.len() as u64) as usize];
        let street = if b.len() == 3 {
            Street::Flop
        } else {
            Street::River
        };
        assert_eq!(
            cached.bucket(street, *h, b).unwrap(),
            plain.bucket(street, *h, b).unwrap()
        );
        assert_eq!(cached.equity(*h, b), plain.equity(*h, b));
        let s = cached.cache_stats().unwrap();
        assert!(s.len <= s.capacity, "cache exceeded capacity at {i}: {s:?}");
    }
    let s = cached.cache_stats().unwrap();
    eprintln!("cache stats: {s:?}");
    assert!(s.hits > 0 && s.misses > 0 && s.clears > 0);
    assert_eq!(s.capacity, 40);
    assert!(plain.cache_stats().is_none());
}

// ------------------------------------------------------------------ integration

#[test]
fn equity_abstraction_builds_keys_on_every_street() {
    let abs = EquityAbstraction::with_cache(AbstractionConfig::default(), 1024).unwrap();
    let cfg = HandConfig {
        rules: RulesConfig::standard(StackDepth::Bb100),
        button: Seat::Zero,
    };
    let mut hand = HandState::from_seed(cfg, 42).unwrap();
    let mut streets = HashSet::new();
    while !hand.is_terminal() {
        let seat = hand.to_act().unwrap();
        let k = InfoKey::from_observation(&hand.observation(seat), &abs).unwrap();
        assert!(k.bucket() < abs.num_buckets(k.street()));
        streets.insert(k.street());
        let legal = hand.legal_actions();
        let kind = if legal.iter().any(|a| a.kind == ActionKind::Check) {
            ActionKind::Check
        } else {
            ActionKind::Call
        };
        hand.apply(kind).unwrap();
    }
    assert_eq!(streets.len(), 4);
}

// ------------------------------------------------------------------ timing

#[test]
#[ignore = "timing report; run with --release --include-ignored"]
fn equity_timing_report() {
    let cfg = AbstractionConfig::default();
    let mut rng = MakoRng::seed_from_u64(1234);
    for (street, len, n) in [
        (Street::Flop, 3, 300),
        (Street::Turn, 4, 100),
        (Street::River, 5, 1000),
    ] {
        let sits: Vec<_> = (0..n).map(|_| random_situation(&mut rng, len)).collect();
        let plain = EquityAbstraction::new(cfg).unwrap();
        let t = Instant::now();
        for (h, b) in &sits {
            std::hint::black_box(plain.bucket(street, *h, b).unwrap());
        }
        let uncached = t.elapsed() / n;
        let cached = EquityAbstraction::with_cache(cfg, 1 << 16).unwrap();
        for (h, b) in &sits {
            cached.bucket(street, *h, b).unwrap();
        }
        let t = Instant::now();
        for (h, b) in &sits {
            std::hint::black_box(cached.bucket(street, *h, b).unwrap());
        }
        let hit = t.elapsed() / n;
        eprintln!("{street:?}: uncached {uncached:?}/bucket, cache hit {hit:?}/bucket");
    }
}
