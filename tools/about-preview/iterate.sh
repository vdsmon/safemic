#!/bin/bash
# Fast About-window iteration loop.
#
# Launches the sidecar, captures an in-process PNG via
# bitmapImageRepForCachingDisplayInRect (no Screen Recording grant), resizes to
# the target dimensions, then computes structural dissimilarity against the
# design target via `magick compare -metric SSIM`.
#
# Usage:
#   tools/about-preview/iterate.sh
#
# Env overrides:
#   ABOUT_DISSIM_THRESHOLD    default 0.04 (lower = stricter; 0 = identical)
#   SAFEMIC_PREVIEW_SETTLE_MS default 220
#
# Outputs:
#   /tmp/safemic-about-snap/about.png          raw window snapshot
#   /tmp/safemic-about-snap/about-resized.png  resized to target dims
#
# Metric semantics:
#   ImageMagick 7's `compare -metric SSIM` emits structural DISSIMILARITY
#   in the parenthesised normalized value, NOT similarity. Verified by direct
#   test: identical pair -> 0; black-vs-white -> ~0.5; smaller = closer. We
#   keep magick's value as `dissim`, also publish a derived `similarity` for
#   readability where 1.0 = identical.
#
# Exit codes: 0 ACCEPT (dissim <= threshold), 1 setup failure, 3 REJECT.
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
OUT="/tmp/safemic-about-snap"
TARGET="/Users/victordsm/.claude/loop-finder/60bc3b8f2621/target.png"
THRESHOLD="${ABOUT_DISSIM_THRESHOLD:-0.04}"
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

extract_dissim() {
  local raw="$1"
  local d
  d=$(printf '%s\n' "$raw" | sed -n 's/.*(\([0-9.eE+-]*\)).*/\1/p')
  if [ -z "$d" ]; then d="$raw"; fi
  echo "$d"
}

RAW=$(magick compare -metric SSIM "$TARGET" "$OUT/about-resized.png" NULL: 2>&1 | tr -d '\n')
DISSIM=$(extract_dissim "$RAW")
SIMILARITY=$(awk -v d="$DISSIM" 'BEGIN {printf "%.4f", 1.0 - 2.0 * d}')

# G2: per-quadrant dissim emission. Split target + resized render into 4 quadrants
# (top-left, top-right, bottom-left, bottom-right) so the next product-iteration
# cycle gets spatial signal on WHERE the design differs, not just an opaque scalar.
TW=$(magick identify -format "%w" "$TARGET")
TH=$(magick identify -format "%h" "$TARGET")
HW=$((TW / 2))
HH=$((TH / 2))

magick "$TARGET" -crop "${HW}x${HH}+0+0" +repage "$OUT/target-q1.png"
magick "$TARGET" -crop "${HW}x${HH}+${HW}+0" +repage "$OUT/target-q2.png"
magick "$TARGET" -crop "${HW}x${HH}+0+${HH}" +repage "$OUT/target-q3.png"
magick "$TARGET" -crop "${HW}x${HH}+${HW}+${HH}" +repage "$OUT/target-q4.png"

magick "$OUT/about-resized.png" -crop "${HW}x${HH}+0+0" +repage "$OUT/about-q1.png"
magick "$OUT/about-resized.png" -crop "${HW}x${HH}+${HW}+0" +repage "$OUT/about-q2.png"
magick "$OUT/about-resized.png" -crop "${HW}x${HH}+0+${HH}" +repage "$OUT/about-q3.png"
magick "$OUT/about-resized.png" -crop "${HW}x${HH}+${HW}+${HH}" +repage "$OUT/about-q4.png"

Q1=$(extract_dissim "$(magick compare -metric SSIM "$OUT/target-q1.png" "$OUT/about-q1.png" NULL: 2>&1 | tr -d '\n')")
Q2=$(extract_dissim "$(magick compare -metric SSIM "$OUT/target-q2.png" "$OUT/about-q2.png" NULL: 2>&1 | tr -d '\n')")
Q3=$(extract_dissim "$(magick compare -metric SSIM "$OUT/target-q3.png" "$OUT/about-q3.png" NULL: 2>&1 | tr -d '\n')")
Q4=$(extract_dissim "$(magick compare -metric SSIM "$OUT/target-q4.png" "$OUT/about-q4.png" NULL: 2>&1 | tr -d '\n')")

WORST_Q=$(awk -v q1="$Q1" -v q2="$Q2" -v q3="$Q3" -v q4="$Q4" 'BEGIN {
  n="q1"; v=q1+0;
  if (q2+0 > v) { n="q2"; v=q2+0 }
  if (q3+0 > v) { n="q3"; v=q3+0 }
  if (q4+0 > v) { n="q4"; v=q4+0 }
  printf "%s %s", n, v
}')
WORST_NAME=$(echo "$WORST_Q" | awk '{print $1}')
WORST_VAL=$(echo "$WORST_Q" | awk '{print $2}')

echo "dissim: $DISSIM similarity: $SIMILARITY threshold: $THRESHOLD path: $out (raw: $RAW)"
echo "  q1 (top-left):     $Q1"
echo "  q2 (top-right):    $Q2"
echo "  q3 (bottom-left):  $Q3"
echo "  q4 (bottom-right): $Q4"
echo "worst_quadrant: $WORST_NAME dissim=$WORST_VAL"

if awk -v d="$DISSIM" -v t="$THRESHOLD" 'BEGIN {exit !(d <= t)}'; then
  echo "ACCEPT"
  exit 0
else
  echo "REJECT (dissim $DISSIM > $THRESHOLD)"
  exit 3
fi
