//! Acceptance tests: hand evaluator golden cases, invariances,
//! fast-path vs brute-force reference agreement, and category histograms.
//!
//! Release-only exhaustive runs:
//! `cargo test -p pokerbot-core --release --test evaluator -- --include-ignored --nocapture`

use std::time::Instant;

use pokerbot_core::card::{Card, Rank, Suit, parse_cards};
use pokerbot_core::deck::Deck;
use pokerbot_core::eval::{
    HandCategory, HandRank, best_five, evaluate, evaluate5, evaluate7, reference_evaluate,
    reference_evaluate5,
};
use pokerbot_core::rng::PokerbotRng;

use HandCategory::*;

fn cards(s: &str) -> Vec<Card> {
    parse_cards(s).unwrap()
}

fn ev(s: &str) -> HandRank {
    let c = cards(s);
    let r = evaluate(&c);
    assert_eq!(r, reference_evaluate(&c), "fast vs reference on {s}");
    r
}

fn rank_str(r: HandRank) -> String {
    r.ranks().iter().map(|x| x.to_char()).collect()
}

fn random_hand(rng: &mut PokerbotRng, n: usize) -> Vec<Card> {
    Deck::shuffled(rng).deal_n(n).unwrap()
}

// --------------------------------------------------------- golden cases ---

#[test]
fn evaluator_every_category() {
    let cases = [
        ("Ac Kd 9h 7s 3c", HighCard, "AK973"),
        ("Ac Ad 9h 7s 3c", Pair, "AA973"),
        ("Ac Ad 9h 9s 3c", TwoPair, "AA993"),
        ("Ac Ad Ah 9s 3c", Trips, "AAA93"),
        ("5c 6d 7h 8s 9c", Straight, "98765"),
        ("Ac 2d 3h 4s 5c", Straight, "5432A"),
        ("Ac Jc 9c 7c 3c", Flush, "AJ973"),
        ("Kc Kd Kh 4s 4c", FullHouse, "KKK44"),
        ("9c 9d 9h 9s Ac", Quads, "9999A"),
        ("5h 6h 7h 8h 9h", StraightFlush, "98765"),
        ("As Ks Qs Js Ts", StraightFlush, "AKQJT"),
    ];
    for (s, cat, ranks) in cases {
        let r = ev(s);
        assert_eq!(r.category(), cat, "{s}");
        assert_eq!(rank_str(r), ranks, "{s}");
    }
    // Categories strictly increase along the list (one per category).
    let ladder = [
        "Ac Kd 9h 7s 3c",
        "2c 2d 4h 5s 7c",
        "2c 2d 3h 3s 4c",
        "2c 2d 2h 3s 4c",
        "Ac 2d 3h 4s 5c",
        "2c 3c 4c 5c 7c",
        "2c 2d 2h 3s 3c",
        "2c 2d 2h 2s 3c",
        "Ah 2h 3h 4h 5h",
    ];
    for w in ladder.windows(2) {
        assert!(ev(w[0]) < ev(w[1]), "{} < {}", w[0], w[1]);
    }
}

#[test]
fn evaluator_kicker_ordering() {
    assert!(ev("As Ad Kc 9h 3d") > ev("Ac Ah Qc Jh Td"));
    assert!(ev("As Ad Kc 9h 4d") > ev("Ac Ah Kd 9s 3d")); // third kicker
    assert!(ev("Ah Kd 9c 7s 3c") > ev("As Kc 9d 7h 2c")); // fifth high card
    assert_eq!(ev("Ah Kd 9c 7s 3c"), ev("As Kc 9d 7h 3d")); // exact tie
    assert!(ev("Kc Kd Kh 2s 3c") > ev("Qc Qd Qh As Kc"));
    assert!(ev("9c 9d 9h 9s Ac") > ev("9c 9d 9h 9s Kc"));
    assert!(ev("Ac Jc 9c 7c 4c") > ev("Ad Jd 9d 7d 3d"));
}

