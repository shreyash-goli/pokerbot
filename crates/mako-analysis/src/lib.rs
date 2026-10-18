//! `mako-analysis` — hand history, session statistics and (later) coaching
//! and opponent modelling, all from information a player may legitimately
//! know.
//!
//! * [`history`] — [`history::HandRecord`], built from the human seat's
//!   viewer-filtered events, and the `mako-history v1` text format (M-17).
//! * [`stats`] — [`stats::SessionStats`]: net chips, BB/100, VPIP, PFR,
//!   aggression, showdowns and positional splits from public actions (M-17).
//! * [`store`] — atomic local persistence and the CLI-facing
//!   [`store::SessionRecorder`] (M-17).

pub mod history;
pub mod stats;
pub mod store;

pub use history::{HandRecord, HistoryError, OpponentLabel};
pub use stats::{PlayerStats, Role, SessionStats, SplitStats};
pub use store::{LoadNotice, ResetOutcome, SessionRecorder, SessionStore, StoreError};

/// Crate version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
