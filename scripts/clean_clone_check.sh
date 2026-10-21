#!/usr/bin/env bash
# Clean-clone check for Pokerbot (M-24).
#
#   scripts/clean_clone_check.sh [--include-working-tree] [ci_local.sh args...]
#
# Exports `git archive HEAD` into a fresh temporary directory (no .git, no
# target/, no node_modules, no gitignored files), gives it its OWN
# CARGO_TARGET_DIR inside that directory, and runs scripts/ci_local.sh there.
# Nothing in the repository is modified; the temporary directory and its
# target are deleted afterwards (also on failure or Ctrl-C).
#
#   --include-working-tree  also copy uncommitted changes (modified tracked
#                           files and untracked, non-ignored files) on top of
#                           HEAD, to check work that is not committed yet.
#                           Only read-only git commands are used. Files
#                           deleted in the working tree stay as in HEAD.
#
# The gitignored smoke policy artifacts/smoke_20bb.pokerbot (+ manifest) is
# copied in from this repository if present -- this is announced loudly,
# because a true clean clone would not have it (ci_local.sh then skips the
# steps that need it, or trains it with --with-smoke-training).
#
# Aborts if less than 3 GiB is free on the temp filesystem (the run needs
# about 1.5 GiB for a debug workspace build, wasm release build and npm).
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

overlay=0
if [[ "${1:-}" == "--include-working-tree" ]]; then
  overlay=1
  shift
fi

tmp_base="${TMPDIR:-/tmp}"
tmp_base="${tmp_base%/}"
free_kb=$(df -Pk "$tmp_base" | awk 'NR==2 {print $4}')
need_kb=$((3 * 1024 * 1024))
if (( free_kb < need_kb )); then
  echo "clean_clone_check: only $((free_kb / 1024)) MiB free on $tmp_base; need >= 3072 MiB. Aborting." >&2
  exit 3
fi
echo "clean_clone_check: $((free_kb / 1024)) MiB free on $tmp_base"

work="$(mktemp -d "$tmp_base/pokerbot-clean-clone.XXXXXX")"
cleanup() {
  echo "clean_clone_check: deleting $work (sources + target)"
  rm -rf "$work"
}
trap cleanup EXIT INT TERM

head_rev="$(git rev-parse --short HEAD)"
echo "clean_clone_check: exporting HEAD $head_rev into $work/src"
mkdir -p "$work/src"
git archive --format=tar HEAD | tar -x -C "$work/src"

if [[ $overlay == 1 ]]; then
  echo "clean_clone_check: overlaying uncommitted working-tree changes:"
  while IFS= read -r f; do
    [[ -f "$f" ]] || continue
    echo "  + $f"
    mkdir -p "$work/src/$(dirname "$f")"
    cp -p "$f" "$work/src/$f"
  done < <(git ls-files --modified --others --exclude-standard)
fi

if [[ -s artifacts/smoke_20bb.pokerbot ]]; then
  echo "!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!"
  echo "!! NOTE: copying the gitignored artifacts/smoke_20bb.pokerbot (+ manifest)"
  echo "!! from this repository into the clean clone. It is NOT in git."
  echo "!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!"
  mkdir -p "$work/src/artifacts"
  cp -p artifacts/smoke_20bb.pokerbot "$work/src/artifacts/"
  [[ -f artifacts/smoke_20bb.manifest.json ]] && cp -p artifacts/smoke_20bb.manifest.json "$work/src/artifacts/"
else
  echo "clean_clone_check: no artifacts/smoke_20bb.pokerbot to copy; ci_local.sh will skip loudly"
fi

export CARGO_TARGET_DIR="$work/target"
echo "clean_clone_check: CARGO_TARGET_DIR=$CARGO_TARGET_DIR"
set +e
(cd "$work/src" && scripts/ci_local.sh "$@")
code=$?
set -e
logs="$root/target/ci-local/clean-clone-$(date +%Y%m%d-%H%M%S)"
mkdir -p "$logs"
cp -p "$work/target/ci-local/"*.log "$logs/" 2>/dev/null || true
echo "clean_clone_check: ci_local.sh exited $code (HEAD $head_rev, overlay=$overlay); log copied to $logs"
exit $code
