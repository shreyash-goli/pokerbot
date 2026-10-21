//! Information-set keys and the card-abstraction interface.
//!
//! An [`InfoKey`] is what the acting player legally knows, in abstracted
//! form: betting street, own position, own card bucket (from own hole cards
//! and the *revealed* board), and the public [`HistoryKey`]. The only
//! live-play constructor is [`InfoKey::from_observation`], which reads a
//! [`PlayerObservation`] and nothing else, so opponent hole cards and undealt
//! board cards cannot reach a key.
//!
//! # Byte layout (format version 1, stable)
//! | offset | size | field                                             |
//! |--------|------|---------------------------------------------------|
//! | 0      | 1    | format version = [`KEY_FORMAT_VERSION`]           |
//! | 1      | 1    | street: Preflop 0, Flop 1, Turn 2, River 3        |
//! | 2      | 1    | position: Button 0, BigBlind 1                    |
//! | 3      | 2    | bucket, `u16` little-endian                       |
//! | 5      | 1    | history length `n` (`<= MAX_HISTORY_BYTES` = 135) |
//! | 6      | n    | history bytes ([`HistoryKey::as_bytes`])          |
//!
//! # Hash
//! [`InfoKey::hash64`] is 64-bit FNV-1a (Fowler–Noll–Vo; offset basis
//! `0xcbf29ce484222325`, prime `0x100000001b3`) over those bytes. It is
//! integer-only, so it is identical on every platform including wasm32. It is
//! not collision-free; tables that need exactness should key on the bytes.

use std::fmt;

use pokerbot_core::card::Card;
use pokerbot_core::observation::PlayerObservation;
use pokerbot_core::rules::{Position, Street};

use super::history::{HistoryError, HistoryKey, MAX_HISTORY_BYTES};
use super::preflop::{NUM_PREFLOP_CLASSES, PreflopClass};

/// Current key byte-format version.
pub const KEY_FORMAT_VERSION: u8 = 1;

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// 64-bit FNV-1a over `bytes`.
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h = FNV_OFFSET;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(FNV_PRIME);
    }
    h
}

/// Errors from a card abstraction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AbstractionError {
    /// This abstraction has no buckets for the street.
    UnsupportedStreet(Street),
    /// Board length does not match the street.
    BoardLength { street: Street, len: usize },
}

impl fmt::Display for AbstractionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AbstractionError::UnsupportedStreet(s) => {
                write!(f, "card abstraction has no buckets for {s:?}")
            }
            AbstractionError::BoardLength { street, len } => {
                write!(f, "{len} board cards is invalid for {street:?}")
            }
        }
    }
}

impl std::error::Error for AbstractionError {}

/// Number of revealed board cards on each street.
pub fn board_len(street: Street) -> usize {
    match street {
        Street::Preflop => 0,
        Street::Flop => 3,
        Street::Turn => 4,
        Street::River => 5,
    }
}

/// Maps a player's own hole cards plus the revealed board to a bucket.
///
/// Implementations receive only the acting player's cards and public board
/// cards. `num_buckets(street) == 0` means the street is unsupported, and
/// `bucket` must then return [`AbstractionError::UnsupportedStreet`].
pub trait CardAbstraction {
    /// Bucket in `0..num_buckets(street)`.
    fn bucket(
        &self,
        street: Street,
        hole: [Card; 2],
        board: &[Card],
    ) -> Result<u16, AbstractionError>;

    /// Number of buckets on `street` (0 if unsupported).
    fn num_buckets(&self, street: Street) -> u16;
}

/// Preflop-only abstraction: the bucket is the [`PreflopClass`] index.
///
/// Postflop streets are **unsupported** here (`num_buckets = 0`, `bucket`
/// errors); real postflop buckets come from the postflop abstraction, so a
/// placeholder can never be mistaken for one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PreflopClassAbstraction;

impl CardAbstraction for PreflopClassAbstraction {
    fn bucket(
        &self,
        street: Street,
        hole: [Card; 2],
        board: &[Card],
    ) -> Result<u16, AbstractionError> {
        if street != Street::Preflop {
            return Err(AbstractionError::UnsupportedStreet(street));
        }
        if !board.is_empty() {
            return Err(AbstractionError::BoardLength {
                street,
                len: board.len(),
            });
        }
        Ok(u16::from(PreflopClass::of(hole).index()))
    }

    fn num_buckets(&self, street: Street) -> u16 {
        if street == Street::Preflop {
            NUM_PREFLOP_CLASSES as u16
        } else {
            0
        }
    }
}

/// Errors building or decoding an [`InfoKey`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyError {
    /// The hand is over; there is no decision.
    Terminal,
    /// The observing seat is not the one to act.
    NotActing,
    /// The card abstraction failed.
    Abstraction(AbstractionError),
    /// The abstraction returned a bucket `>= num_buckets`.
    BucketOutOfRange { bucket: u16, num_buckets: u16 },
    /// History encoding failed.
    History(HistoryError),
    /// Malformed serialized key.
    Malformed(&'static str),
}

