# pokerbot

An independently implemented heads-up no-limit Texas Hold'em game ("Mako") with a
Rust rules engine, a from-scratch external-sampling linear MCCFR bot, a retro
terminal UI, evaluation tools and a WebAssembly/React embed.

> Status: **scaffold only (M-00)**. No poker logic, trained policy or browser
> integration exists yet. See `docs/TASKS.json` and `docs/PROGRESS.md`.

## Layout

| Crate | Purpose |
|-------|---------|
| `crates/mako-core` | Cards, deck/RNG, evaluator, hand state machine, observations |
| `crates/mako-cfr` | Kuhn-poker CFR reference, abstraction, MCCFR trainer, strategy artifacts |
| `crates/mako-cli` | Terminal game (`mako` binary) |
| `crates/mako-analysis` | History, stats, coach, opponent model |
| `crates/mako-eval` | Policy evaluation harness |
| `crates/mako-wasm` | wasm-bindgen browser bridge |

## Build

Requires [rustup](https://rustup.rs); `rust-toolchain.toml` pins the toolchain
(with `rustfmt`, `clippy`, `wasm32-unknown-unknown`).

```bash
make ci        # fmt-check, check, clippy, test, task-graph validation
cargo test --workspace
```

## Provenance

All code is newly written for this repository. It is inspired at the feature level by
[PvRao-29/Mako](https://github.com/PvRao-29/Mako), but no source, tests, binaries
or strategy files from that project are used. The CFR methods follow the published
literature (Zinkevich et al. 2007; Lanctot et al. 2009, "Monte Carlo Sampling for
Regret Minimization in Extensive Games"; Brown & Sandholm 2019, linear CFR).

The trained bot is an abstraction-based approximation. It is **not** a GTO or Nash
equilibrium strategy. Simulation only; no real money.
