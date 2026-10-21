//! Hand-strength equity against a uniformly random opponent, and
//! suit-isomorphism canonicalization (M-08).
//!
//! # Definition
//! `equity(hole, board)` is the probability that `hole` beats a hand drawn
//! uniformly from the cards not in `hole ∪ board`, with any missing board
//! cards also run out uniformly. A win counts 1 and a tie counts ½. This is
//! **not** a range model: it is equity against a random hand, a deliberately
//! coarse approximation for abstraction, not a measure of optimal play.
//!
//! Results are exact rationals ([`Equity`]: `points / (2·trials)`, where
//! `points = 2·wins + ties`), so bucketing is pure integer arithmetic and
//! identical on every platform, including wasm32.
//!
//! - River: exact over all C(45,2) = 990 opponent hands.
//! - Turn: exact over 46 rivers × C(45,2) opponent hands (45,540 showdowns).
//! - Flop: exact is C(47,2)·C(45,2) ≈ 1.07M showdowns ([`exact_equity`] can
//!   do it for tests); training uses seeded Monte Carlo
//!   ([`sampled_equity`]).
//!
//! # Canonicalization
//! Equity depends only on the *set* of board cards (not which street each
//! arrived on) and is invariant under the 24 permutations of suits.
//! [`canonical_form`] sorts hole and board separately, then takes the
//! lexicographically smallest encoding over all suit permutations. Hole vs
//! board stays distinguished. [`CanonicalHand::key`] packs the result into
//! 45 bits of a `u64` **injectively** (no hashing, so no collisions).

use pokerbot_core::card::{Card, CardSet};
use pokerbot_core::eval::evaluate;
use pokerbot_core::rng::PokerbotRng;

/// All 24 permutations of the four suits.
fn suit_permutations() -> [[u8; 4]; 24] {
    let mut out = [[0u8; 4]; 24];
    let mut n = 0;
    for a in 0..4u8 {
        for b in 0..4u8 {
            for c in 0..4u8 {
                for d in 0..4u8 {
                    if a != b && a != c && a != d && b != c && b != d && c != d {
                        out[n] = [a, b, c, d];
                        n += 1;
                    }
                }
            }
        }
    }
    out
}

/// Apply a suit permutation to a card.
pub fn permute_suit(card: Card, perm: [u8; 4]) -> Card {
    let i = card.index();
    Card::from_index(i & !3 | perm[usize::from(i & 3)]).expect("valid card")
}

/// Canonical (suit-normalized, sorted) hole cards and board.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CanonicalHand {
    hole: [u8; 2],
    board: [u8; 5],
    board_len: u8,
}

impl CanonicalHand {
    /// Canonical hole cards (ascending index).
    pub fn hole(&self) -> [Card; 2] {
        self.hole.map(|i| Card::from_index(i).expect("valid"))
    }

    /// Canonical board (ascending index).
    pub fn board(&self) -> Vec<Card> {
        self.board[..usize::from(self.board_len)]
            .iter()
            .map(|&i| Card::from_index(i).expect("valid"))
            .collect()
    }

    /// Number of board cards (0, 3, 4 or 5).
    pub fn board_len(&self) -> usize {
        usize::from(self.board_len)
    }

    /// Stable injective 64-bit key: bits 0–2 board length, then two hole
    /// card indices and up to five board indices, 6 bits each, in canonical
    /// order (unused board slots are 0; the length field disambiguates).
    pub fn key(&self) -> u64 {
        let mut k = u64::from(self.board_len);
        let mut shift = 3;
        for &c in self.hole.iter().chain(self.board.iter()) {
            k |= u64::from(c) << shift;
            shift += 6;
        }
        k
    }
}

fn encode(hole: [Card; 2], board: &[Card], perm: [u8; 4]) -> ([u8; 2], [u8; 5]) {
    let mut h = hole.map(|c| permute_suit(c, perm).index());
    h.sort_unstable();
    let mut b = [0u8; 5];
    for (slot, &c) in b.iter_mut().zip(board) {
        *slot = permute_suit(c, perm).index();
    }
    b[..board.len()].sort_unstable();
    (h, b)
}

/// Canonical form of `(hole, board)` under suit permutations.
///
/// # Panics
/// If `board.len() > 5` or the cards are not distinct.
pub fn canonical_form(hole: [Card; 2], board: &[Card]) -> CanonicalHand {
    assert!(board.len() <= 5, "board has more than five cards");
    let mut all = vec![hole[0], hole[1]];
    all.extend_from_slice(board);
    assert!(
        CardSet::try_from_cards(&all).is_ok(),
        "hole and board cards must be distinct"
    );
    let mut best: Option<([u8; 2], [u8; 5])> = None;
    for perm in suit_permutations() {
        let e = encode(hole, board, perm);
        // Compare hole first, then board: lexicographic on the tuple.
        if best.is_none_or(|b| e < b) {
            best = Some(e);
        }
    }
    let (hole, board_bytes) = best.expect("24 permutations");
    CanonicalHand {
        hole,
        board: board_bytes,
        board_len: board.len() as u8,
    }
}

/// Canonical key of `(hole, board)`; see [`CanonicalHand::key`].
pub fn canonical_key(hole: [Card; 2], board: &[Card]) -> u64 {
    canonical_form(hole, board).key()
}

/// Equity as an exact rational: `points / (2 * trials)`, where each trial
/// adds 2 for a win, 1 for a tie and 0 for a loss.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Equity {
    /// `2·wins + ties`.
    pub points: u64,
    /// Number of showdowns evaluated.
    pub trials: u64,
}