#[test]
fn evaluator_two_pair_kicker() {
    // Three pairs in 7 cards: best two pairs + best remaining card.
    let r = ev("Ks Kd 9c 9h 4s 4d Ac");
    assert_eq!((r.category(), rank_str(r).as_str()), (TwoPair, "KK99A"));
    let r = ev("Ks Kd 9c 9h 4s 4d 2c");
    assert_eq!(rank_str(r), "KK994"); // kicker comes from the third pair
    assert!(ev("Ks Kd 9c 9h Qd") > ev("Ks Kd 9c 9h Jd"));
    assert!(ev("Ks Kd 9c 9h 2d") > ev("Ks Kd 8c 8h Ad"));
}

#[test]
fn evaluator_full_house_two_trips() {
    let r = ev("Ks Kd Kc 4h 4s 4d 2c");
    assert_eq!((r.category(), rank_str(r).as_str()), (FullHouse, "KKK44"));
    let r = ev("4s 4d 4c Kh Ks Kd 2c");
    assert_eq!(rank_str(r), "KKK44");
    let r = ev("Ks Kd Kc 9h 9s 4d 4c");
    assert_eq!(rank_str(r), "KKK99");
    assert!(ev("3c 3d 3h 2s 2c") > ev("2c 2d 2h As Ac"));
}

#[test]
fn evaluator_wheel_and_straights() {
    let wheel = ev("As 2d 3c 4h 5s");
    let six = ev("2d 3c 4h 5s 6d");
    assert!(wheel < six);
    assert!(wheel > ev("Ac Ad Ah Ks Qc")); // still beats trips
    assert_eq!(wheel.ranks()[0], Rank::Five);
    assert_eq!(wheel.ranks()[4], Rank::Ace);
    // Six-card run: highest straight wins.
    assert_eq!(rank_str(ev("2c 3d 4h 5s 6c 7d Ah")), "76543");
    // Ace-high Broadway.
    assert_eq!(rank_str(ev("Tc Jd Qh Ks Ac 2d 3h")), "AKQJT");
    // No wrap-around straight.
    assert_eq!(ev("Qc Kd Ah 2s 3c").category(), HighCard);
}

#[test]
fn evaluator_wheel_straight_flush() {
    let r = ev("Ah 2h 3h 4h 5h Kd Kc");
    assert_eq!(
        (r.category(), rank_str(r).as_str()),
        (StraightFlush, "5432A")
    );
    assert!(r < ev("2h 3h 4h 5h 6h"));
    assert!(r > ev("9c 9d 9h 9s Ac"));
    assert!(ev("As Ks Qs Js Ts 2d 3c") > ev("Ks Qs Js Ts 9s 2d 3c"));
}

#[test]
fn evaluator_flush_vs_straight_same_seven() {
    // 9-T-J-Q-K straight and a heart flush: flush wins.
    let r = ev("2h 5h 9h Jh Kh Tc Qd");
    assert_eq!((r.category(), rank_str(r).as_str()), (Flush, "KJ952"));
    // Six suited cards: best five of the suit.
    assert_eq!(rank_str(ev("2h 4h 6h 8h Th Qh As")), "QT864");
    // Straight flush inside six suited cards with a higher non-suited card.
    let r = ev("4h 5h 6h 7h 8h 9h Tc");
    assert_eq!(
        (r.category(), rank_str(r).as_str()),
        (StraightFlush, "98765")
    );
    // Straight 8-high plus flush that is not a straight flush.
    let r = ev("4h 5d 6h 7h 8c 2h Kh");
    assert_eq!(r.category(), Flush);
}

#[test]
fn evaluator_board_plays_split() {
    let board = "As Ks Qs Js Ts";
    assert_eq!(ev(&format!("{board} 2c 3d")), ev(&format!("{board} 4h 5c")));
    let board = "5c 6d 7h 8s 9c";
    assert_eq!(ev(&format!("{board} 2d 3h")), ev(&format!("{board} Kd Kh")));
    assert_eq!(ev(&format!("{board} 2d 3h")), ev(&format!("{board} 2s 2h")));
    // Board two pair with ace kicker: neither hole pair matters.
    let board = "Kc Kd 7h 7s Ad";
    assert_eq!(ev(&format!("{board} 2c 3c")), ev(&format!("{board} Qh Jh")));
    // Kicker plays when the board kicker is low.
    let board = "Kc Kd 7h 7s 2d";
    assert!(ev(&format!("{board} Ah 3c")) > ev(&format!("{board} Qh Jh")));
    // Quads on board, kicker decides or ties.
    let board = "9c 9d 9h 9s 3c";
    assert!(ev(&format!("{board} Ac 2d")) > ev(&format!("{board} Kc Qd")));
    assert_eq!(ev(&format!("{board} 2c 2d")), ev(&format!("{board} 3d 2h")));
}

