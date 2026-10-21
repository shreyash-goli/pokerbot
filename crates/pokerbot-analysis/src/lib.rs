//! `pokerbot-analysis` — hand history, session statistics and the post-hand
//! coach, all from information a player may legitimately know.
//!
//! * [`history`] — [`history::HandRecord`], built from the human seat's
//!   viewer-filtered events, and the `pokerbot-history v1` text format.
//! * [`stats`] — [`stats::SessionStats`]: net chips, BB/100, VPIP, PFR,
//!   aggression, showdowns and positional splits from public actions.
//! * [`store`] — atomic local persistence and the CLI-facing
//!   [`store::SessionRecorder`].
//! * [`coach`] — post-hand review separating decision-time information from
//!   hindsight, and replay frames.
//! * [`equity`] — showdown equity vs a random or a shown hand.
//! * [`opponent`] — opt-in, experimental opponent tendency model and
//!   [`opponent::AdaptiveAgent`], from public actions only.

pub mod coach;
pub mod equity;
pub mod history;
pub mod opponent;
pub mod stats;
pub mod store;

pub use history::{HandRecord, HistoryError, OpponentLabel};
pub use stats::{PlayerStats, Role, SessionStats, SplitStats};
pub use store::{LoadNotice, ResetOutcome, SessionRecorder, SessionStore, StoreError};

/// Crate version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
