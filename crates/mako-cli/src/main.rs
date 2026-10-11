//! `mako` terminal game — placeholder binary created by M-00.
//! The playable game is implemented by M-05 and later tasks.

fn main() {
    println!(
        "mako {} — scaffold only; the game is not implemented yet.",
        env!("CARGO_PKG_VERSION")
    );
}

// Scaffold: reference workspace dependencies so the crate graph is exercised.
const _: &str = mako_core::VERSION;
const _: &str = mako_cfr::VERSION;
const _: &str = mako_analysis::VERSION;
