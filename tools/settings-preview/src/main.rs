// Permit unused API in the included settings_window.rs (some methods are only
// called by the full safemic event loop, not by the sidecar's main).
#![allow(dead_code)]

//! Standalone Settings-window iteration sidecar.
//!
//! Hosts only `src/settings_window.rs` (and its `shortcut_recorder.rs` sibling)
//! plus a handful of stubs for the cross-module references the layout code
//! makes. The result is a tiny binary whose dep tree excludes coreaudio,
//! tray-icon, muda, image, resvg, global-hotkey, async-std — so cargo can
//! incrementally rebuild it in ~1s when only settings_window.rs changes.
//!
//! Run with `cargo run -p settings-preview --target aarch64-apple-darwin` or
//! via the wrapper at `tools/settings-preview/iterate.sh` which also captures.

#[macro_use]
extern crate objc;

use anyhow::Result;
use log::info;
use std::sync::{Arc, RwLock};
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};

// Stubs that satisfy the `crate::{event_loop, settings, shortcuts, utils}`
// imports inside `settings_window.rs`. Kept intentionally tiny — the real
// Settings round-trip is irrelevant for visual iteration.

pub mod settings {
    #[derive(Debug, Clone)]
    pub struct ShortcutConfig {
        pub modifiers: Vec<String>,
        pub key: String,
    }

    impl Default for ShortcutConfig {
        fn default() -> Self {
            Self {
                modifiers: vec!["shift".to_string(), "meta".to_string()],
                key: "A".to_string(),
            }
        }
    }

    #[derive(Debug, Clone)]
    pub struct Settings {
        pub mic_shortcut: ShortcutConfig,
        pub launch_at_login: bool,
        pub popup_duration_ms: u64,
    }

    impl Default for Settings {
        fn default() -> Self {
            Self {
                mic_shortcut: ShortcutConfig::default(),
                launch_at_login: false,
                popup_duration_ms: 1000,
            }
        }
    }

    impl Settings {
        pub fn load() -> Self {
            Self::default()
        }

        pub fn save(&self) -> anyhow::Result<()> {
            // sidecar: in-memory only, never persisted.
            Ok(())
        }
    }
}

pub mod event_loop {
    use super::settings::ShortcutConfig;

    #[derive(Debug)]
    pub enum Message {
        ApplySettings {
            previous_shortcut: Option<ShortcutConfig>,
        },
        CloseSettings,
        // Sidecar-only: trigger the snapshot timeline once the window has
        // settled / animations have run.
        Snapshot,
    }

    pub type EventLoopMessage = tao::event_loop::EventLoop<Message>;
    pub type EventLoopProxyMessage = tao::event_loop::EventLoopProxy<Message>;
}

pub mod shortcuts {
    use super::settings::ShortcutConfig;

    #[derive(Debug)]
    pub enum ShortcutConflict {
        ReservedByMacOS(&'static str),
        InvalidKey(String),
    }

    impl std::fmt::Display for ShortcutConflict {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                ShortcutConflict::ReservedByMacOS(name) => write!(f, "Reserved by macOS: {}", name),
                ShortcutConflict::InvalidKey(key) => write!(f, "Invalid key: {}", key),
            }
        }
    }

    // Accept-everything stub. The real validator (src/shortcuts.rs) checks
    // against a list of macOS-reserved combos; visual iteration doesn't need
    // that path.
    pub fn validate_shortcut(_config: &ShortcutConfig) -> Result<(), ShortcutConflict> {
        Ok(())
    }
}

pub mod utils {
    use super::settings::ShortcutConfig;

    pub fn format_shortcut(config: &ShortcutConfig) -> String {
        let mut parts: Vec<&str> = vec![];
        for modifier in &config.modifiers {
            match modifier.as_str() {
                "shift" => parts.push("\u{21E7}"),
                "meta" | "cmd" | "command" => parts.push("\u{2318}"),
                "ctrl" | "control" => parts.push("\u{2303}"),
                "alt" | "option" => parts.push("\u{2325}"),
                _ => {}
            }
        }
        parts.push(config.key.as_str());
        parts.join("")
    }
}

// Pull the real layout code in verbatim. Changes to src/settings_window.rs
// flow to this binary automatically — single source of truth.
#[path = "../../../src/settings_window.rs"]
mod settings_window;

use event_loop::Message;
use settings::Settings;
use settings_window::SettingsWindow;

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_timestamp(None)
        .init();

    let mut event_loop = EventLoopBuilder::<Message>::with_user_event().build();
    event_loop.set_activation_policy(ActivationPolicy::Regular);

    let settings = Arc::new(RwLock::new(Settings::default()));
    let proxy = event_loop.create_proxy();

    let mut window = SettingsWindow::new(&event_loop)?;
    window.bind_actions(settings.clone(), proxy.clone());
    window.open(&settings.read().unwrap());
    let window_id = window.id();

    // Snapshot mode: drive the window into `SAFEMIC_PREVIEW_STATE`, wait one
    // event-loop tick + the requested settle delay, snapshot to
    // `SAFEMIC_PREVIEW_SNAPSHOT`, then exit. iterate.sh loops over states.
    let snapshot_path = std::env::var("SAFEMIC_PREVIEW_SNAPSHOT").ok();
    let state = std::env::var("SAFEMIC_PREVIEW_STATE").unwrap_or_default();
    let settle_ms: u64 = std::env::var("SAFEMIC_PREVIEW_SETTLE_MS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(220);

    if !state.is_empty() {
        window.preview_state(&state);
    }

    info!("Settings window opened (state={state})");

    // Snapshot mode: pump the NSRunLoop briefly so the window renders + any
    // pulse-in animations land, then write the PNG and exit. Avoids the
    // tao event_loop entirely — UserEvent wakeup is flaky on macOS.
    if let Some(path) = snapshot_path {
        pump_run_loop_ms(settle_ms);
        if let Err(e) = window.snapshot_to_png(&path) {
            log::error!("snapshot failed: {e:#}");
            std::process::exit(1);
        }
        info!("snapshot -> {path}");
        std::process::exit(0);
    }

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::UserEvent(Message::CloseSettings) => {
                window.close();
                *control_flow = ControlFlow::Exit;
            }
            Event::UserEvent(Message::ApplySettings { .. }) => {
                let s = settings.read().unwrap();
                window.refresh_from(&s);
            }
            Event::UserEvent(Message::Snapshot) => {}
            Event::WindowEvent {
                window_id: id,
                event: WindowEvent::CloseRequested,
                ..
            } if id == window_id => {
                window.close();
                *control_flow = ControlFlow::Exit;
            }
            _ => {}
        }
    });
}

/// Pump the main thread's NSRunLoop for approximately `ms` milliseconds so
/// pending AppKit work (window layout, layer commits, animations) lands.
fn pump_run_loop_ms(ms: u64) {
    use objc::{class, msg_send, runtime::Object, sel, sel_impl};
    let secs = ms as f64 / 1000.0;
    unsafe {
        let date: *mut Object = msg_send![class!(NSDate), dateWithTimeIntervalSinceNow: secs];
        let run_loop: *mut Object = msg_send![class!(NSRunLoop), currentRunLoop];
        let _: bool = msg_send![run_loop, runUntilDate: date];
    }
}
