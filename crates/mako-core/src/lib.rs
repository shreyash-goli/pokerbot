//! `mako-core` — placeholder crate created by M-00 (workspace scaffold).
//!
//! Real functionality is added by later tasks in `docs/TASKS.json`.

/// Crate version, exposed so the scaffold has a testable item.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    #[test]
    fn scaffold_version_is_set() {
        assert!(!super::VERSION.is_empty());
    }
}
