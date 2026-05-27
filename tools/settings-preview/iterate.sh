#!/bin/bash
# Fast Settings-window iteration loop.
#
# For each visual state, launches the sidecar, captures an in-process PNG via
# bitmapImageRepForCachingDisplayInRect (no Screen Recording grant needed),
# montages all states into a 2x2 grid, and optionally diffs against a baseline.
#
# Usage:
#   tools/settings-preview/iterate.sh                # capture all default states
#   tools/settings-preview/iterate.sh recording      # capture one state only
#   tools/settings-preview/iterate.sh --diff         # capture all + diff vs snapshots/
#   tools/settings-preview/iterate.sh --update       # capture all + overwrite snapshots/
#
# Outputs:
#   /tmp/safemic-snap/<state>.png    per-state captures
#   /tmp/safemic-snap/grid.png       2x2 montage of all states
#   /tmp/safemic-snap/diff-<state>.png  pixel diff (only when --diff and diff found)
#
# Requirements:
#   magick (brew install imagemagick)
#   odiff  (brew install odiff)   — only needed for --diff / --update
set -e

# Derive repo/worktree root from script location. Works in both main checkout
# and git worktrees (the old hardcoded ROOT raced parallel variants through
# main's binary).
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
if [ -d "$ROOT/.git" ] || [ -f "$ROOT/.git" ]; then
  GIT_ROOT="$(cd "$ROOT" && git rev-parse --show-toplevel 2>/dev/null || true)"
  if [ -n "$GIT_ROOT" ]; then ROOT="$GIT_ROOT"; fi
fi
BIN="$ROOT/target/aarch64-apple-darwin/debug/settings-preview"
OUT="/tmp/safemic-snap"
SNAPSHOT_BASE="$ROOT/snapshots/settings"

DEFAULT_STATES=(default recording warning status_ok)
SETTLE_MS="${SAFEMIC_PREVIEW_SETTLE_MS:-260}"

DIFF=0
UPDATE=0
SINGLE_STATE=""
for arg in "$@"; do
  case "$arg" in
    --diff)   DIFF=1 ;;
    --update) UPDATE=1; DIFF=1 ;;
    --*)      echo "unknown flag: $arg" >&2; exit 2 ;;
    *)        SINGLE_STATE="$arg" ;;
  esac
done

if [ -n "$SINGLE_STATE" ]; then
  STATES=("$SINGLE_STATE")
else
  STATES=("${DEFAULT_STATES[@]}")
fi

mkdir -p "$OUT"
rm -f "$OUT"/*.png

# Kill any prior sidecar instance (idempotent).
pkill -f "target/aarch64-apple-darwin/debug/settings-preview" 2>/dev/null || true

# Incremental build.
( cd "$ROOT" && cargo build -p settings-preview --target aarch64-apple-darwin 2>&1 ) | tail -3

# Capture each state in its own sidecar process. Snapshot mode pumps the
# NSRunLoop briefly, writes the PNG, and exits — no event-loop dance.
for state in "${STATES[@]}"; do
  out="$OUT/$state.png"
  if ! SAFEMIC_PREVIEW_STATE="$state" \
       SAFEMIC_PREVIEW_SNAPSHOT="$out" \
       SAFEMIC_PREVIEW_SETTLE_MS="$SETTLE_MS" \
       "$BIN" >/dev/null 2>&1; then
    echo "FAIL: state=$state did not produce $out" >&2
    exit 1
  fi
  if [ ! -s "$out" ]; then
    echo "FAIL: $out empty or missing" >&2
    exit 1
  fi
  printf "  %-12s -> %s\n" "$state" "$out"
done

# Compose 2x2 montage when capturing the full set. Opt-in via SAFEMIC_PREVIEW_GRID=1.
if [ "${SAFEMIC_PREVIEW_GRID:-0}" = "1" ] && [ -z "$SINGLE_STATE" ]; then
  # 2x2 grid via append (no font needed — bypasses magick montage's label
  # text rendering, which fails when no fonts are configured).
  magick \
    \( "$OUT/default.png" "$OUT/recording.png" +append \) \
    \( "$OUT/warning.png" "$OUT/status_ok.png" +append \) \
    -background "#1a1a1a" -append \
    -bordercolor "#1a1a1a" -border 12 \
    "$OUT/grid.png"
  echo "grid: $OUT/grid.png"
fi

# Diff or update against baselines.
if [ "$DIFF" = 1 ]; then
  mkdir -p "$SNAPSHOT_BASE"
  fail=0
  for state in "${STATES[@]}"; do
    actual="$OUT/$state.png"
    expected="$SNAPSHOT_BASE/$state.png"
    diff_out="$OUT/diff-$state.png"
    if [ "$UPDATE" = 1 ] || [ ! -f "$expected" ]; then
      cp "$actual" "$expected"
      echo "  baseline written: $expected"
      continue
    fi
    if odiff "$expected" "$actual" "$diff_out" --threshold 0.02 --antialiasing >/dev/null 2>&1; then
      rm -f "$diff_out"
      echo "  ok:    $state"
    else
      echo "  DIFF:  $state -> $diff_out"
      fail=1
    fi
  done
  [ "$fail" = 0 ] || exit 3
fi
