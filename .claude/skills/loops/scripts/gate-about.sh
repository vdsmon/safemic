#!/bin/bash
# Thin wrapper: invoke mic-mute about-window target-conformance gate from anywhere.
# Auto-derives repo root from this script's location.
#
# Usage:
#   .claude/skills/loops/scripts/gate-about.sh
#
# Env overrides:
#   ABOUT_DISSIM_THRESHOLD     default 0.04 (lower = stricter; 0 = identical)
#   SAFEMIC_PREVIEW_SETTLE_MS  default 220
#
# Class id: 60bc3b8f2621
# Target image: ~/.claude/loop-finder/60bc3b8f2621/target.png
# IMPORTANT: oracle emits DISSIMILARITY (lower=closer). See references/magick-ssim-quick-reference.md.

set -e

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
cd "$REPO"
exec tools/about-preview/iterate.sh "$@"
