//! Information abstraction for heads-up NLHE (M-07).
//!
//! - [`preflop`]: the 169 preflop starting-hand classes.
//! - [`history`]: public betting-history encoding.
//! - [`postflop`]: equity buckets, [`AbstractionConfig`], bounded cache,
//!   and the full [`EquityAbstraction`] (M-08).
//! - [`key`]: [`InfoKey`] (street, position, card bucket, public history),
//!   its stable bytes and portable FNV-1a hash, and the [`CardAbstraction`]
//!   trait.
//!
//! Hidden-information rule: keys are built only from the acting player's
//! [`pokerbot_core::observation::PlayerObservation`], meaning own hole cards,
//! revealed board and public actions. Opponent cards and undealt board cards
//! are never inputs. Abstraction is lossy by design: it groups distinct
//! situations together and is not a claim of optimal play.

pub mod history;
pub mod key;
pub mod postflop;
pub mod preflop;

pub use history::HistoryKey;
pub use key::{CardAbstraction, InfoKey, PreflopClassAbstraction};
pub use postflop::{AbstractionConfig, EquityAbstraction};
pub use preflop::PreflopClass;