impl fmt::Display for KeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KeyError::Terminal => write!(f, "no information set at a terminal state"),
            KeyError::NotActing => write!(f, "observing seat is not to act"),
            KeyError::Abstraction(e) => write!(f, "{e}"),
            KeyError::BucketOutOfRange {
                bucket,
                num_buckets,
            } => write!(f, "bucket {bucket} out of range 0..{num_buckets}"),
            KeyError::History(e) => write!(f, "{e}"),
            KeyError::Malformed(m) => write!(f, "malformed key bytes: {m}"),
        }
    }
}

impl std::error::Error for KeyError {}

impl From<AbstractionError> for KeyError {
    fn from(e: AbstractionError) -> Self {
        KeyError::Abstraction(e)
    }
}

impl From<HistoryError> for KeyError {
    fn from(e: HistoryError) -> Self {
        KeyError::History(e)
    }
}

/// Information-set key for the acting player.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InfoKey {
    street: Street,
    position: Position,
    bucket: u16,
    history: HistoryKey,
}

fn street_byte(s: Street) -> u8 {
    match s {
        Street::Preflop => 0,
        Street::Flop => 1,
        Street::Turn => 2,
        Street::River => 3,
    }
}

impl InfoKey {
    /// Build the key for the seat that owns `obs`, which must be to act.
    ///
    /// Reads only `seat`, `button`, `street`, `hole_cards` (own), `board`
    /// (revealed), `history` (public), `to_act` and `terminal`. Never reads
    /// `opponent_hole_cards`.
    pub fn from_observation(
        obs: &PlayerObservation,
        abstraction: &dyn CardAbstraction,
    ) -> Result<InfoKey, KeyError> {
        if obs.terminal.is_some() {
            return Err(KeyError::Terminal);
        }
        if !obs.is_my_turn() {
            return Err(KeyError::NotActing);
        }
        let street = obs.street.betting().ok_or(KeyError::Terminal)?;
        if obs.board.len() != board_len(street) {
            return Err(AbstractionError::BoardLength {
                street,
                len: obs.board.len(),
            }
            .into());
        }
        let position = if obs.seat == obs.button {
            Position::Button
        } else {
            Position::BigBlind
        };
        let bucket = abstraction.bucket(street, obs.hole_cards, &obs.board)?;
        let num_buckets = abstraction.num_buckets(street);
        if bucket >= num_buckets {
            return Err(KeyError::BucketOutOfRange {
                bucket,
                num_buckets,
            });
        }
        let history = HistoryKey::from_records(&obs.history, street)?;
        Ok(InfoKey {
            street,
            position,
            bucket,
            history,
        })
    }

    /// Betting street.
    pub fn street(&self) -> Street {
        self.street
    }

    /// Acting player's position.
    pub fn position(&self) -> Position {
        self.position
    }

    /// Card bucket.
    pub fn bucket(&self) -> u16 {
        self.bucket
    }

    /// Public history.
    pub fn history(&self) -> &HistoryKey {
        &self.history
    }

    /// Stable serialization (see module docs).
    pub fn to_bytes(&self) -> Vec<u8> {
        let h = self.history.as_bytes();
        let mut out = Vec::with_capacity(6 + h.len());
        out.push(KEY_FORMAT_VERSION);
        out.push(street_byte(self.street));
        out.push(match self.position {
            Position::Button => 0,
            Position::BigBlind => 1,
        });
        out.extend_from_slice(&self.bucket.to_le_bytes());
        out.push(h.len() as u8);
        out.extend_from_slice(h);
        out
    }

    /// Decode [`InfoKey::to_bytes`] output (for artifacts and tooling, not
    /// live play). Checks structure; bucket range is checked against an
    /// abstraction by the caller.
    pub fn from_bytes(bytes: &[u8]) -> Result<InfoKey, KeyError> {
        if bytes.len() < 6 {
            return Err(KeyError::Malformed("too short"));
        }
        if bytes[0] != KEY_FORMAT_VERSION {
            return Err(KeyError::Malformed("unknown format version"));
        }
        let street = *Street::ALL
            .get(usize::from(bytes[1]))
            .ok_or(KeyError::Malformed("bad street"))?;
        let position = match bytes[2] {
            0 => Position::Button,
            1 => Position::BigBlind,
            _ => return Err(KeyError::Malformed("bad position")),
        };
        let bucket = u16::from_le_bytes([bytes[3], bytes[4]]);
        let n = usize::from(bytes[5]);
        if n > MAX_HISTORY_BYTES || bytes.len() != 6 + n {
            return Err(KeyError::Malformed("bad history length"));
        }
        let history = HistoryKey::from_bytes(&bytes[6..])?;
        if history.street() != street {
            return Err(KeyError::Malformed("street does not match history"));
        }
        Ok(InfoKey {
            street,
            position,
            bucket,
            history,
        })
    }

    /// Portable 64-bit FNV-1a hash of [`InfoKey::to_bytes`].
    pub fn hash64(&self) -> u64 {
        fnv1a64(&self.to_bytes())
    }
}

impl fmt::Display for InfoKey {
    /// e.g. `BTN#13 Pre:` or `BB#7 Pre:c-k|Flop:`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let pos = match self.position {
            Position::Button => "BTN",
            Position::BigBlind => "BB",
        };
        write!(f, "{pos}#{} {}", self.bucket, self.history)
    }
}
