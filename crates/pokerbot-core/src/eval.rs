//! Hand evaluation: 5-card categories and best-five-of-seven ranking.
//!
//! The fast path ([`evaluate`]) works on per-suit 13-bit rank masks and
//! per-rank counts, with no allocation and no subset enumeration. A flush is
//! checked first: with at most seven cards a flush excludes quads and full
//! houses (each would need at least eight cards), so the order is safe.
//!
//! A deliberately simple, independently written reference
//! ([`reference_evaluate5`], [`reference_evaluate`]) sorts the five ranks and
//! classifies rank groups; the 5–7 card reference takes the maximum over all
//! five-card subsets. Tests assert both agree.
//!
//! # Encoding
//! [`HandRank`] packs into a `u32`:
//! `category << 20 | r0 << 16 | r1 << 12 | r2 << 8 | r3 << 4 | r4`, where
//! `r0..r4` are the ranks (0 = Two .. 12 = Ace) of the best five cards in
//! order of significance: larger groups first, then higher rank (e.g. full
//! house `K K K 4 4`, two pair `Q Q 9 9 A`). Straights list ranks top-down,
//! and the wheel is `5 4 3 2 A`, so its leading `5` ranks it below a six-high
//! straight. Integer comparison therefore orders hands exactly: greater
//! wins, equal is a tie.

use std::fmt;

use crate::card::{Card, CardSet, Rank};

/// Hand categories, weakest to strongest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum HandCategory {
    HighCard = 0,
    Pair,
    TwoPair,
    Trips,
    Straight,
    Flush,
    FullHouse,
    Quads,
    StraightFlush,
}

impl HandCategory {
    /// All categories, weakest first.
    pub const ALL: [HandCategory; 9] = [
        HandCategory::HighCard,
        HandCategory::Pair,
        HandCategory::TwoPair,
        HandCategory::Trips,
        HandCategory::Straight,
        HandCategory::Flush,
        HandCategory::FullHouse,
        HandCategory::Quads,
        HandCategory::StraightFlush,
    ];

    /// Index `0..9`.
    pub const fn index(self) -> usize {
        self as usize
    }

    /// Human-readable name.
    pub const fn name(self) -> &'static str {
        match self {
            HandCategory::HighCard => "High card",
            HandCategory::Pair => "Pair",
            HandCategory::TwoPair => "Two pair",
            HandCategory::Trips => "Three of a kind",
            HandCategory::Straight => "Straight",
            HandCategory::Flush => "Flush",
            HandCategory::FullHouse => "Full house",
            HandCategory::Quads => "Four of a kind",
            HandCategory::StraightFlush => "Straight flush",
        }
    }
}

/// Totally ordered hand strength; greater wins, equal ties.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HandRank(u32);

impl HandRank {
    #[inline]
    const fn pack(cat: HandCategory, r: [u8; 5]) -> HandRank {
        HandRank(
            (cat as u32) << 20
                | (r[0] as u32) << 16
                | (r[1] as u32) << 12
                | (r[2] as u32) << 8
                | (r[3] as u32) << 4
                | r[4] as u32,
        )
    }

    /// The packed comparison value.
    pub const fn as_u32(self) -> u32 {
        self.0
    }

    /// Hand category.
    pub fn category(self) -> HandCategory {
        HandCategory::ALL[(self.0 >> 20) as usize]
    }

    /// Ranks of the best five cards in order of significance (see module
    /// docs), e.g. `[K, K, K, 4, 4]` or `[5, 4, 3, 2, A]` for the wheel.
    pub fn ranks(self) -> [Rank; 5] {
        let nib = |i: u32| Rank::ALL[((self.0 >> (16 - 4 * i)) & 0xF) as usize];
        [nib(0), nib(1), nib(2), nib(3), nib(4)]
    }
}

impl fmt::Display for HandRank {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.category().name())?;
        for (i, r) in self.ranks().iter().enumerate() {
            write!(f, "{}{}", if i == 0 { " (" } else { " " }, r.to_char())?;
        }
        write!(f, ")")
    }
}

// ------------------------------------------------------------ fast path ---

const ACE_BIT: u16 = 1 << 12;

/// Highest set bit index of a non-zero 13-bit mask.
#[inline]
fn hi(m: u16) -> u8 {
    debug_assert!(m != 0);
    (15 - m.leading_zeros()) as u8
}

