//! M-02 acceptance tests: cards, card sets, deterministic RNG and shuffle.
//!
//! Golden values below were produced by the Rust implementation and
//! cross-checked with an independently written Python 3 reference of
//! SplitMix64 / xoshiro256** / Lemire / Fisher–Yates (identical outputs).

use pokerbot_core::card::{
    Card, CardSet, NUM_CARDS, Rank, Suit, find_duplicate, has_duplicates, parse_cards,
};
use pokerbot_core::deck::Deck;
use pokerbot_core::rng::{PokerbotRng, splitmix64_next};

// ----------------------------------------------------------------- cards ---

#[test]
fn card_index_layout() {
    let c = Card::new(Rank::Ace, Suit::Spades);
    assert_eq!(c.index(), 51);
    assert_eq!(Card::new(Rank::Two, Suit::Clubs).index(), 0);
    assert_eq!(Card::new(Rank::Ten, Suit::Diamonds).index(), 8 * 4 + 1);
    for i in 0..52u8 {
        let c = Card::from_index(i).unwrap();
        assert_eq!(c.rank().as_u8() * 4 + c.suit().as_u8(), i);
        assert_eq!(Card::new(c.rank(), c.suit()), c);
    }
}

#[test]
fn string_roundtrip_all_52() {
    let mut seen = std::collections::HashSet::new();
    for c in Card::all() {
        let s = c.to_string();
        assert_eq!(s.len(), 2);
        assert!(seen.insert(s.clone()));
        assert_eq!(s.parse::<Card>().unwrap(), c);
    }
    assert_eq!(seen.len(), 52);
    assert_eq!("As".parse::<Card>().unwrap().to_string(), "As");
    assert_eq!("Td".parse::<Card>().unwrap().to_string(), "Td");
    assert_eq!("2c".parse::<Card>().unwrap().to_string(), "2c");
}

#[test]
fn parse_is_case_insensitive() {
    let a = Card::new(Rank::Ace, Suit::Spades);
    for s in ["As", "AS", "as", "aS"] {
        assert_eq!(s.parse::<Card>().unwrap(), a);
    }
    assert_eq!(
        "tH".parse::<Card>().unwrap(),
        Card::new(Rank::Ten, Suit::Hearts)
    );
}

#[test]
fn rejects_invalid_strings_and_indices() {
    for bad in [
        "", "A", "Asd", "1s", "0c", "10h", "Ax", "sA", " As", "As ", "A s", "??", "Ÿs", "AŸ",
    ] {
        assert!(bad.parse::<Card>().is_err(), "accepted {bad:?}");
    }
    for i in 52..=255u8 {
        assert!(Card::from_index(i).is_none());
    }
    assert!(Rank::from_u8(13).is_none());
    assert!(Suit::from_u8(4).is_none());
}

#[test]
fn parse_cards_and_duplicates() {
    let v = parse_cards("As Kd  7c").unwrap();
    assert_eq!(v.len(), 3);
    assert!(parse_cards("As Kd As").is_err());
    assert!(parse_cards("As Zz").is_err());
    assert!(has_duplicates(&[v[0], v[1], v[0]]));
    assert!(!has_duplicates(&v));
    assert_eq!(find_duplicate(&[v[2], v[1], v[2]]), Some(v[2]));
}

#[test]
fn cardset_operations() {
    let mut s = CardSet::new();
    assert!(s.is_empty());
    let a: Card = "As".parse().unwrap();
    let k: Card = "Kd".parse().unwrap();
    assert!(s.insert(a));
    assert!(!s.insert(a), "second insert reports duplicate");
    assert!(s.insert(k));
    assert_eq!(s.len(), 2);
    assert!(s.contains(a) && s.contains(k));
    assert_eq!(s.iter().collect::<Vec<_>>(), vec![k, a]); // ascending index
    assert!(s.remove(a));
    assert!(!s.remove(a));
    assert_eq!(s.len(), 1);
    assert_eq!(CardSet::full().len(), 52);
    assert_eq!(CardSet::full().iter().count(), 52);
    assert!(CardSet::from_bits(1 << 52).is_none());
    assert!(CardSet::try_from_cards(&[a, k, a]).is_err());
    let t = CardSet::try_from_cards(&[a]).unwrap();
    assert!(t.is_disjoint(s));
    assert_eq!(t.union(s).len(), 2);
    assert_eq!(CardSet::full().difference(t).len(), 51);
    assert_eq!(t.intersection(s).len(), 0);
}

