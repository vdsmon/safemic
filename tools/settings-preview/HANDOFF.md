# Settings UI handoff

Cold-start brief for the next agent iterating on `src/settings_window.rs`.

## Goal

Settings window for SafeMic (macOS tray app, Rust + Cocoa via objc 0.2 + tao 0.35) needs to look SUPER clean. User bar is high: zero visible misalignment, zero dead space, zero stylistic dissonance. Native macOS preferences-pane vibe (think modern Apple Settings panel, not gamer HUD).

Three form rows live in the window:
1. Mute shortcut — chip displaying `⇧⌘A` style combo; whole chip is click-to-record
2. Launch at login — NSSwitch toggle
3. Popup duration — `NSTextField` (monospaced digit) + `ms` helper

Window auto-applies on every control change (no Save/Cancel). Status dot bottom-right pulses on persist (currently invisible by default, alpha=0 → fades in on save).

## Fast iteration loop

Use this. Not `mise run start`. Not `cargo run`.

```sh
tools/settings-preview/iterate.sh
# or
mise run ui:preview
```

**What it does:** rebuilds a tiny sidecar binary (~1-2s incremental), launches it, waits for the "Settings window opened" sentinel in the log, captures the window via `screencapture -l`, writes `/tmp/settings.png`. Sidecar process stays alive until next iterate.sh run kills + relaunches it.

**Read result:** `Read /tmp/settings.png` — multimodal Read returns the image inline. Critique visually, edit `src/settings_window.rs`, re-run.

**Per-cycle wall time:**
- No source change: 1.2s
- Source change: 2.7s
- (Was 5-7s with the old kill-and-restart-full-safemic loop)

**Why sidecar is fast:** `tools/settings-preview/` is a workspace member with a slim dep tree — no coreaudio, no tray-icon, no muda, no image/resvg, no global-hotkey, no async-std. `src/settings_window.rs` is included via `#[path]` so changes flow to both binaries. Same Cocoa/objc/tao surface = 100% visual fidelity vs the real app.

**Pre-built tools at /tmp:**
- `/tmp/getwin` — compiled Swift binary that lists on-screen windows owned by `safemic` or `settings-preview` (returns `wid<tab>layer<tab>owner<tab>name`)
- `/tmp/getwin.swift` — source (recompile with `swiftc -O /tmp/getwin.swift -o /tmp/getwin` if it disappears)

## What's already been done

See `.rapidfire/T11-rebrand-safemic.md` for the rename commit `646feda` that landed `mic-mute → SafeMic`.

After that, an iteration pass redesigned the settings window from an editorial-typographic HUD (serif "Settings" wordmark, all-caps small section labels, vibrant dark blur background) to a native form layout:

