//! Seeded, portable deck shuffling and dealing.
//!
//! The shuffle is the Fisher–Yates / Durstenfeld algorithm driven by the
//! unbiased [`MakoRng::gen_range`], starting from the deck in ascending
//! card-index order (`2c 2d 2h 2s 3c ... As`). For `i` from `n-1` down to
//! `1`, swap position `i` with position `gen_range(i + 1)`. Cards are dealt
//! from position 0 upward. Given the same seed (or RNG state) the order is
//! identical on every platform.
//!
//! # Recommended dealing order (consumed by the hand state machine, M-04)
//!
//! Hole cards first, one at a time, alternating, starting with the
//! non-button seat (the big blind): BB, Button, BB, Button. Then the board
//! is taken from the next five cards in order: flop = cards 4..7,
//! turn = card 7, river = card 8. No burn cards are used (burns add nothing
//! in a simulation with a trusted shuffle). With this order the first nine
//! dealt cards fully determine a hand, and a hand's private cards are
//! always dealt before any public card.

use crate::card::{Card, CardSet, NUM_CARDS};
use crate::rng::MakoRng;

/// An ordered deck with a deal cursor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Deck {
    cards: Vec<Card>,
    next: usize,
}

impl Deck {
    /// The unshuffled 52-card deck in ascending index order.
    pub fn ordered() -> Deck {
        Deck {
            cards: Card::all().collect(),
            next: 0,
        }
    }

    /// A full deck shuffled with a fresh generator seeded by `seed`.
    pub fn from_seed(seed: u64) -> Deck {
        Deck::shuffled(&mut MakoRng::seed_from_u64(seed))
    }

    /// A full deck shuffled with the supplied generator (which advances).
    pub fn shuffled(rng: &mut MakoRng) -> Deck {
        Deck::shuffled_excluding(rng, CardSet::new())
    }

    /// A shuffled deck containing every card not in `excluded`, e.g. for
    /// sampling unknown cards given known hole cards and board.
    pub fn shuffled_excluding(rng: &mut MakoRng, excluded: CardSet) -> Deck {
        let mut deck = Deck {
            cards: Card::all().filter(|c| !excluded.contains(*c)).collect(),
            next: 0,
        };
        deck.shuffle(rng);
        deck
    }

    /// Fisher–Yates shuffle of the undealt cards, resetting the cursor to
    /// include all cards in the deck.
    pub fn shuffle(&mut self, rng: &mut MakoRng) {
        self.next = 0;
        let n = self.cards.len();
        for i in (1..n).rev() {
            let j = rng.gen_range(i as u64 + 1) as usize;
            self.cards.swap(i, j);
        }
    }

    /// Deals the next card, or `None` when exhausted.
    pub fn deal(&mut self) -> Option<Card> {
        let c = self.cards.get(self.next).copied()?;
        self.next += 1;
        Some(c)
    }

    /// Deals `n` cards, or `None` (without dealing any) if fewer remain.
    pub fn deal_n(&mut self, n: usize) -> Option<Vec<Card>> {
        if self.remaining() < n {
            return None;
        }
        let out = self.cards[self.next..self.next + n].to_vec();
        self.next += n;
        Some(out)
    }

    /// Number of undealt cards.
    pub fn remaining(&self) -> usize {
        self.cards.len() - self.next
    }

    /// The undealt cards, next card first.
    pub fn remaining_cards(&self) -> &[Card] {
        &self.cards[self.next..]
    }

    /// The full deck order (dealt and undealt), for replay/serialization.
    pub fn order(&self) -> &[Card] {
        &self.cards
    }

    /// Rebuilds a deck from an explicit order (e.g. a replay file).
    /// Rejects duplicates and more than 52 cards.
    pub fn from_order(cards: Vec<Card>) -> Result<Deck, crate::card::DuplicateCard> {
        CardSet::try_from_cards(&cards)?;
        debug_assert!(cards.len() <= NUM_CARDS);
        Ok(Deck { cards, next: 0 })
    }
}