// ------------------------------------------------------------------- rng ---

#[test]
fn splitmix64_published_vector_seed0() {
    // Published SplitMix64 reference: first output for seed 0 is
    // 0xE220A8397B1DCDAF (also reproduced by the Python cross-check).
    let mut st = 0u64;
    assert_eq!(splitmix64_next(&mut st), 0xE220_A839_7B1D_CDAF);
    assert_eq!(splitmix64_next(&mut st), 0x6E78_9E6A_A1B9_65F4);
    assert_eq!(splitmix64_next(&mut st), 0x06C4_5D18_8009_454F);
}

#[test]
fn xoshiro_known_answers() {
    let r = PokerbotRng::seed_from_u64(0);
    assert_eq!(
        r.state(),
        [
            0xE220_A839_7B1D_CDAF,
            0x6E78_9E6A_A1B9_65F4,
            0x06C4_5D18_8009_454F,
            0xF88B_B8A8_724C_81EC
        ]
    );
    let mut r = r;
    let got: Vec<u64> = (0..4).map(|_| r.next_u64()).collect();
    assert_eq!(
        got,
        [
            0x99EC_5F36_CB75_F2B4,
            0xBF6E_1F78_4956_452A,
            0x1A5F_849D_4933_E6E0,
            0x6AA5_94F1_262D_2D2C
        ]
    );
    let mut r = PokerbotRng::seed_from_u64(7);
    let got: Vec<u64> = (0..4).map(|_| r.next_u64()).collect();
    assert_eq!(
        got,
        [
            0xB358_FAF7_4EF9_765A,
            0x475C_3D96_4F48_2CD2,
            0xD6F1_D349_952C_7996,
            0xFB29_3873_1E80_7240
        ]
    );
}

#[test]
fn gen_range_known_answers_and_bounds() {
    let mut r = PokerbotRng::seed_from_u64(7);
    let got: Vec<u64> = (0..10).map(|_| r.gen_range(52)).collect();
    assert_eq!(got, [36, 14, 43, 51, 51, 45, 3, 5, 20, 7]);
    let mut r = PokerbotRng::seed_from_u64(99);
    for n in [1u64, 2, 3, 7, 52, 1000, u64::MAX] {
        for _ in 0..200 {
            assert!(r.gen_range(n) < n);
        }
    }
    assert_eq!(PokerbotRng::seed_from_u64(5).gen_range(1), 0);
}

#[test]
fn gen_f64_in_unit_interval() {
    let mut r = PokerbotRng::seed_from_u64(3);
    let mut sum = 0.0;
    for _ in 0..10_000 {
        let x = r.gen_f64();
        assert!((0.0..1.0).contains(&x));
        sum += x;
    }
    let mean = sum / 10_000.0;
    assert!((mean - 0.5).abs() < 0.02, "mean {mean}");
}

#[test]
fn state_export_restore_continues_identically() {
    let mut a = PokerbotRng::seed_from_u64(12345);
    for _ in 0..17 {
        a.next_u64();
    }
    let mut b = PokerbotRng::from_state(a.state());
    for _ in 0..1000 {
        assert_eq!(a.next_u64(), b.next_u64());
    }
    assert_eq!(a.gen_range(52), b.gen_range(52));
    assert_eq!(a.gen_f64().to_bits(), b.gen_f64().to_bits());
}

#[test]
fn gen_range_uniformity_chi_square() {
    // 60_000 draws over 6 bins, expected 10_000 each. chi^2 with 5 dof:
    // the 99.99th percentile is ~25.7; use a loose bound of 30.
    let mut r = PokerbotRng::seed_from_u64(2024);
    let mut bins = [0u64; 6];
    for _ in 0..60_000 {
        bins[r.gen_range(6) as usize] += 1;
    }
    let chi: f64 = bins
        .iter()
        .map(|&o| (o as f64 - 10_000.0).powi(2) / 10_000.0)
        .sum();
    assert!(chi < 30.0, "chi^2 = {chi}, bins = {bins:?}");
}

