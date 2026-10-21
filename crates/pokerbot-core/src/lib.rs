//! `pokerbot-core` — heads-up no-limit hold'em engine primitives.
//!
//! * [`action`] — seats, action records, action errors
//! * [`card`] — cards and card sets
//! * [`deck`] — seeded portable shuffle and dealing
//! * [`events`] — replayable event stream with per-viewer visibility
//! * [`eval`] — hand evaluator: categories, best five of seven
//! * [`observation`] — per-seat observation, the hidden-information boundary
//! * [`replay`] — hand logs, deterministic replay, public-state rebuild
//! * [`rng`] — deterministic xoshiro256** RNG
//! * [`rules`] — betting rules, legal actions, chip accounting
//! * [`state`] — authoritative hand state machine
//!
//! The crate has no dependencies and builds for `wasm32-unknown-unknown`.

pub mod action;
pub mod card;
pub mod deck;
pub mod eval;
pub mod events;
pub mod observation;
pub mod replay;
pub mod rng;
pub mod rules;
pub mod state;

/// Crate version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    #[test]
    fn scaffold_version_is_set() {
        assert!(!super::VERSION.is_empty());
    }
}
