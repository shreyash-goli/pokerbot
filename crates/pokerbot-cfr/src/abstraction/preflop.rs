//! The 169 strategically distinct preflop starting-hand classes.
//!
//! Before the flop, suits matter only through suitedness, so the 1326 hole
//! card combinations collapse to 169 classes: 13 pairs (6 combos each), 78
//! suited non-pairs (4 combos each) and 78 offsuit non-pairs (12 combos
//! each); `13·6 + 78·4 + 78·12 = 1326`. This is standard poker combinatorics.
//!
//! # Stable index order (pinned; artifacts depend on it)
//! - `0..13`: pairs by descending rank: `AA = 0, KK = 1, …, 22 = 12`.
//! - `13..91`: suited hands, ordered by high rank descending, then low rank
//!   descending: `AKs = 13, AQs = 14, …, A2s = 24, KQs = 25, …, 32s = 90`.
//! - `91..169`: offsuit hands in the same order: `AKo = 91, …, 32o = 168`.
//!
//! For high rank `h` and low rank `l` (`Two = 0 … Ace = 12`, `h > l`) the
//! non-pair offset is `78 - h(h+1)/2 + (h - 1 - l)`.

use std::fmt;
use std::str::FromStr;

use pokerbot_core::card::{Card, Rank, Suit};

/// Number of preflop classes.
pub const NUM_PREFLOP_CLASSES: usize = 169;
/// Number of distinct two-card hole combinations.
pub const NUM_HOLE_COMBOS: usize = 1326;

const NUM_PAIRS: u8 = 13;
const NUM_NONPAIR: u8 = 78;

/// A preflop hand class in `0..169` (see the module docs for the order).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PreflopClass(u8);

/// Pair / suited / offsuit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ClassKind {
    /// Both cards share a rank.
    Pair,
    /// Different ranks, same suit.
    Suited,
    /// Different ranks, different suits.
    Offsuit,
}

fn nonpair_offset(high: u8, low: u8) -> u8 {
    debug_assert!(high > low && high <= 12);
    let h = u16::from(high);
    (78 - h * (h + 1) / 2 + (h - 1 - u16::from(low))) as u8
}

impl PreflopClass {
    /// Class of two distinct hole cards. Order-independent.
    ///
    /// # Panics
    /// If both cards are identical.
    pub fn of(hole: [Card; 2]) -> PreflopClass {
        assert_ne!(hole[0], hole[1], "hole cards must be distinct");
        let (r0, r1) = (hole[0].rank().as_u8(), hole[1].rank().as_u8());
        let (high, low) = if r0 >= r1 { (r0, r1) } else { (r1, r0) };
        Self::from_parts(
            high,
            low,
            if high == low {
                ClassKind::Pair
            } else if hole[0].suit() == hole[1].suit() {
                ClassKind::Suited
            } else {
                ClassKind::Offsuit
            },
        )
    }

    fn from_parts(high: u8, low: u8, kind: ClassKind) -> PreflopClass {
        match kind {
            ClassKind::Pair => PreflopClass(12 - high),
            ClassKind::Suited => PreflopClass(NUM_PAIRS + nonpair_offset(high, low)),
            ClassKind::Offsuit => PreflopClass(NUM_PAIRS + NUM_NONPAIR + nonpair_offset(high, low)),
        }
    }

    /// Class from its index, if `< 169`.
    pub fn from_index(index: u8) -> Option<PreflopClass> {
        (usize::from(index) < NUM_PREFLOP_CLASSES).then_some(PreflopClass(index))
    }

    /// Index in `0..169`.
    pub fn index(self) -> u8 {
        self.0
    }

    /// All classes in index order.
    pub fn all() -> impl Iterator<Item = PreflopClass> {
        (0..NUM_PREFLOP_CLASSES as u8).map(PreflopClass)
    }

    /// Pair, suited, or offsuit.
    pub fn kind(self) -> ClassKind {
        if self.0 < NUM_PAIRS {
            ClassKind::Pair
        } else if self.0 < NUM_PAIRS + NUM_NONPAIR {
            ClassKind::Suited
        } else {
            ClassKind::Offsuit
        }
    }

    /// `(high rank, low rank)` as `Rank` values (equal for pairs).
    pub fn ranks(self) -> (Rank, Rank) {
        let to_rank = |v: u8| Rank::from_u8(v).expect("rank in range");
        if self.0 < NUM_PAIRS {
            let r = to_rank(12 - self.0);
            return (r, r);
        }
        let off = (self.0 - NUM_PAIRS) % NUM_NONPAIR;
        // Walk high ranks from Ace down; high rank h owns h low ranks.
        let mut start = 0u8;
        for high in (1..=12u8).rev() {
            if off < start + high {
                let low = high - 1 - (off - start);
                return (to_rank(high), to_rank(low));
            }
            start += high;
        }
        unreachable!("offset {off} out of range")
    }

    /// Number of concrete combos: 6 (pair), 4 (suited) or 12 (offsuit).
    pub fn combos(self) -> u8 {
        match self.kind() {
            ClassKind::Pair => 6,
            ClassKind::Suited => 4,
            ClassKind::Offsuit => 12,
        }
    }

    /// All concrete hole-card combos in this class, each as
    /// `[high-rank card, low-rank card]` (pairs: lower suit first), in
    /// ascending suit order. Deterministic.
    pub fn hole_combos(self) -> Vec<[Card; 2]> {
        let (hi, lo) = self.ranks();
        let mut out = Vec::with_capacity(usize::from(self.combos()));
        for s0 in Suit::ALL {
            for s1 in Suit::ALL {
                let keep = match self.kind() {
                    ClassKind::Pair => s0 < s1,
                    ClassKind::Suited => s0 == s1,
                    ClassKind::Offsuit => s0 != s1,
                };
                if keep {
                    out.push([Card::new(hi, s0), Card::new(lo, s1)]);
                }
            }
        }
        out
    }
}

impl fmt::Display for PreflopClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (hi, lo) = self.ranks();
        write!(f, "{}{}", hi.to_char(), lo.to_char())?;
        match self.kind() {
            ClassKind::Pair => Ok(()),
            ClassKind::Suited => write!(f, "s"),
            ClassKind::Offsuit => write!(f, "o"),
        }
    }
}

/// Error parsing a preflop class string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsePreflopClassError(pub String);

impl fmt::Display for ParsePreflopClassError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid preflop class {:?}", self.0)
    }
}

impl std::error::Error for ParsePreflopClassError {}

impl FromStr for PreflopClass {
    type Err = ParsePreflopClassError;

    /// Parses `"AA"`, `"AKs"`, `"T9o"`. Ranks may appear in either order
    /// (`"KAs"` = `"AKs"`). Pairs take no suffix; non-pairs require `s`/`o`.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = || ParsePreflopClassError(s.to_string());
        let chars: Vec<char> = s.chars().collect();
        if chars.len() < 2 || chars.len() > 3 {
            return Err(err());
        }
        let r0 = Rank::from_char(chars[0]).ok_or_else(err)?.as_u8();
        let r1 = Rank::from_char(chars[1]).ok_or_else(err)?.as_u8();
        let (high, low) = if r0 >= r1 { (r0, r1) } else { (r1, r0) };
        let kind = match (high == low, chars.get(2)) {
            (true, None) => ClassKind::Pair,
            (false, Some('s')) => ClassKind::Suited,
            (false, Some('o')) => ClassKind::Offsuit,
            _ => return Err(err()),
        };
        Ok(PreflopClass::from_parts(high, low, kind))
    }
}
