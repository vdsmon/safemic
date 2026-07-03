#!/bin/bash
# Fast Popup-bezel iteration loop.
#
# For each state (muted/unmuted) × appearance (dark/light), launches the
# sidecar, captures an in-process PNG via
# bitmapImageRepForCachingDisplayInRect (no Screen Recording grant, immune to
# the release popup's content protection), and optionally diffs against a
# baseline. NOTE: the capture shows the vibrancy tint plate without live
# backdrop blur — the gate covers layout/glyph/tint/radius, not blur quality.
#
# Usage:
#   tools/popup-preview/iterate.sh            # capture all states
#   tools/popup-preview/iterate.sh --diff     # gate: exit 0 = match, 3 = drift
#   tools/popup-preview/iterate.sh --update   # rewrite snapshots/popup/ baselines
set -e

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
if [ -d "$ROOT/.git" ] || [ -f "$ROOT/.git" ]; then
  GIT_ROOT="$(cd "$ROOT" && git rev-parse --show-toplevel 2>/dev/null || true)"
  if [ -n "$GIT_ROOT" ]; then ROOT="$GIT_ROOT"; fi
fi
BIN="$ROOT/target/aarch64-apple-darwin/debug/popup-preview"
# Overridable so sandboxed agent runs can point at a writable tmp dir.
OUT="${SAFEMIC_SNAP_DIR:-/tmp/safemic-popup-snap}"
SNAPSHOT_BASE="$ROOT/snapshots/popup"

STATES=(muted unmuted)
APPEARANCES=(dark light)
SETTLE_MS="${SAFEMIC_PREVIEW_SETTLE_MS:-220}"

DIFF=0
UPDATE=0
for arg in "$@"; do
  case "$arg" in
    --diff)   DIFF=1 ;;
    --update) UPDATE=1; DIFF=1 ;;
    *) echo "unknown arg: $arg" >&2; exit 2 ;;
  esac
done

mkdir -p "$OUT"
rm -f "$OUT"/*.png
pkill -f "target/aarch64-apple-darwin/debug/popup-preview" 2>/dev/null || true

( cd "$ROOT" && cargo build -p popup-preview --target aarch64-apple-darwin 2>&1 ) | tail -3

CAPTURES=()
for state in "${STATES[@]}"; do
  for appearance in "${APPEARANCES[@]}"; do
    name="$state-$appearance"
    out="$OUT/$name.png"
    if ! SAFEMIC_PREVIEW_STATE="$state" \
         SAFEMIC_PREVIEW_APPEARANCE="$appearance" \
         SAFEMIC_PREVIEW_SNAPSHOT="$out" \
         SAFEMIC_PREVIEW_SETTLE_MS="$SETTLE_MS" \
         "$BIN" >/dev/null 2>&1; then
      echo "FAIL: state=$name did not produce $out" >&2
      exit 1
    fi
    if [ ! -s "$out" ]; then
      echo "FAIL: $out empty or missing" >&2
      exit 1
    fi
    CAPTURES+=("$name")
    printf "  %-16s -> %s\n" "$name" "$out"
  done
done

if [ "$DIFF" = 1 ]; then
  mkdir -p "$SNAPSHOT_BASE"
  fail=0
  for name in "${CAPTURES[@]}"; do
    actual="$OUT/$name.png"
    expected="$SNAPSHOT_BASE/$name.png"
    diff_out="$OUT/diff-$name.png"
    if [ "$UPDATE" = 1 ] || [ ! -f "$expected" ]; then
      cp "$actual" "$expected"
      echo "  baseline written: $expected"
      continue
    fi
    if odiff "$expected" "$actual" "$diff_out" --threshold 0.02 --antialiasing >/dev/null 2>&1; then
      rm -f "$diff_out"
      echo "  ok:    $name"
    else
      echo "  DIFF:  $name -> $diff_out"
      fail=1
    fi
  done
  [ "$fail" = 0 ] || exit 3
fi
