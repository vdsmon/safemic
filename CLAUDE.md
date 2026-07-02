# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build / dev commands

Prefer `mise run <task>` (defined in `mise.toml`) over raw cargo. Build/release targets always cross-compile to `aarch64-apple-darwin` even on Apple Silicon, so cargo-only builds land in a different `target/` subdirectory than `mise run build` and the two won't share artifacts.

| Command | What it does |
|---|---|
| `mise run init` | Install dev deps (mise tools + lefthook hooks). Run once. |
| `mise run start` | `watchexec --poll 500ms` + `cargo run` with `RUST_LOG=info,safemic=trace`. Live-reload dev loop. |
| `mise run build` | `cargo build --locked --release --target aarch64-apple-darwin` then `cargo bundle` → `target/aarch64-apple-darwin/release/bundle/osx/SafeMic.app`. Opens Finder to bundle. |
| `mise run check` (alias `lint`) | `cargo clippy --locked --release -- -D warnings` + `cargo fmt -- --check`. CI gate. |
| `mise run fix` | clippy --fix + cargo fmt. |
| `mise run test` | `cargo test`. |
| `mise run release` | Full release bundle + `rcodesign` self-sign + DMG via `hdiutil`. Requires `sign.crt` (see README for openssl invocation). Dev-time mirror of CI. |
| `git tag vX.Y.Z && git push --tags` | Canonical release action. Triggers `.github/workflows/release.yaml` on `macos-latest`, which runs `mise run release`, publishes a GitHub Release with the DMG + sha256, then opens a PR in `github.com/vdsmon/homebrew-tap` bumping `Casks/safemic.rb`. Requires repo secrets `RCODESIGN_CERT_PEM` (contents of local `sign.crt`) and `TAP_REPO_TOKEN` (fine-grained PAT for the tap repo). |

Single test: `cargo test --release <test_name>` (e.g. `cargo test --release test_settings_json_round_trip`). Tests are colocated with source via `#[cfg(test)] mod tests`.

- `watchexec` must run with `--poll 500ms` on macOS — atomic-rename writes (Edit tool, many IDEs) don't fire FSEvents. Already wired into `mise run start`.
- Homebrew cask formula source-of-truth is `packaging/homebrew/safemic.rb.tmpl` in this repo. Concrete renders ship to `vdsmon/homebrew-tap`. End users install via `brew install --cask vdsmon/tap/safemic`. The cask's `postflight` strips `com.apple.quarantine` because the DMG is rcodesign self-signed, not Apple notarized.

## Visual gates (loop-finder)

This repo has two pre-built self-verifiable visual gates produced by the `loop-finder` skill on 2026-05-27. Use them via the project-scoped `loops` skill (`.claude/skills/loops/`, auto-discovered by Claude Code), or invoke directly:

- `tools/settings-preview/iterate.sh --diff` — settings-window snapshot regression vs `snapshots/settings/*.png`. Exit 0 = match, 3 = drift.
- `tools/about-preview/iterate.sh` — about-window target-conformance vs `~/.claude/loop-finder/60bc3b8f2621/target.png` via `magick compare -metric SSIM`. **NOTE: emits DISSIMILARITY (0=identical) despite metric name; predicate is `dissim ≤ ABOUT_DISSIM_THRESHOLD` (default 0.04).**

Update settings baselines after intentional UI changes: `tools/settings-preview/iterate.sh --update`. Then `git add snapshots/settings/`.

Per-class cache (history, config, summaries) lives at `~/.claude/loop-finder/<class-id>/`. Class ids: settings = `4aa9e37f9396`, about = `60bc3b8f2621`. To add a new class (popup HUD, mic.rs unit-test, etc.), invoke `/loop-finder <task description>`.

Full caveat list — magick SSIM direction, non-monotone-under-extremes, sidecar version-source bug, AppKit setAlignment numbering — lives in `.claude/skills/loops/SKILL.md` and `references/`.

## Architecture

Single-binary tray app. macOS-only (uses cocoa/CoreAudio directly). Entry point `src/main.rs` wires `MicController` + `UI` into one `tao` event loop on the main thread.

### Module ownership

