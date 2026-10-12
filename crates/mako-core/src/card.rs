//! Playing cards and compact card sets.
//!
//! A [`Card`] is a single byte `index = rank * 4 + suit` in `0..52`, with
//! ranks `0..13` = Two..Ace and suits `0..4` = clubs, diamonds, hearts,
//! spades. Text form is two characters: rank (`23456789TJQKA`) followed by
//! suit (`cdhs`), e.g. `"As"`, `"Td"`, `"2c"`.

use std::fmt;
use std::str::FromStr;

/// Number of distinct cards in the deck.
pub const NUM_CARDS: usize = 52;

const RANK_CHARS: [char; 13] = [
    '2', '3', '4', '5', '6', '7', '8', '9', 'T', 'J', 'Q', 'K', 'A',
];
const SUIT_CHARS: [char; 4] = ['c', 'd', 'h', 's'];

/// Card rank, `Two` (0) through `Ace` (12).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum Rank {
    Two = 0,
    Three,
    Four,
    Five,
    Six,
    Seven,
    Eight,
    Nine,
    Ten,
    Jack,
    Queen,
    King,
    Ace,
}

impl Rank {
    /// All ranks in ascending order.
    pub const ALL: [Rank; 13] = [
        Rank::Two,
        Rank::Three,
        Rank::Four,
        Rank::Five,
        Rank::Six,
        Rank::Seven,
        Rank::Eight,
        Rank::Nine,
        Rank::Ten,
        Rank::Jack,
        Rank::Queen,
        Rank::King,
        Rank::Ace,
    ];

    /// Rank from its numeric value `0..13`.
    pub fn from_u8(v: u8) -> Option<Rank> {
        Rank::ALL.get(usize::from(v)).copied()
    }

    /// Numeric value `0..13`.
    pub fn as_u8(self) -> u8 {
        self as u8
    }

    /// Display character (`2`..`9`, `T`, `J`, `Q`, `K`, `A`).
    pub fn to_char(self) -> char {
        RANK_CHARS[self as usize]
    }

    /// Parses a rank character (case-insensitive for letters).
    pub fn from_char(c: char) -> Option<Rank> {
        let up = c.to_ascii_uppercase();
        RANK_CHARS
            .iter()
            .position(|&r| r == up)
            .and_then(|i| Rank::from_u8(i as u8))
    }
}

/// Card suit: clubs (0), diamonds (1), hearts (2), spades (3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum Suit {
    Clubs = 0,
    Diamonds,
    Hearts,
    Spades,
}

impl Suit {
    /// All suits in index order.
    pub const ALL: [Suit; 4] = [Suit::Clubs, Suit::Diamonds, Suit::Hearts, Suit::Spades];

    /// Suit from its numeric value `0..4`.
    pub fn from_u8(v: u8) -> Option<Suit> {
        Suit::ALL.get(usize::from(v)).copied()
    }

    /// Numeric value `0..4`.
    pub fn as_u8(self) -> u8 {
        self as u8
    }

    /// Display character (`c`, `d`, `h`, `s`).
    pub fn to_char(self) -> char {
        SUIT_CHARS[self as usize]
    }

    /// Parses a suit character (case-insensitive).
    pub fn from_char(c: char) -> Option<Suit> {
        let low = c.to_ascii_lowercase();
        SUIT_CHARS
            .iter()
            .position(|&s| s == low)
            .and_then(|i| Suit::from_u8(i as u8))
    }
}

/// A single playing card, stored as `rank * 4 + suit`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Card(u8);

impl Card {
    /// Builds a card from rank and suit.
    pub const fn new(rank: Rank, suit: Suit) -> Card {
        Card(rank as u8 * 4 + suit as u8)
    }

    /// Card from an index in `0..52`; `None` otherwise.
    pub const fn from_index(index: u8) -> Option<Card> {
        if (index as usize) < NUM_CARDS {
            Some(Card(index))
        } else {
            None
        }
    }

    /// Index in `0..52` (`rank * 4 + suit`).
    pub const fn index(self) -> u8 {
        self.0
    }

    /// The card's rank.
    pub fn rank(self) -> Rank {
        Rank::ALL[usize::from(self.0 / 4)]
    }

    /// The card's suit.
    pub fn suit(self) -> Suit {
        Suit::ALL[usize::from(self.0 % 4)]
    }

    /// All 52 cards in index order.
    pub fn all() -> impl Iterator<Item = Card> {
        (0..NUM_CARDS as u8).map(Card)
    }
}

impl fmt::Display for Card {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", self.rank().to_char(), self.suit().to_char())
    }
}

/// Error returned when parsing a card string fails.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseCardError(pub String);

impl fmt::Display for ParseCardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid card string {:?}", self.0)
    }
}

impl std::error::Error for ParseCardError {}

impl FromStr for Card {
    type Err = ParseCardError;

