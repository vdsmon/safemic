---
name: loops
description: >-
  Run the mic-mute repo's two pre-built visual feedback gates — settings-window
  snapshot regression (`tools/settings-preview/iterate.sh`) and about-window
  target-conformance (`tools/about-preview/iterate.sh`). Use this skill whenever
  the user asks to "test the settings window", "check about-window vs target",
  "run the visual gate", "verify the UI hasn't drifted", "snapshot regression
  test", "visual diff this", "did I break the layout", "does the about match
  the design", "iterate the UI toward the target image", or makes any change to
  `src/settings_window.rs`, `src/about.rs`, `src/popup_content.rs`,
  `src/shortcut_recorder.rs` that should be verified before commit. Both gates
  are self-verifiable (exit 0 = accept, non-zero = reject); never ask a human
  to judge mid-iteration. The skill bakes in the `magick compare -metric SSIM`
  direction caveat (emits DISSIMILARITY not similarity) that cost hours during
  the loop-finder dogfood. Strongly prefer this skill over re-discovering the
  gates by reading tools/ directories.
when_to_use: >-
  Mic-mute repo only (project-scoped skill). UI verification tasks. Do NOT use
  for unit tests, Rust lint, build verification (those don't have registered
  gates yet — invoke `/loop-finder` if a new class is needed).
argument-hint: "[settings | about | both]"
allowed-tools:
  - Read
  - Bash
---

# loops

Two visual gates live in this repo. This skill documents them, the caveats baked in, and how to run each. Both were produced by `loop-finder` during a 4-cycle dogfood on 2026-05-27 and are battle-tested.

## TL;DR — invoking the gates

| Goal | Command | Exit codes |
|---|---|---|
| Settings window matches snapshots | `tools/settings-preview/iterate.sh --diff` | 0 = match, 3 = drift |
| Update settings baselines | `tools/settings-preview/iterate.sh --update` | always 0; commit afterwards |
| About window matches design target | `tools/about-preview/iterate.sh` | 0 = ACCEPT, 3 = REJECT (dissim too high) |

Run from repo root. Both scripts derive their working root from `${BASH_SOURCE[0]}` so worktrees are isolated.

Thin wrappers also live at `.claude/skills/loops/scripts/gate-settings.sh` and `.claude/skills/loops/scripts/gate-about.sh` for invocation from outside the repo root (they cd to git-toplevel first).

## Gate 1 — settings-window snapshot regression