// ------------------------------------------------------------------ deck ---

#[test]
fn every_shuffle_has_52_unique_cards() {
    for seed in 0..500u64 {
        let d = Deck::from_seed(seed);
        assert_eq!(d.remaining(), NUM_CARDS);
        let set = CardSet::try_from_cards(d.order()).expect("no duplicates");
        assert_eq!(set, CardSet::full());
    }
}

#[test]
fn dealing_exhausts_without_duplicates() {
    let mut d = Deck::from_seed(11);
    let mut set = CardSet::new();
    while let Some(c) = d.deal() {
        assert!(set.insert(c));
    }
    assert_eq!(set.len(), 52);
    assert_eq!(d.remaining(), 0);
    assert!(d.deal().is_none());
    assert!(d.deal_n(1).is_none());
}

#[test]
fn same_seed_same_order_different_seed_differs() {
    assert_eq!(Deck::from_seed(42).order(), Deck::from_seed(42).order());
    let mut distinct = std::collections::HashSet::new();
    for seed in 0..200u64 {
        distinct.insert(Deck::from_seed(seed).order().to_vec());
    }
    assert_eq!(distinct.len(), 200);
}

#[test]
fn golden_seed7_first_nine_cards() {
    // Fisher–Yates from ordered deck with PokerbotRng::seed_from_u64(7);
    // cross-checked with the Python reference implementation.
    let mut d = Deck::from_seed(7);
    let nine = d.deal_n(9).unwrap();
    let s: Vec<String> = nine.iter().map(|c| c.to_string()).collect();
    assert_eq!(s.join(" "), "Qh 9c Kh 8h Js 7s 2c Ts 7d");
    assert_eq!(d.remaining(), 43);
}

#[test]
fn shuffled_with_rng_matches_from_seed_and_advances() {
    let mut rng = PokerbotRng::seed_from_u64(7);
    let a = Deck::shuffled(&mut rng);
    assert_eq!(a.order(), Deck::from_seed(7).order());
    let b = Deck::shuffled(&mut rng);
    assert_ne!(a.order(), b.order(), "second shuffle uses advanced state");
}

#[test]
fn excluding_known_cards() {
    let known = CardSet::try_from_cards(&parse_cards("As Kd 7c 2h Th").unwrap()).unwrap();
    let mut rng = PokerbotRng::seed_from_u64(5);
    let mut d = Deck::shuffled_excluding(&mut rng, known);
    assert_eq!(d.remaining(), 47);
    let mut set = CardSet::new();
    while let Some(c) = d.deal() {
        assert!(!known.contains(c));
        assert!(set.insert(c));
    }
    assert_eq!(set, CardSet::full().difference(known));
}

#[test]
fn from_order_roundtrip_and_rejects_duplicates() {
    let d = Deck::from_seed(9);
    let r = Deck::from_order(d.order().to_vec()).unwrap();
    assert_eq!(r, d);
    let a: Card = "As".parse().unwrap();
    assert!(Deck::from_order(vec![a, a]).is_err());
}

#[test]
fn shuffle_position_uniformity() {
    // Card-by-position counts over 10_400 shuffles (expected 200 per cell).
    // Sum of 52 per-position chi^2 statistics, each with 51 dof, has
    // mean 2652 and sd ~72.8; bound at mean + 6 sd.
    let n = 10_400u64;
    let mut counts = vec![[0u32; 52]; 52];
    let mut rng = PokerbotRng::seed_from_u64(0xDECC);
    for _ in 0..n {
        let d = Deck::shuffled(&mut rng);
        for (pos, c) in d.order().iter().enumerate() {
            counts[pos][c.index() as usize] += 1;
        }
    }
    let e = n as f64 / 52.0;
    let chi: f64 = counts
        .iter()
        .flat_map(|row| row.iter())
        .map(|&o| (o as f64 - e).powi(2) / e)
        .sum();
    assert!(chi < 2652.0 + 6.0 * 72.8, "chi^2 = {chi}");
    assert!(chi > 2652.0 - 6.0 * 72.8, "suspiciously uniform: {chi}");
}
