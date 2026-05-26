# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build / dev commands

Prefer `mise run <task>` (defined in `mise.toml`) over raw cargo. Build/release targets always cross-compile to `aarch64-apple-darwin` even on Apple Silicon, so cargo-only builds land in a different `target/` subdirectory than `mise run build` and the two won't share artifacts.

| Command | What it does |
|---|---|
| `mise run init` | Install dev deps (mise tools + lefthook hooks). Run once. |
| `mise run start` | `watchexec --poll 500ms` + `cargo run` with `RUST_LOG=info,mic_mute=trace`. Live-reload dev loop. |
| `mise run build` | `cargo build --locked --release --target aarch64-apple-darwin` then `cargo bundle` → `target/aarch64-apple-darwin/release/bundle/osx/Mic Mute.app`. Opens Finder to bundle. |
| `mise run check` (alias `lint`) | `cargo clippy --locked --release -- -D warnings` + `cargo fmt -- --check`. CI gate. |
| `mise run fix` | clippy --fix + cargo fmt. |
| `mise run test` | `cargo test`. |
| `mise run release` | Full release bundle + `rcodesign` self-sign + DMG via `hdiutil`. Requires `sign.crt` (see README for openssl invocation). |

Single test: `cargo test --release <test_name>` (e.g. `cargo test --release test_settings_json_round_trip`). Tests are colocated with source via `#[cfg(test)] mod tests`.

- `watchexec` must run with `--poll 500ms` on macOS — atomic-rename writes (Edit tool, many IDEs) don't fire FSEvents. Already wired into `mise run start`.

## Architecture

Single-binary tray app. macOS-only (uses cocoa/CoreAudio directly). Entry point `src/main.rs` wires `MicController` + `UI` into one `tao` event loop on the main thread.

### Module ownership

- `main.rs` — load `Settings`, construct `MicController` + `UI`, install SIGTERM/SIGINT handlers (cleanup-on-exit thread restores mic state), hand control to `event_loop::start`.
- `event_loop.rs` — the hub. Single `tao::EventLoop<Message>` running on the main thread, polling four sources on a `WaitUntil` schedule:
  1. `MenuEvent::receiver()` — tray clicks (quit, toggle mute, settings, about).
  2. `GlobalHotKeyEvent::receiver()` — system-wide hotkey presses (filter to `Pressed` only; library fires both press + release).
  3. `MicController` poll every `POLL_INTERVAL_MILLIS=200` ms — re-asserts mute on newly-plugged devices via `should_enforce_mute()`.
  4. Settings file mtime poll every 2 s — when `~/Library/Application Support/mic-mute/settings.json` changes on disk, reload + `UI::apply_settings(&new)` so external edits take effect without restart.
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

`~/Library/Application Support/mic-mute/settings.json` is the source of truth. Two write paths:
1. Settings window Save click — `MMSettingsActions::saveAction:` writes to `Settings`, persists, then dispatches `Message::ApplySettings` so the main thread reloads + applies.
2. User text-edits the file — mtime poll detects it, calls `Settings::load()` + `UI::apply_settings()`.

Both converge through `UI::apply_settings()` so adding a new setting means: add the field to `Settings` (with `#[serde(default)]`), thread it through any constructor that needs it at init time, and wire the live-apply into `UI::apply_settings()`. Don't bypass `apply_settings`.

### Concurrency

`MicController`, `UI`, `Settings` all live in `Arc<RwLock<_>>` (`arc_lock()` helper in `utils.rs`). Only the main thread touches the `tao` event loop and any `cocoa::id` — UI is `Send + Sync` via `unsafe impl`, but actual UI mutations happen on the main thread inside the event handler. The signal-handler cleanup thread is the only other thread spawned by the app.

### Native AppKit target/action

For NSButton/NSMenuItem callbacks: declare a custom `NSObject` subclass via `objc::declare::ClassDecl` once behind `OnceLock<&'static Class>`. Store handles + proxy in a `Box<Context>` leaked via `Box::into_raw` and pinned to an ivar. Action handlers run synchronously on the main thread (tao drives the run loop there), so single-threaded mutation through the raw pointer is sound. See `settings_window.rs::actions_class` for the template.

## macOS specifics

- App is unsigned by default (no Apple Developer ID). After `mise run build`, users need `xattr -dr com.apple.quarantine "/Applications/Mic Mute.app"` or the "Open Anyway" dance in System Settings → Privacy & Security.
- First launch needs **Accessibility** + **Input Monitoring** permissions (global hotkey) and **Microphone** permission (CoreAudio reads). No auto-prompt for Accessibility — user must add it manually.
- `mise run release` uses `rcodesign` with a self-signed cert (`sign.crt` from openssl), not Apple notarization. DMG is unsigned.
- Bundle identifier is hardcoded in `Cargo.toml` `[package.metadata.bundle]`. Don't change it without coordinating with `launch_at_login.rs` plist label.
- `osx_minimum_system_version = "10"` in Cargo.toml is misleading — current dependencies (objc2-core-audio, tao 0.35) require much newer macOS in practice.

## Conventions

- Anyhow `Result<T>` everywhere user-facing, with `.context("...")` at boundary calls. No custom error types.
- `log::trace!` is dense and assumed on in dev (`RUST_LOG=info,mic_mute=trace` in `mise run start`). `log::error!` for recoverable errors that shouldn't crash. `.unwrap()` is used liberally for invariants that genuinely can't fail.
- `cargo clippy -- -D warnings` is enforced — fix lints, don't `#[allow]` unless there's a real reason. The single `[lints.rust] unexpected_cfgs = "allow"` in Cargo.toml is there because `objc 0.2.x` macros trip it.