/// Top rank of the highest straight in a rank mask (wheel = 3, i.e. Five).
#[inline]
fn straight_top(m: u16) -> Option<u8> {
    // Bit i of `ext` stands for rank i-1, with the ace duplicated at bit 0.
    let ext = (u32::from(m) << 1) | u32::from(m & ACE_BIT != 0);
    let runs = ext & (ext >> 1) & (ext >> 2) & (ext >> 3) & (ext >> 4);
    if runs == 0 {
        None
    } else {
        // Run starting at ext bit j covers ranks j-1 ..= j+3.
        Some((31 - runs.leading_zeros()) as u8 + 3)
    }
}

#[inline]
fn straight_ranks(top: u8) -> [u8; 5] {
    if top == 3 {
        [3, 2, 1, 0, 12]
    } else {
        [top, top - 1, top - 2, top - 3, top - 4]
    }
}

/// Up to `N` highest ranks of `m`, highest first, written into `out`.
#[inline]
fn take_top(mut m: u16, out: &mut [u8]) {
    for slot in out.iter_mut() {
        let h = hi(m);
        *slot = h;
        m &= !(1 << h);
    }
}

/// Evaluates 5 to 7 distinct cards with the mask-based fast path.
///
/// # Panics
/// Panics if `cards.len()` is not in `5..=7`. Duplicate cards are a caller
/// error (checked with `debug_assert!`).
pub fn evaluate(cards: &[Card]) -> HandRank {
    assert!(
        (5..=7).contains(&cards.len()),
        "evaluate needs 5 to 7 cards, got {}",
        cards.len()
    );
    debug_assert!(
        CardSet::try_from_cards(cards).is_ok(),
        "duplicate cards passed to evaluate"
    );
    let mut suit_masks = [0u16; 4];
    let mut counts = [0u8; 13];
    for c in cards {
        let r = c.index() >> 2;
        suit_masks[usize::from(c.index() & 3)] |= 1 << r;
        counts[usize::from(r)] += 1;
    }

    for &m in &suit_masks {
        if m.count_ones() >= 5 {
            if let Some(top) = straight_top(m) {
                return HandRank::pack(HandCategory::StraightFlush, straight_ranks(top));
            }
            let mut r = [0u8; 5];
            take_top(m, &mut r);
            return HandRank::pack(HandCategory::Flush, r);
        }
    }

    // Rank masks by exact multiplicity.
    let mut by_count = [0u16; 5];
    for (r, &n) in counts.iter().enumerate() {
        by_count[usize::from(n)] |= 1 << r;
    }
    let all = suit_masks[0] | suit_masks[1] | suit_masks[2] | suit_masks[3];
    let (m2, m3, m4) = (by_count[2], by_count[3], by_count[4]);

    if m4 != 0 {
        let q = hi(m4);
        let k = hi(all & !(1 << q));
        return HandRank::pack(HandCategory::Quads, [q, q, q, q, k]);
    }
    if m3 != 0 {
        let t = hi(m3);
        let rest = (m3 & !(1 << t)) | m2;
        if rest != 0 {
            let p = hi(rest);
            return HandRank::pack(HandCategory::FullHouse, [t, t, t, p, p]);
        }
    }
    if let Some(top) = straight_top(all) {
        return HandRank::pack(HandCategory::Straight, straight_ranks(top));
    }
    if m3 != 0 {
        let t = hi(m3);
        let mut k = [0u8; 2];
        take_top(all & !(1 << t), &mut k);
        return HandRank::pack(HandCategory::Trips, [t, t, t, k[0], k[1]]);
    }
    if m2.count_ones() >= 2 {
        let hp = hi(m2);
        let lp = hi(m2 & !(1 << hp));
        let k = hi(all & !(1 << hp) & !(1 << lp));
        return HandRank::pack(HandCategory::TwoPair, [hp, hp, lp, lp, k]);
    }
    if m2 != 0 {
        let p = hi(m2);
        let mut k = [0u8; 3];
        take_top(all & !(1 << p), &mut k);
        return HandRank::pack(HandCategory::Pair, [p, p, k[0], k[1], k[2]]);
    }
    let mut r = [0u8; 5];
    take_top(all, &mut r);
    HandRank::pack(HandCategory::HighCard, r)
}

/// Evaluates exactly five cards.
#[inline]
pub fn evaluate5(cards: &[Card; 5]) -> HandRank {
    evaluate(cards)
}

/// Evaluates seven cards (best five of seven).
#[inline]
pub fn evaluate7(cards: &[Card; 7]) -> HandRank {
    evaluate(cards)
}

