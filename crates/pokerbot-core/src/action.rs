//! Seats, action records, and the results and errors of applying actions.

use std::fmt;

pub use crate::rules::{ActionKind, LegalAction};
use crate::rules::{Chips, RulesError};

/// One of the two physical seats. Which seat is the button changes every
/// hand; see [`crate::state::HandConfig`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Seat {
    Zero,
    One,
}

impl Seat {
    /// Both seats in index order.
    pub const ALL: [Seat; 2] = [Seat::Zero, Seat::One];

    /// The other seat.
    pub const fn other(self) -> Seat {
        match self {
            Seat::Zero => Seat::One,
            Seat::One => Seat::Zero,
        }
    }

    /// Array index: Zero = 0, One = 1.
    pub const fn index(self) -> usize {
        match self {
            Seat::Zero => 0,
            Seat::One => 1,
        }
    }

    /// Seat from an index (0 or 1).
    pub const fn from_index(i: usize) -> Option<Seat> {
        match i {
            0 => Some(Seat::Zero),
            1 => Some(Seat::One),
            _ => None,
        }
    }
}

/// The button for the hand after one played with button `prev`.
pub const fn next_button(prev: Seat) -> Seat {
    prev.other()
}

/// Street or terminal phase of a hand. `Showdown` is reported for **every**
/// terminal state (fold or showdown); use
/// [`crate::state::HandState::terminal_reason`] to tell them apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Street {
    Preflop,
    Flop,
    Turn,
    River,
    Showdown,
}

impl Street {
    /// Converts a betting street from `rules`.
    pub const fn from_betting(s: crate::rules::Street) -> Street {
        match s {
            crate::rules::Street::Preflop => Street::Preflop,
            crate::rules::Street::Flop => Street::Flop,
            crate::rules::Street::Turn => Street::Turn,
            crate::rules::Street::River => Street::River,
        }
    }

    /// The betting street, or `None` for `Showdown`.
    pub const fn betting(self) -> Option<crate::rules::Street> {
        match self {
            Street::Preflop => Some(crate::rules::Street::Preflop),
            Street::Flop => Some(crate::rules::Street::Flop),
            Street::Turn => Some(crate::rules::Street::Turn),
            Street::River => Some(crate::rules::Street::River),
            Street::Showdown => None,
        }
    }
}

/// How a finished hand ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TerminalReason {
    /// The other player folded.
    Fold { winner: Seat },
    /// Cards were compared (after river betting or an all-in runout).
    Showdown,
}

/// A public record of one voluntary action (blinds are not recorded here;
/// they are implied by the config and button).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ActionRecord {
    pub seat: Seat,
    pub street: Street,
    pub kind: ActionKind,
    /// Seat's street contribution after the action.
    pub raise_to: Chips,
    /// Chips moved from the seat's stack by the action.
    pub chips_committed: Chips,
}

/// What happened as a result of an accepted action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActionOutcome {
    /// The record appended to the history.
    pub record: ActionRecord,
    /// Whether this action closed the betting round.
    pub round_closed: bool,
    /// The street (or `Showdown`) the hand is now on.
    pub street: Street,
    /// Set when the hand ended with this action.
    pub terminal: Option<TerminalReason>,
}

/// Rejection of an action. The state is unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionError {
    /// The hand is already over.
    HandOver,
    /// The action class is not currently legal.
    Illegal(ActionKind),
    /// The supplied `LegalAction` does not exactly match a legal entry.
    Mismatch(LegalAction),
}

impl fmt::Display for ActionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ActionError::HandOver => write!(f, "the hand is over"),
            ActionError::Illegal(k) => write!(f, "{k:?} is not legal now"),
            ActionError::Mismatch(a) => write!(f, "{a:?} does not match a legal action"),
        }
    }
}

impl std::error::Error for ActionError {}

/// Errors creating a hand.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HandError {
    /// The rules config failed validation.
    Config(RulesError),
    /// The deck had fewer than the nine cards a hand needs.
    DeckTooSmall { remaining: usize },
    /// A starting stack was zero or above the maximum.
    InvalidStack(Chips),
}

impl fmt::Display for HandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HandError::Config(e) => write!(f, "{e}"),
            HandError::DeckTooSmall { remaining } => {
                write!(f, "deck has {remaining} cards; a hand needs 9")
            }
            HandError::InvalidStack(s) => write!(f, "invalid starting stack {s}"),
        }
    }
}

impl std::error::Error for HandError {}
