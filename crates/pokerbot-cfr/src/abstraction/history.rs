//! Public betting-history encoding for information-set keys.
//!
//! A [`HistoryKey`] is built **only** from the public action records in a
//! player's observation ([`ActionRecord`] has seat, street, action class and
//! public chip amounts; no cards). It keeps the sequence of [`ActionKind`]s
//! per street plus the street boundaries. Chip amounts are left out because,
//! under the six-action abstraction, they are a deterministic function of the
//! action-class sequence, the blinds and the starting stack (`GAME_RULES.md`
//! §5).
//!
//! # What is (not) in the history
//! - **Street boundaries: included.** One separator byte per street change, so
//!   `"Pre:c-k|Flop:"` (flop, nothing yet) differs from `"Pre:c-k"`.
//! - **Acting position: not in the history.** It is stored explicitly in
//!   [`crate::abstraction::key::InfoKey`]; that is simpler than deriving it
//!   from parity, which breaks when all-in players stop acting.
//! - **Stack depth and blinds: not in keys.** A strategy artifact is trained
//!   for one `RulesConfig` (blinds + equal starting stacks), which is recorded
//!   in artifact metadata. Keys from a different depth must not be looked up
//!   in that artifact. Unequal stacks (`HandState::new_with_stacks`, used in
//!   tests only) are outside the abstraction.
//!
//! # Byte encoding (stable)
//! One byte per item: an action is its canonical `ActionKind::index()`
//! (`Fold 0, Check 1, Call 2, HalfPot 3, Pot 4, AllIn 5`); a street boundary is
//! [`STREET_SEPARATOR`] (`0x0F`). The number of separators equals the index of
//! the current street (Preflop 0 … River 3).
//!
//! # Length bound
//! On one street every non-final aggressive action at least doubles the
//! aggressor's street contribution (pot-after-call sizing gives
//! `raise_to >= 2·opp_street`; an opening bet is `>= big_blind`). So a street
//! has at most `k = floor(log2(S / bb)) + 2` aggressive actions (`S` = larger
//! starting stack) plus at most two passive ones (an opening check/limp and the
//! closing call/check). With four streets and three separators:
//! `len <= 4·(k + 2) + 3` — see [`max_history_len`]. For 100 BB (`S = 200`,
//! `bb = 2`) that is 43 bytes; the global ceiling over all valid configs is
//! [`MAX_HISTORY_BYTES`] = 135, so a length always fits in one byte.

use std::fmt;

use pokerbot_core::action::ActionRecord;
use pokerbot_core::rules::{ActionKind, Chips, Street};

/// Byte marking the start of the next street.
pub const STREET_SEPARATOR: u8 = 0x0F;

/// Upper bound on encoded history length for any valid config
/// (`max_history_len(pokerbot_core::rules::MAX_STARTING_STACK, 1)`).
pub const MAX_HISTORY_BYTES: usize = 135;

/// Bound on history bytes for starting stack `stack` and big blind `bb`.
pub fn max_history_len(stack: Chips, big_blind: Chips) -> usize {
    let bb = big_blind.max(1);
    let ratio = (stack / bb).max(1);
    let log2 = (u32::BITS - 1 - ratio.leading_zeros()) as usize;
    let k = log2 + 2;
    4 * (k + 2) + 3
}

/// Errors building or decoding a [`HistoryKey`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HistoryError {
    /// Records are not in street order, or past the current street.
    StreetOrder,
    /// A byte is neither an action index nor the separator.
    BadByte(u8),
    /// More than three separators.
    TooManyStreets,
    /// Longer than [`MAX_HISTORY_BYTES`].
    TooLong(usize),
}

impl fmt::Display for HistoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HistoryError::StreetOrder => write!(f, "action records out of street order"),
            HistoryError::BadByte(b) => write!(f, "invalid history byte {b:#04x}"),
            HistoryError::TooManyStreets => write!(f, "more than four streets"),
            HistoryError::TooLong(n) => write!(f, "history of {n} bytes exceeds bound"),
        }
    }
}

impl std::error::Error for HistoryError {}