- Visible standard titlebar with "Settings" title (was custom serif wordmark inside content)
- Sentence-case labels right-aligned at gutter via `place_form_label` helper (was left-aligned ALL-CAPS small text)
- 3 form rows, controls left-aligned at `CONTROL_X`, sharing `CONTROL_W=72` width (chip + field), toggle right-aligned to that same edge
- Window 320×118 (was 480×280 originally; chase the bottom)
- Yellow/green traffic lights hidden (chrome non-resizable, non-minimizable — greyed lights read "broken")
- Chip background + border matched to NSTextField dark-mode appearance (rgb 30,30,30 bg; rgb 86,86,86 α=0.6 border)
- Chip text vertical-centered manually (NSTextField centerAlignment unreliable for keyboard-symbol glyphs)
- Locked appearance to DarkAqua (CALayer chip colors don't auto-adapt; light mode would render badly)
- Stripped 7 dead functions + 5 dead consts left over from editorial design
- Lifted every inline magic to named module-level const (`SIDE_PAD`, `TOP_PAD`, `ROW_H`, `ROW_GAP`, `LABEL_GUTTER`, `CONTROL_X`, `CONTROL_W`, `CONTROL_H`, `SWITCH_W/H`, `HELP_H`, `HELP_LEFT_GAP`, `STATUS_DOT_SIZE`, `STATUS_LABEL_W/H`, `STATUS_BOTTOM_MARGIN`, `WARNING_H`)
- SIGUSR1 handler added to main app (`src/event_loop.rs:OPEN_SETTINGS_REQUESTED` + `handle_sigusr1` registered in `src/main.rs`) so the running tray app can be poked to open Settings without clicking the tray menu. (Sidecar doesn't need this — auto-opens at launch.)

These changes are **uncommitted** — `git status` shows: `Cargo.toml`, `Cargo.lock`, `mise.toml`, `src/event_loop.rs`, `src/main.rs`, `src/settings_window.rs`, plus `tools/` directory. User has not given commit/push permission yet — ask before committing.

## Known open issues

Last critique pass (before this handoff) saw:

- **Vertical alignment** mostly fixed by `place_form_label` + chip-y centering, but verify with fresh capture; baseline mismatches between SF text and keyboard-symbol glyphs in the chip are still slightly visible (M character cap-height ≠ ⇧⌘ glyph optical height).
- **NSSwitch off-state contrast** is dim — macOS limitation; can't tint off-state with public API. Wrapping in a backing surface didn't help. Acceptable per current bar but worth a custom toggle if user pushes back.
- **Status indicator (dot + label, bottom-right)** rendered invisible by default (alpha 0); only appears on save success/error pulse. If positioning is wrong it'll never be noticed in capture. Trigger a save to verify.
- **Recording state** (chip text → "press a combo…") not yet captured/critiqued. To see it, modify Settings; click chip → keyboard recorder activates; capture mid-recording.
- **Warning state** (collision detected, e.g., ⌘Q) also not captured/critiqued. To see it, attempt to set shortcut to a reserved combo.

## Pitfalls

- **`cargo run` from project root launches the full safemic tray app**, not the sidecar. Always use `tools/settings-preview/iterate.sh` for visual iteration.
- **Two safemic instances at once = two tray icons** (overlap, confusing). Always `pkill -f safemic` before starting fresh.
- **Watchexec dies silently** if `mise run start` is backgrounded with `&` from inside a non-interactive shell. Use the sidecar instead; don't try to revive watchexec.
- **`osascript`/`System Events`** for clicking the tray menu is blocked — would need Accessibility grant for `osascript`. Sidecar's auto-open at startup sidesteps this entirely.
- **Screen Recording permission required** for `screencapture -l <id>`. Granted on the Claude Code CLI; if you see `could not create image from window`, that grant was revoked.
- **Light mode would break the chip** (CALayer colors don't auto-adapt). Window is locked to DarkAqua to avoid this. If you ever undo that lock, also fix chip color to use semantic NSColor + redraw on appearance change.
- **`cocoa = "0.24"` and `objc = "0.2"` are the legacy crates** (not `cocoa-foundation`, not `objc2`). Don't mix. `Cargo.toml` has a `[lints.rust] unexpected_cfgs = "allow"` shim because objc 0.2 macros trip `unexpected_cfgs`.
- **NSSwitch state set via `setState: i64`** (0/1), not bool.
- **Two custom NSObject subclasses** (`MMSettingsActions`, `MMShortcutRecorderView`) are declared via `objc::declare::ClassDecl`, registered once behind `OnceLock<&'static Class>`. Ivar `_ctxPtr` holds a leaked `Box<ActionContext>` valid for app lifetime. Don't free.

## Files to know

- `src/settings_window.rs` — layout. Edit this. (~1100 LoC, but the meaty `build_content_view` is ~150 LoC near the top.)
- `src/shortcut_recorder.rs` — keyboard capture for the record-chip flow. Included via `#[path]` in `settings_window.rs`. Same in sidecar.
- `tools/settings-preview/src/main.rs` — sidecar entry. Has the stub modules (`event_loop`, `settings`, `shortcuts`, `utils`) that satisfy `settings_window.rs`'s `crate::` imports.
- `tools/settings-preview/Cargo.toml` — slim deps. Don't bloat.
- `tools/settings-preview/iterate.sh` — build + launch + capture script. Tweak if you need extra options (e.g., crop, multi-capture).
- `CLAUDE.md` (project root) — module-level architecture notes.

## Layout-constant cheat sheet

In `src/settings_window.rs`:

| Const | Default | What it controls |
|---|---|---|
| `WINDOW_SIZE` | 320×118 | content area; titlebar adds ~28pt |
| `SIDE_PAD` | 24 | left/right window padding |
| `TOP_PAD` | 14 | space below titlebar before first row |
| `ROW_H` | 24 | row height for label + control |
| `ROW_GAP` | 10 | vertical space between rows |
| `LABEL_GUTTER` | 170 | x where right-aligned label text ends |
| `CONTROL_X` | 178 | left edge of chip/field/(reserved for toggle) |
| `CONTROL_W` | 72 | chip + field shared width |
| `CONTROL_H` | 22 | field height (NSTextField) |
| `SWITCH_W/H` | 38/22 | NSSwitch dimensions |
| `HELP_H` | 13 | helper text ("ms") height |
| `HELP_LEFT_GAP` | 10 | gap from field right edge to helper |

Change one number → re-run iterate.sh → screencapture. Tight feedback loop.

## Verification before declaring "done"

1. `cargo build --target aarch64-apple-darwin` (root) — main app builds
2. `cargo clippy --locked --release -- -D warnings` — CI gate
3. `cargo test --target aarch64-apple-darwin` — 46 tests pass
4. `mise run ui:preview` — sidecar launches, `/tmp/settings.png` exists, looks clean
5. `mise run start` — real tray app shows the same Settings window via tray menu
6. ASK USER before committing or pushing. They have a "no auto-push" rule.