/// The best five of 5–7 cards, ordered by significance to match
/// [`HandRank::ranks`] (e.g. trips first, then the pair; wheel as 5-4-3-2-A).
/// When several subsets tie, the first in lexicographic index order of the
/// input is used, so the result is deterministic.
///
/// # Panics
/// Panics if `cards.len()` is not in `5..=7`.
pub fn best_five(cards: &[Card]) -> [Card; 5] {
    let best = evaluate(cards);
    let mut chosen = None;
    for_each_subset5(cards, |sub| {
        if chosen.is_none() && evaluate(&sub) == best {
            chosen = Some(sub);
        }
    });
    let sub = chosen.expect("some subset attains the best rank");
    // Order the subset to follow the significance order of ranks().
    let mut out = sub;
    let mut used = [false; 5];
    for (slot, want) in out.iter_mut().zip(best.ranks()) {
        // Within equal ranks keep higher suit index first for stable display.
        let i = (0..5)
            .filter(|&i| !used[i] && sub[i].rank() == want)
            .max_by_key(|&i| sub[i].suit())
            .expect("rank present in subset");
        used[i] = true;
        *slot = sub[i];
    }
    out
}

/// Calls `f` for every five-card subset (in lexicographic index order).
fn for_each_subset5(cards: &[Card], mut f: impl FnMut([Card; 5])) {
    let n = cards.len();
    for a in 0..n {
        for b in a + 1..n {
            for c in b + 1..n {
                for d in c + 1..n {
                    for e in d + 1..n {
                        f([cards[a], cards[b], cards[c], cards[d], cards[e]]);
                    }
                }
            }
        }
    }
}

// ------------------------------------------------------------ reference ---

/// Straightforward reference 5-card evaluator (sorting and grouping). Slow;
/// intended for cross-checking the fast path.
pub fn reference_evaluate5(cards: &[Card; 5]) -> HandRank {
    let mut ranks: Vec<u8> = cards.iter().map(|c| c.rank().as_u8()).collect();
    ranks.sort_unstable_by(|a, b| b.cmp(a));
    let flush = cards.iter().all(|c| c.suit() == cards[0].suit());
    let distinct = ranks.windows(2).all(|w| w[0] != w[1]);
    let straight = if !distinct {
        None
    } else if ranks[0] - ranks[4] == 4 {
        Some([ranks[0], ranks[1], ranks[2], ranks[3], ranks[4]])
    } else if ranks == [12, 3, 2, 1, 0] {
        Some([3, 2, 1, 0, 12])
    } else {
        None
    };
    if let Some(s) = straight {
        let cat = if flush {
            HandCategory::StraightFlush
        } else {
            HandCategory::Straight
        };
        return HandRank::pack(cat, s);
    }
    // Groups as (count, rank), sorted by count then rank, descending.
    let mut groups: Vec<(u8, u8)> = Vec::new();
    for &r in &ranks {
        match groups.iter_mut().find(|g| g.1 == r) {
            Some(g) => g.0 += 1,
            None => groups.push((1, r)),
        }
    }
    groups.sort_unstable_by(|a, b| b.cmp(a));
    let shape: Vec<u8> = groups.iter().map(|g| g.0).collect();
    let mut ordered = Vec::with_capacity(5);
    for &(n, r) in &groups {
        for _ in 0..n {
            ordered.push(r);
        }
    }
    let r = [ordered[0], ordered[1], ordered[2], ordered[3], ordered[4]];
    let cat = match shape.as_slice() {
        [4, 1] => HandCategory::Quads,
        [3, 2] => HandCategory::FullHouse,
        _ if flush => HandCategory::Flush,
        [3, 1, 1] => HandCategory::Trips,
        [2, 2, 1] => HandCategory::TwoPair,
        [2, 1, 1, 1] => HandCategory::Pair,
        _ => HandCategory::HighCard,
    };
    HandRank::pack(cat, r)
}

/// Reference 5–7 card evaluator: maximum of [`reference_evaluate5`] over
/// every five-card subset (up to 21).
pub fn reference_evaluate(cards: &[Card]) -> HandRank {
    assert!((5..=7).contains(&cards.len()));
    let mut best = None;
    for_each_subset5(cards, |sub| {
        let r = reference_evaluate5(&sub);
        if best.is_none_or(|b| r > b) {
            best = Some(r);
        }
    });
    best.expect("at least one subset")
}