fn street_index(s: Street) -> usize {
    match s {
        Street::Preflop => 0,
        Street::Flop => 1,
        Street::Turn => 2,
        Street::River => 3,
    }
}

fn kind_char(k: ActionKind) -> char {
    match k {
        ActionKind::Fold => 'f',
        ActionKind::Check => 'k',
        ActionKind::Call => 'c',
        ActionKind::HalfPot => 'h',
        ActionKind::Pot => 'p',
        ActionKind::AllIn => 'a',
    }
}

/// Public betting history: action classes per street with street separators.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct HistoryKey {
    bytes: Vec<u8>,
}

impl HistoryKey {
    /// Encode public action records, ending on `current` street.
    ///
    /// Takes only public records; there is no parameter through which cards
    /// could enter.
    pub fn from_records(records: &[ActionRecord], current: Street) -> Result<Self, HistoryError> {
        let target = street_index(current);
        let mut bytes = Vec::with_capacity(records.len() + target);
        let mut cur = 0usize;
        for r in records {
            let s = r.street.betting().ok_or(HistoryError::StreetOrder)?;
            let si = street_index(s);
            if si < cur || si > target {
                return Err(HistoryError::StreetOrder);
            }
            while cur < si {
                bytes.push(STREET_SEPARATOR);
                cur += 1;
            }
            bytes.push(r.kind.index() as u8);
        }
        while cur < target {
            bytes.push(STREET_SEPARATOR);
            cur += 1;
        }
        if bytes.len() > MAX_HISTORY_BYTES {
            return Err(HistoryError::TooLong(bytes.len()));
        }
        Ok(HistoryKey { bytes })
    }

    /// Stable byte serialization.
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Decode bytes produced by [`HistoryKey::as_bytes`]. Validates syntax.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, HistoryError> {
        if bytes.len() > MAX_HISTORY_BYTES {
            return Err(HistoryError::TooLong(bytes.len()));
        }
        let mut seps = 0;
        for &b in bytes {
            if b == STREET_SEPARATOR {
                seps += 1;
                if seps > 3 {
                    return Err(HistoryError::TooManyStreets);
                }
            } else if usize::from(b) >= ActionKind::ALL.len() {
                return Err(HistoryError::BadByte(b));
            }
        }
        Ok(HistoryKey {
            bytes: bytes.to_vec(),
        })
    }

    /// Length in bytes.
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    /// True for the empty preflop history.
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Current street (number of separators).
    pub fn street(&self) -> Street {
        let n = self
            .bytes
            .iter()
            .filter(|&&b| b == STREET_SEPARATOR)
            .count();
        Street::ALL[n]
    }

    /// Action classes per street, up to the current street.
    pub fn streets(&self) -> Vec<Vec<ActionKind>> {
        let mut out = vec![Vec::new()];
        for &b in &self.bytes {
            if b == STREET_SEPARATOR {
                out.push(Vec::new());
            } else {
                out.last_mut()
                    .expect("non-empty")
                    .push(ActionKind::ALL[usize::from(b)]);
            }
        }
        out
    }
}

impl fmt::Display for HistoryKey {
    /// Human-readable form, e.g. `Pre:c-k|Flop:h-c|Turn:`.
    /// Letters: f=Fold k=Check c=Call h=HalfPot p=Pot a=AllIn.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        const NAMES: [&str; 4] = ["Pre", "Flop", "Turn", "River"];
        for (i, acts) in self.streets().iter().enumerate() {
            if i > 0 {
                write!(f, "|")?;
            }
            write!(f, "{}:", NAMES[i])?;
            for (j, k) in acts.iter().enumerate() {
                if j > 0 {
                    write!(f, "-")?;
                }
                write!(f, "{}", kind_char(*k))?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pokerbot_core::rules::MAX_STARTING_STACK;

    #[test]
    fn global_bound_matches_constant() {
        assert_eq!(max_history_len(MAX_STARTING_STACK, 1), MAX_HISTORY_BYTES);
        assert_eq!(max_history_len(200, 2), 43);
    }
}