- `main.rs` — load `Settings`, construct `MicController` + `UI`, install SIGTERM/SIGINT handlers (cleanup-on-exit thread restores mic state), hand control to `event_loop::start`.
- `event_loop.rs` — the hub. Single `tao::EventLoop<Message>` running on the main thread, polling four sources on a `WaitUntil` schedule:
  1. `MenuEvent::receiver()` — tray clicks (quit, toggle mute, settings, about).
  2. `GlobalHotKeyEvent::receiver()` — system-wide hotkey presses (filter to `Pressed` only; library fires both press + release).
  3. `MicController` poll every `POLL_INTERVAL_MILLIS=200` ms — re-asserts mute on newly-plugged devices via `should_enforce_mute()`.
  4. Settings file mtime poll every 2 s — when `~/Library/Application Support/safemic/settings.json` changes on disk, reload + `UI::apply_settings(&new)` so external edits take effect without restart.
  Also handles user events: `Message::HidePopup`, `Message::FinalizeHidePopup` (fade-out animation completion), `Message::ApplySettings`, `Message::CloseSettings`.
- `mic.rs` — `MicController`. Wraps CoreAudio `kAudioDevicePropertyMute` on the input scope of every input device; falls back to input-volume-to-zero (with restore on unmute) when the device has no mute property. Polls device list to catch hot-plugged USB/BT mics. iPhone Continuity Mic is intentionally skipped.
- `ui.rs` — `UI` aggregates `Tray`, `Popup`, `SettingsWindow`, `Shortcuts`. `apply_settings()` is the live-reload entry point — idempotent, called by the `Message::ApplySettings` handler (Save click) and the mtime-poll path. When a setting fans out to OS state (launch_at_login plist), `apply_settings` also re-applies that.
- `popup.rs` — borderless `tao` window pinned to the bottom of the cursor's monitor, redrawn on monitor change. Auto-hide via `Arc<AtomicU64>` generation token: every show bumps the counter and spawns a delayed `Message::HidePopup`; stale tokens no-op. `Popup::update` is gated on `last_mic_muted` transition so the 200 ms enforce-mute poll doesn't reset the timer. Fade-out animation via `NSAnimationContext` + delayed `Message::FinalizeHidePopup`.
- `settings_window.rs` — standalone preferences window. tao `Window` (hidden at app start, shown on tray Settings… click) hosting an `NSStackView` of native `NSButton`/`NSTextField`. Save/Cancel wired via `MMSettingsActions` (custom `NSObject` subclass declared via `objc::declare::ClassDecl`, registered once behind `OnceLock<&'static Class>`, ivar `_ctxPtr` holds a leaked `Box<ActionContext>` valid for the app lifetime).
- `tray.rs` — `muda`/`tray-icon` menu, accelerator labels stay in sync with `mic_shortcut` via `update_accelerators()`.
- `shortcuts.rs` — `global-hotkey` registration. `reload()` deregisters + reregisters when the shortcut config changes.
- `settings.rs` — `Settings` struct, serde-tagged `#[serde(default)]` on every field so partial JSON loads cleanly. `Settings::mtime()` is what the event loop polls.
- `config.rs` — `AppVars` (compile-time version/bundle metadata only).
- `launch_at_login.rs` — LaunchAgent plist install/remove.
- `about.rs` — About dialog (NSAlert with project icon + "Open GitHub" button).
- `utils.rs` — `arc_lock`, `get_cursor_pos`, `format_shortcut`.

### Settings flow

`~/Library/Application Support/safemic/settings.json` is the source of truth. Two write paths:
1. Settings window Save click — `MMSettingsActions::saveAction:` writes to `Settings`, persists, then dispatches `Message::ApplySettings` so the main thread reloads + applies.
2. User text-edits the file — mtime poll detects it, calls `Settings::load()` + `UI::apply_settings()`.

Both converge through `UI::apply_settings()` so adding a new setting means: add the field to `Settings` (with `#[serde(default)]`), thread it through any constructor that needs it at init time, and wire the live-apply into `UI::apply_settings()`. Don't bypass `apply_settings`.

### Concurrency

