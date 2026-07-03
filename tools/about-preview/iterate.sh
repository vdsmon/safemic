#!/bin/bash
# Fast About-window iteration loop.
#
# For each appearance (dark/light), launches the sidecar, captures an
# in-process PNG via bitmapImageRepForCachingDisplayInRect (no Screen
# Recording grant), resizes to the target dimensions, then computes
# structural dissimilarity against the accepted-design target via
# `magick compare -metric SSIM`.
#
# Since the 2026-07 native redesign this gate is SELF-REGRESSION, not
# mock-conformance: targets are captures of the accepted build, refreshed
# with --update after intentional design changes.
#
# Usage:
#   tools/about-preview/iterate.sh            # capture + compare both appearances
#   tools/about-preview/iterate.sh --update   # capture + overwrite targets
#
# Env overrides:
#   ABOUT_DISSIM_THRESHOLD    default 0.04 (lower = stricter; 0 = identical)
#   SAFEMIC_PREVIEW_SETTLE_MS default 220
#
# Outputs:
#   /tmp/safemic-about-snap/about-<appearance>.png          raw window snapshots
#   /tmp/safemic-about-snap/about-<appearance>-resized.png  resized to target dims
#
# Metric semantics:
#   ImageMagick 7's `compare -metric SSIM` emits structural DISSIMILARITY
#   in the parenthesised normalized value, NOT similarity. Verified by direct
#   test: identical pair -> 0; black-vs-white -> ~0.5; smaller = closer. We
#   keep magick's value as `dissim`, also publish a derived `similarity` for
#   readability where 1.0 = identical.
#
# Exit codes: 0 ACCEPT (all dissim <= threshold), 1 setup failure, 3 REJECT.
set -e

# Derive repo/worktree root from script location. Works in both main checkout
# and git worktrees (the old hardcoded ROOT raced parallel variants through
# main's binary). Falls back to `git rev-parse` when symlinks are in play.
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
if [ -d "$ROOT/.git" ] || [ -f "$ROOT/.git" ]; then
  GIT_ROOT="$(cd "$ROOT" && git rev-parse --show-toplevel 2>/dev/null || true)"
  if [ -n "$GIT_ROOT" ]; then ROOT="$GIT_ROOT"; fi
fi

BIN="$ROOT/target/aarch64-apple-darwin/debug/about-preview"
# Overridable so sandboxed agent runs can point at a writable tmp dir.
OUT="${SAFEMIC_SNAP_DIR:-/tmp/safemic-about-snap}"
TARGET_DIR="${ABOUT_TARGET_DIR:-$HOME/.claude/loop-finder/60bc3b8f2621}"
THRESHOLD="${ABOUT_DISSIM_THRESHOLD:-0.04}"
SETTLE_MS="${SAFEMIC_PREVIEW_SETTLE_MS:-220}"
APPEARANCES=(dark light)

UPDATE=0
for arg in "$@"; do
  case "$arg" in
    --update) UPDATE=1 ;;
    *) echo "unknown arg: $arg" >&2; exit 2 ;;
  esac
done

mkdir -p "$OUT" "$TARGET_DIR"
( cd "$ROOT" && cargo build -p about-preview --target aarch64-apple-darwin 2>&1 ) | tail -3

pkill -f "target/aarch64-apple-darwin/debug/about-preview" 2>/dev/null || true

extract_dissim() {
  local raw="$1"
  local d
  d=$(printf '%s\n' "$raw" | sed -n 's/.*(\([0-9.eE+-]*\)).*/\1/p')
  if [ -z "$d" ]; then d="$raw"; fi
  echo "$d"
}

fail=0
for appearance in "${APPEARANCES[@]}"; do
  out="$OUT/about-$appearance.png"
  if ! SAFEMIC_PREVIEW_SNAPSHOT="$out" \
       SAFEMIC_PREVIEW_APPEARANCE="$appearance" \
       SAFEMIC_PREVIEW_SETTLE_MS="$SETTLE_MS" "$BIN" >/dev/null 2>&1; then
    echo "FAIL: about-preview ($appearance) did not produce $out" >&2
    exit 1
  fi
  if [ ! -s "$out" ]; then
    echo "FAIL: $out empty or missing" >&2
    exit 1
  fi

  target="$TARGET_DIR/target-$appearance.png"
  if [ "$UPDATE" = 1 ] || [ ! -f "$target" ]; then
    cp "$out" "$target"
    echo "target written: $target"
    continue
  fi

  TARGET_W=$(magick identify -format "%w" "$target")
  TARGET_H=$(magick identify -format "%h" "$target")
  magick "$out" -resize "${TARGET_W}x${TARGET_H}!" "$OUT/about-$appearance-resized.png"

  RAW=$(magick compare -metric SSIM "$target" "$OUT/about-$appearance-resized.png" NULL: 2>&1 | tr -d '\n')
  DISSIM=$(extract_dissim "$RAW")
  SIMILARITY=$(awk -v d="$DISSIM" 'BEGIN {printf "%.4f", 1.0 - 2.0 * d}')
  echo "$appearance: dissim: $DISSIM similarity: $SIMILARITY threshold: $THRESHOLD path: $out (raw: $RAW)"

  if ! awk -v d="$DISSIM" -v t="$THRESHOLD" 'BEGIN {exit !(d <= t)}'; then
    echo "REJECT ($appearance dissim $DISSIM > $THRESHOLD)"
    fail=1
  fi
done

if [ "$fail" = 0 ]; then
  echo "ACCEPT"
  exit 0
else
  exit 3
fi
