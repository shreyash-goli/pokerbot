//! `mako-cfr` — placeholder crate created by M-00 (workspace scaffold).
//!
//! Real functionality is added by later tasks in `docs/TASKS.json`.

/// Crate version, exposed so the scaffold has a testable item.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

// Scaffold: reference workspace dependencies so the crate graph is exercised.
const _: &str = mako_core::VERSION;

#[cfg(test)]
mod tests {
    #[test]
    fn scaffold_version_is_set() {
        assert!(!super::VERSION.is_empty());
    }
}