`MicController`, `UI`, `Settings` all live in `Arc<RwLock<_>>` (`arc_lock()` helper in `utils.rs`). Only the main thread touches the `tao` event loop and any `cocoa::id` — UI is `Send + Sync` via `unsafe impl`, but actual UI mutations happen on the main thread inside the event handler. The signal-handler cleanup thread is the only other thread spawned by the app.

### Native AppKit target/action

For NSButton/NSMenuItem callbacks: declare a custom `NSObject` subclass via `objc::declare::ClassDecl` once behind `OnceLock<&'static Class>`. Store handles + proxy in a `Box<Context>` leaked via `Box::into_raw` and pinned to an ivar. Action handlers run synchronously on the main thread (tao drives the run loop there), so single-threaded mutation through the raw pointer is sound. See `settings_window.rs::actions_class` for the template.

## macOS specifics

- App is unsigned by default (no Apple Developer ID). After `mise run build`, users need `xattr -dr com.apple.quarantine "/Applications/SafeMic.app"` or the "Open Anyway" dance in System Settings → Privacy & Security.
- First launch needs **Accessibility** + **Input Monitoring** permissions (global hotkey) and **Microphone** permission (CoreAudio reads). No auto-prompt for Accessibility — user must add it manually.
- `mise run release` uses `rcodesign` with a self-signed cert (`sign.crt` from openssl), not Apple notarization. DMG is unsigned.
- Bundle identifier is hardcoded in `Cargo.toml` `[package.metadata.bundle]`. Don't change it without coordinating with `launch_at_login.rs` plist label.
- `osx_minimum_system_version = "10"` in Cargo.toml is misleading — current dependencies (objc2-core-audio, tao 0.35) require much newer macOS in practice.

## Conventions

- Anyhow `Result<T>` everywhere user-facing, with `.context("...")` at boundary calls. No custom error types.
- `log::trace!` is dense and assumed on in dev (`RUST_LOG=info,safemic=trace` in `mise run start`). `log::error!` for recoverable errors that shouldn't crash. `.unwrap()` is used liberally for invariants that genuinely can't fail.
- `cargo clippy -- -D warnings` is enforced — fix lints, don't `#[allow]` unless there's a real reason. The single `[lints.rust] unexpected_cfgs = "allow"` in Cargo.toml is there because `objc 0.2.x` macros trip it.


<!-- BEGIN BEADS INTEGRATION v:1 profile:minimal hash:7510c1e2 -->
## Beads Issue Tracker

This project uses **bd (beads)** for issue tracking. Run `bd prime` to see full workflow context and commands.

### Quick Reference

```bash
bd ready              # Find available work
bd show <id>          # View issue details
bd update <id> --claim  # Claim work
bd close <id>         # Complete work
```

### Rules

- Use `bd` for ALL task tracking — do NOT use TodoWrite, TaskCreate, or markdown TODO lists
- Run `bd prime` for detailed command reference and session close protocol
- Use `bd remember` for persistent knowledge — do NOT use MEMORY.md files

**Architecture in one line:** issues live in a local Dolt DB; sync uses `refs/dolt/data` on your git remote; `.beads/issues.jsonl` is a passive export. See https://github.com/gastownhall/beads/blob/main/docs/SYNC_CONCEPTS.md for details and anti-patterns.

## Session Completion

**When ending a work session**, you MUST complete ALL steps below. Work is NOT complete until `git push` succeeds.

**MANDATORY WORKFLOW:**

1. **File issues for remaining work** - Create issues for anything that needs follow-up
2. **Run quality gates** (if code changed) - Tests, linters, builds
3. **Update issue status** - Close finished work, update in-progress items
4. **PUSH TO REMOTE** - This is MANDATORY:
   ```bash
   git pull --rebase
   git push
   git status  # MUST show "up to date with origin"
   ```
5. **Clean up** - Clear stashes, prune remote branches
6. **Verify** - All changes committed AND pushed
7. **Hand off** - Provide context for next session

**CRITICAL RULES:**
- Work is NOT complete until `git push` succeeds
- NEVER stop before pushing - that leaves work stranded locally
- NEVER say "ready to push when you are" - YOU must push
- If push fails, resolve and retry until it succeeds
<!-- END BEADS INTEGRATION -->
