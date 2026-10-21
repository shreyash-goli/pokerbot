#!/usr/bin/env bash
# Train the bounded 20BB smoke policy (M-13) and print its manifest.
#
# Budget (docs/PROGRESS.md): single-threaded, max_wall_secs <= 300 per run.
# The policy is a plumbing smoke test: no strength, Nash or GTO claim.
# Not invoked by `cargo test`. Extra arguments are passed to pokerbot-train
# (e.g. --resume artifacts/smoke_20bb.ckpt).
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

config="${POKERBOT_SMOKE_CONFIG:-configs/smoke.toml}"

cargo build --release -p pokerbot-cfr --bin pokerbot-train
./target/release/pokerbot-train --config "$config" "$@"

manifest="$(sed -n 's/^manifest *= *"\(.*\)".*/\1/p' "$config")"
dir="$(sed -n 's/^dir *= *"\(.*\)".*/\1/p' "$config")"
echo "---- manifest: ${dir:-artifacts}/${manifest} ----"
cat "${dir:-artifacts}/${manifest}"
