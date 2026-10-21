#!/usr/bin/env bash
# Full local CI for Pokerbot.
#
#   scripts/ci_local.sh [--with-smoke-training] [--step <name>]...
#
# Steps, in order (each runs even if an earlier one failed; the summary
# table lists every step's status and exit code):
#   fmt            cargo fmt --all -- --check
#   clippy         cargo clippy --workspace --all-targets -- -D warnings
#   test           cargo test --workspace
#   bins           debug build of pokerbot, pokerbot-train, pokerbot-eval
#   smoke-artifact artifacts/smoke_20bb.pokerbot present? Otherwise trained by
#                  scripts/train_smoke.sh with --with-smoke-training (release
#                  build, ~2 min), else SKIPPED LOUDLY
#   eval-smoke     pokerbot-eval --smoke --require-promote (pinned smoke artifact
#                  must PROMOTE under the locked gate); needs smoke-artifact
#   wasm-build     wasm-pack build (bundler + nodejs) into $TARGET/wasm-*
#   policy-load    tests/e2e/policy_load.sh: ci_tiny trained by the
#                  pokerbot-train binary loads in pokerbot / pokerbot-eval / wasm (Node)
#                  and is rejected at the wrong depth in all three
#   wasm-smoke     node crates/pokerbot-wasm/smoke/smoke.mjs (with the smoke
#                  artifact if present: its CFR digest must equal the value
#                  recorded in docs/WEB_INTEGRATION.md)
#   browser        npm ci in packages/pokerbot-browser, then in examples/react-demo
#                  npm ci && npm run build && npm run smoke (headless local
#                  Google Chrome via playwright-core;
#                  needs the smoke artifact). SKIPPED LOUDLY without npm,
#                  Chrome or the artifact.
#
# Requirements (stated): the rust-toolchain.toml toolchain (with the
# wasm32-unknown-unknown target), wasm-pack, node (>= 22) and npm;
# Google Chrome for the browser step; network access for `npm ci`.
# CARGO_TARGET_DIR is honoured. A log is written to
# $TARGET/ci-local/ci-local-<timestamp>.log. Exit status 1 if any step FAILED.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"
target="${CARGO_TARGET_DIR:-$root/target}"
bin="$target/debug"
smoke_art="$root/artifacts/smoke_20bb.pokerbot"
# CFR check/call digest of the pinned smoke artifact (docs/WEB_INTEGRATION.md).
smoke_digest="0x62bed279a8824406"

with_training=0
only=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    --with-smoke-training) with_training=1 ;;
    --step) only+=("${2:?--step needs a name}"); shift ;;
    -h|--help) sed -n '2,35p' "$0"; exit 0 ;;
    *) echo "ci_local: unknown argument $1" >&2; exit 2 ;;
  esac
  shift
done

logdir="$target/ci-local"
mkdir -p "$logdir"
log="$logdir/ci-local-$(date +%Y%m%d-%H%M%S).log"
exec > >(tee -a "$log") 2>&1

loud_skip() {
  echo "!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!"
  echo "!! SKIPPED: $*"
  echo "!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!"
  return 77
}

have_chrome() {
  [[ -d "/Applications/Google Chrome.app" ]] && return 0
  command -v google-chrome >/dev/null 2>&1 || command -v google-chrome-stable >/dev/null 2>&1
}

