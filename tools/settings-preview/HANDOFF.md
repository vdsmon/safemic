# Settings UI handoff

Cold-start brief for the next agent iterating on `src/settings_window.rs`.

## Current design (2026-07 native redesign)

Ventura-style grouped form following the system appearance (light + dark, semantic NSColors only — the old DarkAqua lock is gone). One rounded NSBox card, three hairline-separated rows, label left / control right:

1. Mute shortcut — NSBox chip showing `⇧⌘A`; whole chip is click-to-record; border flips to accent (recording) / systemRed (conflict)
2. Launch at login — NSSwitch
3. Popup duration — NSTextField (NSNumberFormatter, 0–60s, 1 decimal) + NSStepper (±0.1s) + "s" unit label

Auto-apply on every change. Success is silent; a single footer caption under the card carries transient text (recording help, conflict warnings, "Could not save"). Design decisions: `docs/adr/0001-native-adaptive-appearance.md`; vocabulary: `CONTEXT.md`.

## Fast iteration loop

```sh
tools/settings-preview/iterate.sh            # capture all states × appearances
tools/settings-preview/iterate.sh recording  # one state only
tools/settings-preview/iterate.sh --diff     # gate: exit 0 = match, 3 = drift
tools/settings-preview/iterate.sh --update   # rewrite snapshots/settings/ baselines
```

States: `default`, `recording`, `warning`, `status_err`. Each is captured in `dark` and `light` (`SAFEMIC_PREVIEW_APPEARANCE` pins NSAppearance in the sidecar). Output: `/tmp/safemic-snap/<state>-<appearance>.png` (override dir via `SAFEMIC_SNAP_DIR` for sandboxed runs). Capture is in-process (`bitmapImageRepForCachingDisplayInRect`) — no Screen Recording grant, no window server.

**Read result:** `Read /tmp/safemic-snap/default-dark.png` — multimodal Read returns the image inline. Critique visually, edit `src/settings_window.rs`, re-run. Source change rebuild ≈ 1–3s (slim sidecar dep tree; `src/settings_window.rs` included via `#[path]`, single source of truth).

For pixel-level alignment auditing, scan pixel rows/columns with `magick <img> -crop 1xN+X+Y +repage txt:-` and diff neighboring values — fuzz-trim bboxes lie near soft shadows and bezel halos.

## Pitfalls

- **`cargo run` from project root launches the full safemic tray app**, not the sidecar. Always use iterate.sh for visual iteration.
- **Sidecar needs the window server** — it panics in `NSScreen::mainScreen` under a sandbox that blocks WindowServer. Run unsandboxed.
- **NSTextAlignment uses UIKit numbering here**: 0=left, 1=center, 2=right (verified empirically; `NS_TEXT_ALIGNMENT_*` consts in settings_window.rs).
- **NSSwitch ignores its frame size** — it draws at natural size (~53pt wide) centered in the frame. `sizeToFit` first, then pin the fitted frame's right edge to the control column.
- **NSBoxSeparator draws off-center in its frame** (~1.5pt drift). Use a custom-fill NSBox (`separatorColor`, height 1) for hairlines on an exact grid.
- **CALayer CGColors freeze at set-time** — that's what forced the old dark lock. Use NSBox fill/border (dynamic NSColors, appearance-correct) for custom-drawn surfaces.
- **`cocoa = "0.24"` and `objc = "0.2"` are the legacy crates** (not objc2). Don't mix.
- **Two custom NSObject subclasses** (`MMSettingsActions`, `MMShortcutRecorderView`) registered once behind `OnceLock`. Ivar `_ctxPtr` holds a leaked `Box<ActionContext>`. Don't free.

## Verification before declaring "done"

1. `mise run check` — fmt + clippy CI gate
2. `cargo test` — unit tests
3. `tools/settings-preview/iterate.sh --diff` — exit 0
4. `tools/about-preview/iterate.sh` — exit 0 (sibling gate, both appearances)
