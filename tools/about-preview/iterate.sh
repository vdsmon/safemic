#!/bin/bash
# Fast About-window iteration loop.
#
# Launches the sidecar, captures an in-process PNG via
# bitmapImageRepForCachingDisplayInRect (no Screen Recording grant), resizes to
# the target dimensions, then computes SSIM against the design target via
# `magick compare`.
#
# Usage:
#   tools/about-preview/iterate.sh
#
# Env overrides:
#   ABOUT_SSIM_THRESHOLD     default 0.92
#   SAFEMIC_PREVIEW_SETTLE_MS default 220
#
# Outputs:
#   /tmp/safemic-about-snap/about.png          raw window snapshot
#   /tmp/safemic-about-snap/about-resized.png  resized to target dims
#
# Exit codes: 0 ACCEPT, 1 setup/runtime failure, 3 REJECT (ssim < threshold).
set -e

ROOT="/Users/victordsm/repos/mic-mute"
BIN="$ROOT/target/aarch64-apple-darwin/debug/about-preview"
OUT="/tmp/safemic-about-snap"
TARGET="/Users/victordsm/.claude/loop-finder/60bc3b8f2621/target.png"
THRESHOLD="${ABOUT_SSIM_THRESHOLD:-0.92}"
SETTLE_MS="${SAFEMIC_PREVIEW_SETTLE_MS:-220}"

mkdir -p "$OUT"
( cd "$ROOT" && cargo build -p about-preview --target aarch64-apple-darwin 2>&1 ) | tail -3

pkill -f "target/aarch64-apple-darwin/debug/about-preview" 2>/dev/null || true

out="$OUT/about.png"
if ! SAFEMIC_PREVIEW_SNAPSHOT="$out" SAFEMIC_PREVIEW_SETTLE_MS="$SETTLE_MS" "$BIN" >/dev/null 2>&1; then
  echo "FAIL: about-preview did not produce $out" >&2
  exit 1
fi
if [ ! -s "$out" ]; then
  echo "FAIL: $out empty or missing" >&2
  exit 1
fi

TARGET_W=$(magick identify -format "%w" "$TARGET")
TARGET_H=$(magick identify -format "%h" "$TARGET")
magick "$out" -resize "${TARGET_W}x${TARGET_H}!" "$OUT/about-resized.png"

# SSIM via magick compare. The score is emitted on stderr; redirect to stdout
# to capture. NULL: target discards the diff image (we only want the metric).
# magick 7 formats SSIM output as `<rawSum> (<normalized 0..1>)`. The value
# inside the parens is the actual similarity in [0,1] — that's what we compare.
RAW=$(magick compare -metric SSIM "$TARGET" "$OUT/about-resized.png" NULL: 2>&1 | tr -d '\n')
SSIM=$(printf '%s\n' "$RAW" | sed -n 's/.*(\([0-9.eE+-]*\)).*/\1/p')
if [ -z "$SSIM" ]; then
  # Older magick: emits a bare normalized float.
  SSIM="$RAW"
fi
echo "ssim: $SSIM threshold: $THRESHOLD path: $out (raw: $RAW)"

if awk -v s="$SSIM" -v t="$THRESHOLD" 'BEGIN {exit !(s >= t)}'; then
  echo "ACCEPT"
  exit 0
else
  echo "REJECT (ssim $SSIM < $THRESHOLD)"
  exit 3
fi