- **Class id** (loop-finder cache): `4aa9e37f9396`
- **Sidecar**: `tools/settings-preview/` (Rust workspace member)
- **Oracle**: [odiff](https://github.com/dmtrKovalenko/odiff) pixel diff vs `snapshots/settings/{default,recording,warning,status_ok}.png`
- **States captured**: 4 (default UI, recording-shortcut, shortcut-conflict warning, status-OK pulse)
- **Adopted gate config** (loop-finder cycle 1, V2): magick `+append` grid composition is opt-in via `SAFEMIC_PREVIEW_GRID=1`. Default off saves ~566ms per run (24% wall-clock improvement).
- **Median wall**: ~2.3s
- **Verdict tags in output**: `ok:` per state on match, `DIFF:` per state on drift (with path to per-state diff PNG)

When a state drifts, the gate writes `/tmp/safemic-snap/diff-<state>.png`. Read that file to see the pixel delta. If the drift is intentional, run `--update` then commit the new `snapshots/settings/*.png`.

## Gate 2 — about-window target-conformance

- **Class id**: `60bc3b8f2621`
- **Sidecar**: `tools/about-preview/` (Rust workspace member)
- **Oracle**: `magick compare -metric SSIM` (NOTE: emits DISSIMILARITY in the parens; see caveats below) vs `~/.claude/loop-finder/60bc3b8f2621/target.png`
- **Predicate**: `dissim ≤ ABOUT_DISSIM_THRESHOLD` for ACCEPT (default 0.04; lower = closer to target; 0 = identical)
- **Adopted gate config** (loop-finder cycle 2, G2): per-quadrant dissim emission. Output includes overall `dissim:`, per-quadrant `q1..q4`, and `worst_quadrant:` indicator. Spatial signal so iterating agents can target the region that's furthest off.
- **Current baseline**: dissim 0.055 (~sim 0.89). Threshold 0.04 not yet ACCEPT. Bottom half (q3 + q4) is the residual hotspot.
- **Median wall**: ~7s (per-quadrant adds ~3.5s of magick crops + compares vs scalar SSIM)

To raise/lower strictness, set `ABOUT_DISSIM_THRESHOLD` env var before invoking.

## Critical caveats (read these — they bit us)

### 1. `magick compare -metric SSIM` emits DISSIMILARITY

Despite the metric name, ImageMagick 7's output in the parenthesised value behaves like DSSIM (structural dissimilarity), NOT SSIM (similarity). Verify any time:

```bash
magick compare -metric SSIM image.png image.png NULL: 2>&1
# → "0 (0)" — identical pair gives 0

magick -size 10x10 xc:black /tmp/b.png; magick -size 10x10 xc:white /tmp/w.png
magick compare -metric SSIM /tmp/b.png /tmp/w.png NULL: 2>&1
# → "... (0.49995)" — black vs white gives ~0.5
```

Predicate direction must be `dissim <= threshold` for ACCEPT (lower-better). Never `>= threshold`. The initial about-window iterate.sh had it inverted and the gate was meaningless for an entire cycle before V1's variant agent caught it.

### 2. magick SSIM is non-monotone under extreme corruption

Brightening the render to 200%, or inverting colors, can produce a LOWER dissim than a clean-but-wrong render. Pure white image gives dissim ~0.6 (less than some real renders). Implication:

- Canary fixtures (for adoption-time regression guards) MUST test mid-range corruption — 1-3 element edits, color shifts <30%, single layout-constant mutations. NOT extremes.
- Both classes currently have empty `known-bad/`. Seed before relying on canary protection.

### 3. iterate.sh ROOT derives from script location

Both scripts use:
```bash
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
```

Worktree-safe. Do NOT hardcode `/Users/.../mic-mute`. The hardcoded version race-fights parallel worktree variants through main's binary — bit the loop-finder dogfood twice before fix.

### 4. Sidecar version source is wrong (known bug)

`tools/about-preview/Cargo.toml` declares its own `package.version = "0.0.0"`. The sidecar binary's `env!("CARGO_PKG_VERSION")` reads 0.0.0. `src/about.rs` has the version `"v0.5.1"` hardcoded as a workaround. Proper fix: build-script that reads parent (`safemic`) crate's version. Not yet implemented.

### 5. AppKit text-alignment numbering quirk

On this build, `setAlignment: 1` = Center (UIKit numbering), not 2 (NSTextAlignment enum). Pinned empirically in `src/about.rs`. If centered text rendering left-aligned after an AppKit version bump, this is the suspect.

## Per-class cache layout

Each gate maintains state at `~/.claude/loop-finder/<class-id>/`:

```
baseline.json           current metric tuple (dissim, wall_s, blindness, tokens, flake_rate)
baseline-history.jsonl  append-only audit log of baseline changes
config.yaml             current gate composition + lex_order
summary.md              written on loop-finder halt
known-bad/              canary regression fixtures (frozen)
variants/               per-cycle exploration artifacts
target.png              (about-window only) design target image
```

To inspect history of a class:
```bash
cat ~/.claude/loop-finder/4aa9e37f9396/baseline-history.jsonl   # settings
cat ~/.claude/loop-finder/60bc3b8f2621/baseline-history.jsonl   # about
```

## When to extend (add a 3rd, 4th class)

Both classes here were generated by the `loop-finder` skill. To add another (e.g., popup HUD class, mic.rs unit-test class, full-app SIGUSR1-driven smoke test):

1. Invoke `/loop-finder <task description>`.
2. Skill classifies on two axes: domain (code | UI | audio | etc.) + vision-required (yes/no/maybe).
3. Skill walks `menu.yaml` for fitting oracle patterns.
4. Surfaces tooling gaps as HITL (install / register MCP / commit WIP).
5. Builds the gate (uses `templates/iterate.sh.tmpl` as starting point).
6. Caches per class.

After loop-finder completes a new class, update THIS skill's "Gate 1 / Gate 2" section to include the new gate's command + class-id + caveats.

The loop-finder plugin lives at `~/repos/personal/claude-skills/plugins/loop-finder/`. Its `RETRO-CYCLE-4.md` and `PRIOR_ART.md` document the full design rationale.

## Reference files

- `references/dogfood-history.md` — 4-cycle history of how these two gates landed, what got rejected, why thresholds are where they are.
- `references/magick-ssim-quick-reference.md` — single-page quick-reference on direction, non-monotone corruption, threshold calibration.

## Not in scope

- Unit tests for `src/mic.rs` (CoreAudio mute state machine) — no gate registered. Invoke `/loop-finder` to add.
- Popup HUD rendering — no gate. Pattern would mirror settings-preview (4 states: muted/unmuted/transitioning/cursor-edge).
- Audio output check — no gate. mic-mute doesn't emit audio.
