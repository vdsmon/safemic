# `magick compare -metric SSIM` — quick reference

## TL;DR

ImageMagick 7's `compare -metric SSIM` emits **structural DISSIMILARITY**, NOT similarity, in the parenthesised normalized value. The metric name is misleading.

- `magick compare -metric SSIM A A NULL:` → `0 (0)` (identical = 0)
- `magick compare -metric SSIM black.png white.png NULL:` → `... (0.49995)` (max different ≈ 0.5)
- **Lower output = MORE similar to reference**
- Predicate for ACCEPT: `value <= threshold`

## Why this trips agents

Both the metric name ("SSIM" typically = similarity, 1 = identical in the academic literature) and the documentation imply higher = better. ImageMagick's implementation outputs the opposite. Always verify direction with the two empirical tests above before pinning a threshold.

## Threshold ranges (for ~720×500 UI windows resized to 1538×1023)

Empirical from the about-window class:

| Dissim | Interpretation |
|---|---|
| 0.00 | Identical |
| 0.01–0.03 | Near-pixel-perfect; below this only achievable for self-vs-self under deterministic rendering |
| 0.04 | Strict design-conformance target (~0.92 similarity equivalent) — may be unreachable if font rasterization differs |
| 0.05–0.07 | "Close to design" — captures correct layout, typography weight roughly matches, missing icon details acceptable |
| 0.08–0.12 | Layout broadly correct but missing key elements (no body copy, wrong button size, no icon) |
| 0.15+ | Substantial composition mismatch |
| 0.30+ | Completely different design |
| 0.50 | Max difference (e.g. black vs white frame) |

Above 0.30, the metric becomes non-monotone — see "non-monotone under extreme corruption" below.

## Non-monotone under extreme corruption

`magick compare -metric SSIM` is NOT order-preserving under extreme corruption when source and target compositions differ substantially. Examples seen during the dogfood:

- Clean render of correct design vs target: dissim 0.063
- Same render brightened 200%: dissim 0.802 (huge difference — expected)
- Same render with channels inverted: dissim 0.503 (also large)
- **Same render replaced with PURE WHITE**: dissim 0.606
- **Same render replaced with PURE BLACK**: dissim 0.500

The pure-white and pure-black versions are obviously "more wrong" than a brightened-but-correct render. But they read at LOWER dissim than the brightened version in some cases. Implication:

**Canary fixtures for visual gates MUST test mid-range corruption** (1-3 element edits, color shifts <30%, single layout-constant mutations). Pure-extreme corruption may give false-pass.

## Quick recipes

### Verify direction (do this once when setting up a new gate)

```bash
TARGET=/path/to/target.png
RENDER=/path/to/render.png

# Identity test
magick compare -metric SSIM "$TARGET" "$TARGET" NULL: 2>&1
# Expect: "0 (0)"

# Max-different test
magick -size 10x10 xc:black /tmp/b.png
magick -size 10x10 xc:white /tmp/w.png
magick compare -metric SSIM /tmp/b.png /tmp/w.png NULL: 2>&1
# Expect: "... (0.49995)"

# Real measurement
magick compare -metric SSIM "$TARGET" "$RENDER" NULL: 2>&1
# Output: "<raw> (<dissim>)" — use the parenthesised value
```

### Extract just the dissim score from the output

```bash
RAW=$(magick compare -metric SSIM "$TARGET" "$RENDER" NULL: 2>&1 | tr -d '\n')
DISSIM=$(printf '%s\n' "$RAW" | sed -n 's/.*(\([0-9.eE+-]*\)).*/\1/p')
echo "dissim: $DISSIM"
```

### Convert to similarity for human-readable display

```bash
SIMILARITY=$(awk -v d="$DISSIM" 'BEGIN {printf "%.4f", 1.0 - 2.0 * d}')
echo "similarity: $SIMILARITY"
# 1 - 2*d puts identical at 1.0 and max-different at 0.0 (approximate).
```

### Per-quadrant breakdown (the G2 adoption from cycle 2)

Splitting target and render into 4 quadrants and computing dissim per region surfaces WHERE the design differs — eliminates the "agent must Read the PNG" blindness:

```bash
TW=$(magick identify -format "%w" "$TARGET")
TH=$(magick identify -format "%h" "$TARGET")
HW=$((TW / 2))
HH=$((TH / 2))

for region in q1:0,0 q2:$HW,0 q3:0,$HH q4:$HW,$HH; do
  name=$(echo "$region" | cut -d: -f1)
  coords=$(echo "$region" | cut -d: -f2 | tr ',' ' ')
  x=$(echo "$coords" | awk '{print $1}')
  y=$(echo "$coords" | awk '{print $2}')

  magick "$TARGET" -crop "${HW}x${HH}+${x}+${y}" +repage "/tmp/t-$name.png"
  magick "$RENDER" -crop "${HW}x${HH}+${x}+${y}" +repage "/tmp/r-$name.png"

  D=$(magick compare -metric SSIM "/tmp/t-$name.png" "/tmp/r-$name.png" NULL: 2>&1 \
    | sed -n 's/.*(\([0-9.eE+-]*\)).*/\1/p')
  echo "  $name: $D"
done
```

The about-window gate (`tools/about-preview/iterate.sh`) does this and emits a `worst_quadrant:` indicator so iterating agents know which region to fix.

## Alternatives if SSIM isn't right for your task

| Metric | Use when | Caveats |
|---|---|---|
| LPIPS (Learned Perceptual Image Patch Similarity) | Perceptual judgment matters more than pixel-fidelity | Requires Python + torch + lpips. AlexNet backbone pretrained on ImageNet — weak priors for UI imagery. Try vgg/squeeze. |
| DINO patch similarity | Per-region semantic comparison | Heavy; needs ViT model loaded. |
| LLM-vision-judge with frozen rubric | When perceptual rubric is expressible in natural language | Gameable. Rubric + judge model must be version-pinned. |
| pixelmatch (Node) / Python pixelmatch | Fast pixel-exact diff with anti-aliasing tolerance | Direction is sane (output = pixels different, lower = closer). |
| odiff (already installed) | Same as pixelmatch but Rust binary, drop-in CLI | Direction is sane. Used by settings-preview gate. |

The about-window gate uses magick SSIM because magick is universally available (already installed for other reasons) and the per-quadrant breakdown is trivial in bash. LPIPS would give more perceptually-aligned judgments but adds a Python + torch install gate.