#[test]
fn evaluator_typed_entry_points_agree() {
    let c = cards("Ah 2h 3h 4h 5h Kd Kc");
    let a7: [Card; 7] = c.clone().try_into().unwrap();
    let a5: [Card; 5] = c[..5].try_into().unwrap();
    assert_eq!(evaluate7(&a7), evaluate(&c));
    assert_eq!(evaluate5(&a5), reference_evaluate5(&a5));
    assert_eq!(ev("Kc Kd Kh 4s 4c").to_string(), "Full house (K K K 4 4)");
    // Packed value orders like the struct.
    assert!(ev("Ac Ad 9h 7s 3c").as_u32() < ev("2c 2d 3h 3s 4c").as_u32());
}

#[test]
#[should_panic(expected = "5 to 7 cards")]
fn evaluator_rejects_wrong_length() {
    evaluate(&cards("Ac Kd 9h 7s"));
}

#[test]
fn evaluator_best_five_display() {
    let b = best_five(&cards("2c Ks 4d Kd 4s Kc 4h"));
    let s: Vec<String> = b.iter().map(|c| c.to_string()).collect();
    // First tying subset in input-index order uses 4d and 4s (not 4h).
    assert_eq!(s.join(" "), "Ks Kd Kc 4s 4d");
    let b = best_five(&cards("Kd 3c Ah 5s 2d 4h Kc"));
    let s: Vec<String> = b.iter().map(|c| c.to_string()).collect();
    assert_eq!(s.join(" "), "5s 4h 3c 2d Ah");
    let mut rng = PokerbotRng::seed_from_u64(55);
    for n in 5..=7 {
        for _ in 0..2_000 {
            let h = random_hand(&mut rng, n);
            let b = best_five(&h);
            assert!(b.iter().all(|c| h.contains(c)));
            let r = evaluate(&h);
            assert_eq!(evaluate5(&b), r);
            assert_eq!(b.map(|c| c.rank()), r.ranks());
        }
    }
}

// ------------------------------------------------------- invariances ---

const SUIT_PERMS: [[u8; 4]; 24] = {
    let mut out = [[0u8; 4]; 24];
    let mut n = 0;
    let mut a = 0;
    while a < 4 {
        let mut b = 0;
        while b < 4 {
            let mut c = 0;
            while c < 4 {
                let d = 6u8.saturating_sub(a + b + c);
                if a != b && a != c && b != c && a + b + c <= 6 && d != a && d != b && d != c {
                    out[n] = [a, b, c, d];
                    n += 1;
                }
                c += 1;
            }
            b += 1;
        }
        a += 1;
    }
    out
};

#[test]
fn evaluator_suit_permutation_invariance() {
    let distinct: std::collections::HashSet<[u8; 4]> = SUIT_PERMS.into_iter().collect();
    assert_eq!(distinct.len(), 24);
    let mut rng = PokerbotRng::seed_from_u64(0x5017);
    for _ in 0..3_000 {
        let h = random_hand(&mut rng, 7);
        let base = evaluate(&h);
        for p in SUIT_PERMS {
            let m: Vec<Card> = h
                .iter()
                .map(|c| {
                    Card::new(
                        c.rank(),
                        Suit::from_u8(p[c.suit().as_u8() as usize]).unwrap(),
                    )
                })
                .collect();
            assert_eq!(evaluate(&m), base);
        }
    }
}

#[test]
fn evaluator_order_invariance() {
    let mut rng = PokerbotRng::seed_from_u64(0x0D3E);
    for _ in 0..3_000 {
        let mut h = random_hand(&mut rng, 7);
        let base = evaluate(&h);
        for _ in 0..6 {
            for i in (1..h.len()).rev() {
                let j = rng.gen_range(i as u64 + 1) as usize;
                h.swap(i, j);
            }
            assert_eq!(evaluate(&h), base);
            assert_eq!(best_five(&h).map(|c| c.rank()), base.ranks());
        }
    }
}

