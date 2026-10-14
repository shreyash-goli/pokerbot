//! `mako-core` — heads-up no-limit hold'em engine primitives.
//!
//! * [`action`] — seats, action records, action errors (M-04)
//! * [`card`] — cards and card sets (M-02)
//! * [`deck`] — seeded portable shuffle and dealing (M-02)
//! * [`eval`] — hand evaluator: categories, best five of seven (M-03)
//! * [`observation`] — per-seat observation, the hidden-information boundary (M-04)
//! * [`rng`] — deterministic xoshiro256** RNG (M-02)
//! * [`rules`] — betting rules, legal actions, chip accounting (M-01)
//! * [`state`] — authoritative hand state machine (M-04)
//!
//! The crate has no dependencies and builds for `wasm32-unknown-unknown`.

pub mod action;
pub mod card;
pub mod deck;
pub mod eval;
pub mod observation;
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
