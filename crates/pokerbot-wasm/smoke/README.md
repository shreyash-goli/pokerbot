# pokerbot-wasm Node smoke test

This checks the real wasm build (not the native crate) through its JS API.

```sh
export PATH="$HOME/.cargo/bin:$PATH"
# Build into an ignored or scratch directory. /pkg/ and /target/ are gitignored.
wasm-pack build crates/pokerbot-wasm --target nodejs --out-dir ../../target/wasm-node
node crates/pokerbot-wasm/smoke/smoke.mjs target/wasm-node artifacts/smoke_20bb.pokerbot
```

`--out-dir` is relative to the crate directory. The policy argument is
optional: without it only the baseline section runs. The script exits
nonzero if any check fails.

What it checks:
- Construction errors: missing or garbage policy for `"cfr"`, a bad depth, a
  bad bot, and a policy trained for a different depth.
- Native parity. A check/call session (seed 7, 20 hands, baseline bot) is
  hashed with FNV-1a 64 over every snapshot and history JSON. The digest must
  equal `BASELINE_DIGEST_SEED7` in `tests/parity.rs`, which the native test
  asserts too. Changing any snapshot text changes it.
- 20 random-legal hands with the baseline bot and 20 with the CFR bot. It
  checks chip conservation, zero-sum payoffs, and typed errors with no state
  change for unknown, illegal and out-of-turn input. It checks there are no
  `botCards` before a terminal state or after a fold, and that revealed
  showdown cards never appeared earlier in that hand's snapshots or history.
- It prints the time per bot decision in wasm (synchronous, on the calling
  thread).