step_fmt() { cargo fmt --all -- --check; }
step_clippy() { cargo clippy --workspace --all-targets -- -D warnings; }
step_test() { cargo test --workspace; }
step_bins() { cargo build -p pokerbot-cli -p pokerbot-cfr -p pokerbot-eval --bins; }
step_smoke_artifact() {
  if [[ -s "$smoke_art" ]]; then
    echo "smoke artifact present: $smoke_art ($(wc -c <"$smoke_art") bytes)"
  elif [[ $with_training == 1 ]]; then
    scripts/train_smoke.sh
  else
    loud_skip "artifacts/smoke_20bb.pokerbot is absent (gitignored). Pass --with-smoke-training" \
      "to train it (scripts/train_smoke.sh, ~2 min release build). Steps that need it" \
      "(eval-smoke, the real-artifact checks in wasm-smoke, browser) will be skipped too."
  fi
}
step_eval_smoke() {
  [[ -s "$smoke_art" ]] || { loud_skip "eval-smoke: no artifacts/smoke_20bb.pokerbot"; return; }
  "$bin/pokerbot-eval" --smoke --require-promote
}
step_wasm_build() {
  command -v wasm-pack >/dev/null || { echo "wasm-pack is required (cargo install wasm-pack)"; return 1; }
  wasm-pack build crates/pokerbot-wasm --target bundler --out-dir "$target/wasm-bundler"
  wasm-pack build crates/pokerbot-wasm --target nodejs --out-dir "$target/wasm-node"
}
step_policy_load() {
  rm -rf "$logdir/policy-load"
  tests/e2e/policy_load.sh "$bin" "$logdir/policy-load" "$target/wasm-node"
}
step_wasm_smoke() {
  if [[ -s "$smoke_art" ]]; then
    node crates/pokerbot-wasm/smoke/smoke.mjs "$target/wasm-node" "$smoke_art" | tee "$logdir/wasm-smoke.out"
    grep -q "cfr check/call digest seed 7 x20: $smoke_digest" "$logdir/wasm-smoke.out" || {
      echo "wasm CFR digest differs from the recorded native digest $smoke_digest"
      return 1
    }
  else
    node crates/pokerbot-wasm/smoke/smoke.mjs "$target/wasm-node"
    loud_skip "wasm-smoke ran without the smoke artifact (baseline only); the ci_tiny" \
      "artifact was exercised by policy-load"
  fi
}
step_browser() {
  command -v npm >/dev/null || { loud_skip "browser: npm not installed"; return; }
  have_chrome || { loud_skip "browser: Google Chrome not found (local-only step)"; return; }
  [[ -s "$smoke_art" ]] || { loud_skip "browser: needs artifacts/smoke_20bb.pokerbot"; return; }
  # The demo typechecks packages/pokerbot-browser's TypeScript sources, which
  # resolve react/@types/react from that package's own node_modules: install
  # both lockfiles (a fresh clone has neither).
  (cd packages/pokerbot-browser && npm ci)
  (cd examples/react-demo && npm ci && npm run build && npm run smoke)
}

steps=(fmt clippy test bins smoke-artifact eval-smoke wasm-build policy-load wasm-smoke browser)
if [[ ${#only[@]} -gt 0 ]]; then
  for s in "${only[@]}"; do
    [[ " ${steps[*]} " == *" $s "* ]] || { echo "ci_local: unknown step $s" >&2; exit 2; }
  done
  steps=("${only[@]}")
fi

echo "ci_local: repo $root, target $target, log $log"
names=() codes=() status=() secs=()
for s in "${steps[@]}"; do
  echo
  echo "===================== [$s] ====================="
  t0=$(date +%s)
  set +e
  ( set -euo pipefail; "step_${s//-/_}" )
  code=$?
  set -e
  dt=$(( $(date +%s) - t0 ))
  case $code in
    0) st=PASS ;;
    77) st=SKIP ;;
    *) st=FAIL ;;
  esac
  names+=("$s"); codes+=("$code"); status+=("$st"); secs+=("$dt")
  echo "===================== [$s] $st (exit $code, ${dt}s)"
done

echo
echo "ci_local summary ($(date '+%Y-%m-%d %H:%M:%S'))"
printf '%-16s %-6s %-5s %s\n' STEP STATUS EXIT SECONDS
failed=0
for i in "${!names[@]}"; do
  printf '%-16s %-6s %-5s %s\n' "${names[$i]}" "${status[$i]}" "${codes[$i]}" "${secs[$i]}"
  [[ "${status[$i]}" == FAIL ]] && failed=1
done
echo "(SKIP = exit 77, printed loudly above; log: $log)"
exit $failed
