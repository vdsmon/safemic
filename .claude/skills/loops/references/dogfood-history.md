# Dogfood history — how the two gates landed

Run date: 2026-05-27. First substantive deployment of `loop-finder` skill on mic-mute.

## Settings-window class (4aa9e37f9396)

### Cycle 0 — initial baseline

- Gate: `tools/settings-preview/iterate.sh --diff`
- Composition: `headless_probe_sidecar` + `visual_diff_oracle` + `snapshot_golden_oracle`
- Baselines already existed under `snapshots/settings/`
- Wall: 3.10s median
- flake_rate: 0
- blindness: 0

### Cycle 1 — variant race

| Variant | Goal | Result | Adopted |
|---|---|---|---|
| V1 | Lower SETTLE_MS 260→180 | wall 1.97s but flake_rate 0.10 (animation timing edge) | REJECT |
| V2 | magick montage opt-in | wall 1.84s, flake=0, smoke clean | **ADOPT** |
| V3 | daemon batch mode (4 spawns → 1 stdin-driven) | sidecar wiring complex; needs more cycles | future |

V2 adopted. Final baseline: wall 2.34s. blindness 0.

**Lesson recorded**: tools/ was untracked; worktrees couldn't isolate. Fixed by committing tools/.

## About-window class (60bc3b8f2621)

Started 2026-05-27 same session. Target image: 1538×1023 PNG of intended design (centered vertical layout, salmon pill button, octocat icon, traffic-light close).

### Cycle 0 — Step-0 prereq

Built `tools/about-preview/` (mirror of settings-preview). Split `src/about.rs::run_about_modal` into `build_about_window() + present_about_modal()` so sidecar can render headlessly without `runModalForWindow:` blocking.

Initial render (original 380×220 horizontal about.rs) vs target → dissim 0.0858.

### Cycle 1 — product variants

| Variant | Approach | Dissim | Notes |
|---|---|---|---|
| V1 | Faithful full redesign (720×500 vertical, big icon, body copy, pill, traffic-light close) | 0.0623 | Strong improvement. **ADOPT.** Also caught the inverted-predicate bug. |
| V2 | Window resize + reflow only | 0.0643 | Worse than baseline under inverted predicate (early measurement confusion). |
| V3 | Typography + content at 380×220 | 0.1197 | Window aspect dominates; reflow needed first. |

**Critical finding mid-cycle**: V1's agent noted `magick compare -metric SSIM` emits DISSIMILARITY (verified self-vs-self=0, black-vs-white=0.5). Initial gate had `s >= 0.92` predicate — backwards. Flipped to `dissim <= 0.04`. Re-evaluation under corrected semantics: V1 0.0623 was actually best.

### Cycle 2 — gate variants

| Variant | Change | Result | Adopted |
|---|---|---|---|
| G1 | Resize-direction flip (target down to render dims) | smoke canary failed (magick SSIM non-monotone under brightness corruption — corrupted reads as MORE similar than clean) | REJECT |
| G2 | Per-quadrant dissim emission (4-region SSIM + worst_quadrant indicator) | flake=0, smoke localizes correctly, +150% wall but huge blindness reduction (1→0) | **ADOPT** |
| G3 | LPIPS perceptual metric | works, deterministic, but AlexNet has weak priors for UI (judges V1 at 0.48 vs 0.15 threshold) | defer |
| G4 | Floor probe (calibration only) | floor=0 (snapshot pipeline bit-deterministic), threshold 0.04 physically reachable | informational |

G2 adopted. New baseline wall: 5.86s. Bottom half (q3/q4 ~0.078) emerged as the hotspot.

### Cycle 3 — product variants targeting q3/q4

| Variant | Approach | Dissim | q4 | flake | Adopted |
|---|---|---|---|---|---|
| V4 | Add octocat icon to pill | 0.0624 | 0.0784 | 0 | metric-noise (icon <0.5% of frame, scalar dissim dilutes) |
| V5 | Bottom-half spacing tune (divider/body/button vertical positions) | 0.0556 | 0.0647 | 0 | **ADOPT** |
| V6 | Pill button geometry exact-match (165×41, radius 20.5, font 16pt) | 0.0557 | 0.0650 | 0.20 | hard-gate fail (/tmp race with V5) |

V5 adopted. V6 metric tied but hit gate flake from `/tmp/safemic-about-snap/` shared across concurrent runs. Surfaced the "OUT dir must be per-PID" finding.

### Cycle 4 — combined product variant

| Variant | Approach | Dissim | flake | Adopted |
|---|---|---|---|---|
| V7 | V5 + V6 pill geometry + V4 octocat icon | 0.0550 | 0 | adopted on oracle output (-1% delta within margin but octocat is a real visual win) |

Halted on diminishing returns. Final dissim 0.055 (sim ~0.89). Threshold 0.04 not crossed — residual likely font rasterization differences between target renderer and live AppKit.

## Net progression — about-window

```
Cycle 0 init   dissim 0.0858 (sim ~0.83)
Cycle 1 V1     dissim 0.0623 (sim ~0.88)  — 27% reduction
Cycle 2 G2     gate adopted, blindness 1→0
Cycle 3 V5     dissim 0.0556 (sim ~0.89)  — 11% more
Cycle 4 V7     dissim 0.0550 (sim ~0.89)  — 1% more, octocat icon shipped
```

Total: 36% dissim reduction across 4 cycles.

## Why threshold 0.04 isn't reachable here

G4 confirmed render jitter floor = 0 (snapshot pipeline bit-deterministic). So 0.04 is physically reachable in principle. But the residual 0.015 gap is composed of:

1. Font rasterization differences (AppKit vs target's renderer — different anti-aliasing, hinting, sub-pixel positioning)
2. NSVisualEffectView blur material vs target's flat dark layer
3. Scalar SSIM under-rewarding small high-fidelity additions (V4 octocat was design-correct but registered as metric noise)

To cross 0.04, future work would need: G5 finer-patch metric (16×16 instead of 2×2 quadrants) or LPIPS hybrid (per-quadrant LPIPS) or LLM-vision-judge.

## 8 skill-meta findings (shipped to loop-finder plugin)

See `~/repos/personal/claude-skills/plugins/loop-finder/RETRO-CYCLE-4.md` for full text. Headlines:

1. caveman:cavecrew-builder lacks Bash → use general-purpose for measurement-required variants.
2. iterate.sh must derive ROOT from `${BASH_SOURCE[0]}`.
3. Worktrees miss uncommitted main state → commit WIP first OR run `helpers/bootstrap-worktree.sh`.
4. magick SSIM emits DISSIMILARITY not similarity.
5. magick SSIM non-monotone under extreme corruption.
6. Per-PID OUT dir to avoid concurrent-variant `/tmp` races.
7. Variant-type split: gate variants ranked by lex perf dims; product variants ranked by oracle output.
8. Vision-required is an orthogonal classification axis. Never assign codex to vision-required tasks (NSScreen.mainScreen returns None in codex sandbox → tao panics).