impl Equity {
    /// Equity in `[0, 1]`.
    pub fn value(&self) -> f64 {
        if self.trials == 0 {
            return 0.0;
        }
        self.points as f64 / (2 * self.trials) as f64
    }

    /// `min(floor(equity · n), n − 1)` in exact integer arithmetic.
    ///
    /// # Panics
    /// If `n == 0` or `trials == 0`.
    pub fn bucket(&self, n: u16) -> u16 {
        assert!(n > 0 && self.trials > 0);
        let num = u128::from(self.points) * u128::from(n);
        let den = 2 * u128::from(self.trials);
        let b = (num / den) as u64;
        b.min(u64::from(n) - 1) as u16
    }
}

fn remaining(known: &[Card]) -> Vec<Card> {
    let used: CardSet = known.iter().copied().collect();
    Card::all().filter(|c| !used.contains(*c)).collect()
}

fn showdown_points(hero: u32, opp: u32) -> u64 {
    match hero.cmp(&opp) {
        std::cmp::Ordering::Greater => 2,
        std::cmp::Ordering::Equal => 1,
        std::cmp::Ordering::Less => 0,
    }
}

#[inline]
fn rank_of(cards: &[Card]) -> u32 {
    evaluate(cards).as_u32()
}

/// Exact river showdown tally against every opponent hand from `rest`.
fn river_tally(hole: [Card; 2], board5: &[Card; 5], rest: &[Card], skip: Option<Card>) -> Equity {
    let mut seven = [
        hole[0], hole[1], board5[0], board5[1], board5[2], board5[3], board5[4],
    ];
    let hero = rank_of(&seven);
    let mut points = 0;
    let mut trials = 0;
    for i in 0..rest.len() {
        if Some(rest[i]) == skip {
            continue;
        }
        for j in i + 1..rest.len() {
            if Some(rest[j]) == skip {
                continue;
            }
            seven[0] = rest[i];
            seven[1] = rest[j];
            points += showdown_points(hero, rank_of(&seven));
            trials += 1;
        }
    }
    Equity { points, trials }
}

/// Exact equity by full enumeration of opponent hands and board runouts.
/// Practical for river and turn; the flop (~1.07M showdowns) is meant for
/// tests and offline checks.
///
/// # Panics
/// If `board.len()` is not 3, 4 or 5, or cards repeat.
pub fn exact_equity(hole: [Card; 2], board: &[Card]) -> Equity {
    assert!((3..=5).contains(&board.len()), "board must have 3-5 cards");
    let mut known = vec![hole[0], hole[1]];
    known.extend_from_slice(board);
    assert!(CardSet::try_from_cards(&known).is_ok(), "duplicate cards");
    let rest = remaining(&known);
    let mut total = Equity {
        points: 0,
        trials: 0,
    };
    let mut add = |e: Equity| {
        total.points += e.points;
        total.trials += e.trials;
    };
    match board.len() {
        5 => add(river_tally(
            hole,
            &[board[0], board[1], board[2], board[3], board[4]],
            &rest,
            None,
        )),
        4 => {
            for &r in &rest {
                let b = [board[0], board[1], board[2], board[3], r];
                add(river_tally(hole, &b, &rest, Some(r)));
            }
        }
        _ => {
            for i in 0..rest.len() {
                for j in i + 1..rest.len() {
                    let b = [board[0], board[1], board[2], rest[i], rest[j]];
                    let sub: Vec<Card> = rest
                        .iter()
                        .copied()
                        .filter(|&c| c != rest[i] && c != rest[j])
                        .collect();
                    add(river_tally(hole, &b, &sub, None));
                }
            }
        }
    }
    total
}

/// Monte Carlo equity with `samples` trials drawn from `rng`. Each trial
/// draws the opponent's two cards and the missing board cards without
/// replacement from the unseen cards.
///
/// # Panics
/// If `board.len()` is not 3, 4 or 5, `samples == 0`, or cards repeat.
pub fn sampled_equity(
    hole: [Card; 2],
    board: &[Card],
    samples: u32,
    rng: &mut PokerbotRng,
) -> Equity {
    assert!((3..=5).contains(&board.len()), "board must have 3-5 cards");
    assert!(samples > 0, "samples must be positive");
    let mut known = vec![hole[0], hole[1]];
    known.extend_from_slice(board);
    assert!(CardSet::try_from_cards(&known).is_ok(), "duplicate cards");
    let mut rest = remaining(&known);
    let need = 2 + (5 - board.len());
    let mut hero7 = [hole[0]; 7];
    let mut opp7 = [hole[0]; 7];
    let mut points = 0;
    for _ in 0..samples {
        // Partial Fisher–Yates: draw `need` distinct cards into the front.
        for k in 0..need {
            let j = k + rng.gen_range((rest.len() - k) as u64) as usize;
            rest.swap(k, j);
        }
        let mut b = [hole[0]; 5];
        b[..board.len()].copy_from_slice(board);
        b[board.len()..].copy_from_slice(&rest[2..need]);
        hero7[..2].copy_from_slice(&hole);
        hero7[2..].copy_from_slice(&b);
        opp7[..2].copy_from_slice(&rest[..2]);
        opp7[2..].copy_from_slice(&b);
        points += showdown_points(rank_of(&hero7), rank_of(&opp7));
    }
    Equity {
        points,
        trials: u64::from(samples),
    }
}
