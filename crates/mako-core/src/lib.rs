//! `mako-core` — heads-up no-limit hold'em engine primitives.
//!
//! * [`card`] — cards and card sets (M-02)
//! * [`deck`] — seeded portable shuffle and dealing (M-02)
//! * [`rng`] — deterministic xoshiro256** RNG (M-02)
//! * [`rules`] — betting rules, legal actions, chip accounting (M-01)
//!
//! The crate has no dependencies and builds for `wasm32-unknown-unknown`.

pub mod card;
pub mod deck;
pub mod rng;
pub mod rules;

/// Crate version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    #[test]
    fn scaffold_version_is_set() {
        assert!(!super::VERSION.is_empty());
    }
}
