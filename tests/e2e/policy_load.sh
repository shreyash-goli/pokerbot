#!/usr/bin/env bash
# M-24 end-to-end policy loading across binaries.
#
#   tests/e2e/policy_load.sh <bin-dir> <out-dir> [<wasm-node-pkg-dir>]
#
# Trains the CI tiny artifact with the mako-train BINARY from
# configs/ci_tiny.toml, then checks that it
#   * loads in the terminal game (mako --bot cfr --strategy) and is labelled
#     as a weak, plumbing-only, not-GTO policy,
#   * loads in mako-eval (--smoke --artifact) and never promotes,
#   * loads in mako-wasm under Node (crates/mako-wasm/smoke/smoke.mjs),
# and that a wrong-depth load is rejected by all three. Every `mako` child
# runs with --no-history and a sandboxed HOME/MAKO_HOME under <out-dir>.
# Run from anywhere; exits nonzero on the first failed check.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
bin="${1:?usage: policy_load.sh <bin-dir> <out-dir> [wasm-node-pkg]}"
out="${2:?usage: policy_load.sh <bin-dir> <out-dir> [wasm-node-pkg]}"
pkg="${3:-}"
cd "$root"
mkdir -p "$out"
sandbox="$out/home"
mkdir -p "$sandbox"

fail() { echo "policy_load: FAIL: $*" >&2; exit 1; }
expect_code() { # expect_code <want> <got> <what>
  [[ "$2" == "$1" ]] || fail "$3: exit $2, expected $1"
}

echo "== train ci_tiny with the mako-train binary"
"$bin/mako-train" --config configs/ci_tiny.toml --out "$out/train" --quiet
art="$out/train/ci_tiny_20bb.mako"
[[ -s "$art" ]] || fail "artifact not written"

echo "== terminal game loads it"
set +e
HOME="$sandbox" MAKO_HOME="$sandbox" "$bin/mako" --bot cfr --strategy "$art" \
  --seed 3 --hands 3 --no-color --fast --auto checkcall --no-history \
  >"$out/cli.out" 2>"$out/cli.err"
code=$?
set -e
expect_code 0 "$code" "mako --bot cfr"
grep -q "Opponent: CFR policy" "$out/cli.out" || fail "CLI did not use the CFR policy"
grep -q "ci: tiny plumbing fixture, weak, not GTO" "$out/cli.out" || fail "CLI label missing"
grep -q "plumbing-only smoke policy -- not competitive, not GTO / Nash" "$out/cli.out" \
  || fail "CLI weak-policy note missing"
grep -q "Session complete." "$out/cli.out" || fail "CLI session did not complete"

echo "== terminal game rejects it at 50BB"
set +e
HOME="$sandbox" MAKO_HOME="$sandbox" "$bin/mako" --bot cfr --strategy "$art" --depth 50 \
  --seed 3 --hands 1 --no-color --auto checkcall --no-history >/dev/null 2>"$out/cli50.err"
code=$?
set -e
expect_code 2 "$code" "mako --depth 50"
grep -q "wrong depth" "$out/cli50.err" || fail "CLI wrong-depth message missing"

echo "== mako-eval loads it and holds it"
set +e
"$bin/mako-eval" --smoke --artifact "$art" --pairs 5 >"$out/eval.out" 2>&1
code=$?
"$bin/mako-eval" --smoke --artifact "$art" --pairs 5 --require-promote >/dev/null 2>&1
held=$?
set -e
expect_code 0 "$code" "mako-eval --smoke --artifact"
expect_code 3 "$held" "mako-eval --require-promote on an unpinned artifact"
grep -q "label      ci: tiny plumbing fixture, weak, not GTO" "$out/eval.out" || fail "eval label"
grep -q "promotion gate: HOLD" "$out/eval.out" || fail "eval must HOLD"
grep -q "No exploitability" "$out/eval.out" || fail "eval disclaimer missing"

echo "== mako-eval rejects it in a 50BB config"
sed -e 's/^depth_bb = 20/depth_bb = 50/' configs/eval_smoke.toml >"$out/eval50.toml"
set +e
"$bin/mako-eval" --config "$out/eval50.toml" --artifact "$art" --pairs 5 >"$out/eval50.out" 2>&1
code=$?
set -e
expect_code 2 "$code" "mako-eval 50BB config"
grep -q "wrong depth" "$out/eval50.out" || fail "eval wrong-depth message missing"

if [[ -n "$pkg" ]]; then
  echo "== mako-wasm (Node) loads it; smoke.mjs also asserts the 50BB rejection"
  node crates/mako-wasm/smoke/smoke.mjs "$pkg" "$art" >"$out/wasm.out" 2>&1 \
    || { cat "$out/wasm.out" >&2; fail "wasm smoke with ci_tiny"; }
  grep -q "SMOKE OK" "$out/wasm.out" || fail "wasm smoke not OK"
  grep -q 'label "ci: tiny plumbing fixture, weak, not GTO"' "$out/wasm.out" || fail "wasm label"
else
  echo "== mako-wasm: SKIPPED (no wasm-node package dir given)"
fi
echo "policy_load: OK"