// --------------------------------------------- fast vs reference / counts ---

fn random_agreement(seed: u64, n_hands: usize) {
    let mut rng = PokerbotRng::seed_from_u64(seed);
    for _ in 0..n_hands {
        let h = random_hand(&mut rng, 7);
        assert_eq!(evaluate(&h), reference_evaluate(&h), "{h:?}");
    }
    // Also 5- and 6-card inputs.
    for n in [5usize, 6] {
        for _ in 0..n_hands / 10 {
            let h = random_hand(&mut rng, n);
            assert_eq!(evaluate(&h), reference_evaluate(&h), "{h:?}");
        }
    }
}

#[test]
fn evaluator_fast_matches_reference_200k_random_7card() {
    random_agreement(0xE7A1, 200_000);
}

const COUNTS5: [u64; 9] = [
    1_302_540, 1_098_240, 123_552, 54_912, 10_200, 5_108, 3_744, 624, 40,
];
const COUNTS7: [u64; 9] = [
    23_294_460, 58_627_800, 31_433_400, 6_461_620, 6_180_020, 4_047_644, 3_473_184, 224_848, 41_584,
];

#[test]
fn evaluator_all_five_card_hands_histogram_and_reference() {
    let all: Vec<Card> = Card::all().collect();
    let mut hist = [0u64; 9];
    let mut total = 0u64;
    for a in 0..52 {
        for b in a + 1..52 {
            for c in b + 1..52 {
                for d in c + 1..52 {
                    for e in d + 1..52 {
                        let h = [all[a], all[b], all[c], all[d], all[e]];
                        let r = evaluate5(&h);
                        assert_eq!(r, reference_evaluate5(&h));
                        hist[r.category().index()] += 1;
                        total += 1;
                    }
                }
            }
        }
    }
    println!("5-card histogram: {hist:?}");
    assert_eq!(total, 2_598_960);
    assert_eq!(hist, COUNTS5);
}

#[test]
#[ignore = "release-only: 133,784,560 hands"]
fn evaluator_all_seven_card_hands_histogram() {
    let all: Vec<Card> = Card::all().collect();
    let start = Instant::now();
    let mut hist = [0u64; 9];
    let mut h = [all[0]; 7];
    for a in 0..52 {
        h[0] = all[a];
        for b in a + 1..52 {
            h[1] = all[b];
            for c in b + 1..52 {
                h[2] = all[c];
                for d in c + 1..52 {
                    h[3] = all[d];
                    for e in d + 1..52 {
                        h[4] = all[e];
                        for f in e + 1..52 {
                            h[5] = all[f];
                            for &card in &all[f + 1..] {
                                h[6] = card;
                                hist[evaluate7(&h).category().index()] += 1;
                            }
                        }
                    }
                }
            }
        }
    }
    println!("7-card histogram: {hist:?} in {:.2?}", start.elapsed());
    assert_eq!(hist.iter().sum::<u64>(), 133_784_560);
    assert_eq!(hist, COUNTS7);
}

#[test]
#[ignore = "release-only: larger reference cross-check"]
fn evaluator_fast_matches_reference_2m_random_7card() {
    random_agreement(0xB16, 2_000_000);
}

#[test]
#[ignore = "release-only throughput measurement"]
fn evaluator_throughput() {
    let mut rng = PokerbotRng::seed_from_u64(0x7417);
    let n = 2_000_000usize;
    let hands: Vec<[Card; 7]> = (0..n)
        .map(|_| random_hand(&mut rng, 7).try_into().unwrap())
        .collect();
    let mut best = f64::INFINITY;
    let mut checksum = 0u64;
    for _ in 0..5 {
        let t = Instant::now();
        for h in &hands {
            checksum =
                checksum.wrapping_add(u64::from(std::hint::black_box(evaluate7(h)).as_u32()));
        }
        best = best.min(t.elapsed().as_secs_f64());
    }
    println!(
        "evaluate7 throughput: {:.1} M evals/sec (best of 5 over {n} seeded hands; checksum {checksum})",
        n as f64 / best / 1e6
    );
    assert!(checksum > 0);
}