    /// Parses exactly two characters: rank then suit. Rank letters and suit
    /// letters are accepted in either case (`"as"`, `"AS"`, `"tD"`); no
    /// surrounding whitespace or other characters are allowed.
    fn from_str(s: &str) -> Result<Card, ParseCardError> {
        let err = || ParseCardError(s.to_string());
        let mut chars = s.chars();
        let (r, su) = match (chars.next(), chars.next(), chars.next()) {
            (Some(r), Some(su), None) => (r, su),
            _ => return Err(err()),
        };
        let rank = Rank::from_char(r).ok_or_else(err)?;
        let suit = Suit::from_char(su).ok_or_else(err)?;
        Ok(Card::new(rank, suit))
    }
}

/// Parses a whitespace-separated list of cards, e.g. `"As Kd 7c"`.
/// Rejects duplicates.
pub fn parse_cards(s: &str) -> Result<Vec<Card>, ParseCardError> {
    let mut set = CardSet::new();
    let mut out = Vec::new();
    for tok in s.split_whitespace() {
        let c: Card = tok.parse()?;
        if !set.insert(c) {
            return Err(ParseCardError(format!("duplicate card {c} in {s:?}")));
        }
        out.push(c);
    }
    Ok(out)
}

/// Error describing the first duplicate card found in a collection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DuplicateCard(pub Card);

impl fmt::Display for DuplicateCard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "duplicate card {}", self.0)
    }
}

impl std::error::Error for DuplicateCard {}

/// A set of cards as a 52-bit mask (bit `i` = card with index `i`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct CardSet(u64);

impl CardSet {
    const FULL_MASK: u64 = (1u64 << NUM_CARDS) - 1;

    /// The empty set.
    pub const fn new() -> CardSet {
        CardSet(0)
    }

    /// The set of all 52 cards.
    pub const fn full() -> CardSet {
        CardSet(Self::FULL_MASK)
    }

    /// Builds a set from a raw mask; bits above 51 are rejected.
    pub const fn from_bits(bits: u64) -> Option<CardSet> {
        if bits & !Self::FULL_MASK == 0 {
            Some(CardSet(bits))
        } else {
            None
        }
    }

    /// Raw mask.
    pub const fn bits(self) -> u64 {
        self.0
    }

    /// Builds a set from cards, failing on the first duplicate.
    pub fn try_from_cards(cards: &[Card]) -> Result<CardSet, DuplicateCard> {
        let mut set = CardSet::new();
        for &c in cards {
            if !set.insert(c) {
                return Err(DuplicateCard(c));
            }
        }
        Ok(set)
    }

    /// Inserts a card; returns `true` if it was not already present.
    pub fn insert(&mut self, card: Card) -> bool {
        let bit = 1u64 << card.index();
        let fresh = self.0 & bit == 0;
        self.0 |= bit;
        fresh
    }

    /// Removes a card; returns `true` if it was present.
    pub fn remove(&mut self, card: Card) -> bool {
        let bit = 1u64 << card.index();
        let present = self.0 & bit != 0;
        self.0 &= !bit;
        present
    }

    /// Whether the card is in the set.
    pub const fn contains(self, card: Card) -> bool {
        self.0 & (1u64 << card.0) != 0
    }

    /// Number of cards in the set.
    pub const fn len(self) -> usize {
        self.0.count_ones() as usize
    }

    /// Whether the set is empty.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Set union.
    pub const fn union(self, other: CardSet) -> CardSet {
        CardSet(self.0 | other.0)
    }

    /// Set intersection.
    pub const fn intersection(self, other: CardSet) -> CardSet {
        CardSet(self.0 & other.0)
    }

    /// Cards in `self` but not in `other`.
    pub const fn difference(self, other: CardSet) -> CardSet {
        CardSet(self.0 & !other.0)
    }

    /// Whether the two sets share no card.
    pub const fn is_disjoint(self, other: CardSet) -> bool {
        self.0 & other.0 == 0
    }

    /// Iterates cards in ascending index order.
    pub fn iter(self) -> CardSetIter {
        CardSetIter(self.0)
    }
}

impl FromIterator<Card> for CardSet {
    fn from_iter<I: IntoIterator<Item = Card>>(iter: I) -> CardSet {
        let mut set = CardSet::new();
        for c in iter {
            set.insert(c);
        }
        set
    }
}

impl IntoIterator for CardSet {
    type Item = Card;
    type IntoIter = CardSetIter;
    fn into_iter(self) -> CardSetIter {
        self.iter()
    }
}

/// Iterator over a [`CardSet`] in ascending index order.
#[derive(Clone, Debug)]
pub struct CardSetIter(u64);

impl Iterator for CardSetIter {
    type Item = Card;
    fn next(&mut self) -> Option<Card> {
        if self.0 == 0 {
            return None;
        }
        let i = self.0.trailing_zeros() as u8;
        self.0 &= self.0 - 1;
        Some(Card(i))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let n = self.0.count_ones() as usize;
        (n, Some(n))
    }
}

impl ExactSizeIterator for CardSetIter {}

/// Whether a slice of cards contains any duplicate.
pub fn has_duplicates(cards: &[Card]) -> bool {
    CardSet::try_from_cards(cards).is_err()
}

/// Returns the first card that repeats an earlier one, if any.
pub fn find_duplicate(cards: &[Card]) -> Option<Card> {
    CardSet::try_from_cards(cards).err().map(|d| d.0)
}
