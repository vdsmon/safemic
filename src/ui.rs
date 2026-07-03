use crate::config::AppVars;
use crate::event_loop::{create, EventIds, EventLoopMessage, EventLoopProxyMessage};
use crate::popup::Popup;
use crate::settings::{Settings, ThemePreference};
use crate::settings_window::SettingsWindow;
use crate::shortcuts::Shortcuts;
use crate::tray::Tray;
use crate::utils::system_theme;
use anyhow::{Context, Result};
use cocoa::base::{id, nil};
use cocoa::foundation::NSString;
use log::trace;
use std::sync::atomic::AtomicU32;
use std::sync::{Arc, RwLock};
use tao::window::WindowId;

/// Event loop must remain on the main thread and doesn't implement Copy
#[allow(dead_code)]
pub struct UI {
    tray: Tray,
    popup: Popup,
    settings_window: SettingsWindow,
    shortcuts: Shortcuts,
    mic_muted: bool,
}

unsafe impl Send for UI {}
unsafe impl Sync for UI {}

impl UI {
    pub fn new(
        mic_muted: bool,
        app_vars: AppVars,
        settings: &Settings,
    ) -> Result<(Self, EventLoopMessage, EventIds)> {
        let event_loop = create();
        apply_theme(settings.theme);
        let popup = Popup::new(&event_loop, mic_muted, settings.popup_duration_ms)
            .context("Failed to setup popup window")?;
        let settings_window =
            SettingsWindow::new(&event_loop).context("Failed to setup settings window")?;
        let tray = Tray::new(mic_muted, system_theme(), app_vars, &settings.mic_shortcut)
            .context("Failed to create system tray")?;
        let shortcuts = Shortcuts::new(settings).context("Failed to setup shortcuts")?;

        let event_ids = EventIds {
            button_toggle_mute: tray.toggle_mute_id().clone(),
            button_settings: tray.settings_id().clone(),
            button_about: tray.about_id().clone(),
            button_quit: tray.quit_id().clone(),
            shortcut_mic: Arc::new(AtomicU32::new(shortcuts.mic_hotkey.id())),
        };

        let ui = Self {
            tray,
            popup,
            settings_window,
            shortcuts,
            mic_muted,
        };
        Ok((ui, event_loop, event_ids))
    }

    pub fn settings_window_id(&self) -> WindowId {
        self.settings_window.id()
    }

    pub fn open_settings_window(&self, settings: &Settings) {
        self.settings_window.open(settings);
    }

    pub fn close_settings_window(&self) {
        self.settings_window.close();
    }

    pub fn is_settings_window_open(&self) -> bool {
        self.settings_window.is_open()
    }

    pub fn bind_settings_window_actions(
        &mut self,
        settings: Arc<RwLock<Settings>>,
        proxy: EventLoopProxyMessage,
    ) {
        self.settings_window.bind_actions(settings, proxy);
    }

    pub fn update_mic(
        &mut self,
        muted: bool,
        active_device_name: Option<&str>,
    ) -> Result<&mut Self> {
        trace!("Updating UI mic state {}", muted);
        self.mic_muted = muted;
        self.tray
            .update(muted, system_theme())
            .context("Failed to update UI tray")?;
        self.popup
            .update(muted, active_device_name)
            .context("Failed to update UI popup")?;
        Ok(self)
    }

    pub fn hide_popup(&mut self) -> Result<&mut Self> {
        self.popup.hide().context("Failed to hide UI popup")?;
        Ok(self)
    }

    pub fn finalize_hide_popup(&mut self) -> Result<&mut Self> {
        self.popup
            .finalize_hide()
            .context("Failed to finalize UI popup hide")?;
        Ok(self)
    }

    /// Apply all settings to the live app state.
    /// Safe to call whenever settings change — all operations are idempotent.
    pub fn apply_settings(&mut self, settings: &Settings) -> Result<()> {
        // Re-register hotkeys and update tray accelerator labels
        self.shortcuts.reload(settings)?;
        self.tray
            .update_accelerators(&settings.mic_shortcut)
            .context("Failed to update tray accelerators")?;

        // Apply popup duration live (hides any currently-visible bezel when set to 0)
        self.popup.set_popup_duration_ms(settings.popup_duration_ms);

        apply_theme(settings.theme);

        if let Err(e) = crate::launch_at_login::set(settings.launch_at_login) {
            log::error!("Failed to apply launch_at_login setting: {}", e);
        }

        // Propagate external settings changes to the visible settings UI.
        self.settings_window.refresh_from(settings);

        Ok(())
    }

    pub fn mic_shortcut_id(&self) -> u32 {
        self.shortcuts.mic_hotkey.id()
    }

    pub fn set_hotkey_suspended(&mut self, suspended: bool) {
        if suspended {
            self.shortcuts.suspend();
        } else if let Err(e) = self.shortcuts.resume() {
            log::error!("Failed to re-register hotkey after recording: {}", e);
        }
    }

    pub fn detect(&mut self) -> Result<&mut Self> {
        self.popup
            .detect_cursor_monitor()
            .context("Failed to update UI popup placement")?;
        Ok(self)
    }
}

/// NSApp-level appearance override; `System` clears it so every window
/// follows the OS setting. Idempotent, main-thread only (called from UI
/// construction and the ApplySettings handler).
fn apply_theme(theme: ThemePreference) {
    let name = match theme {
        ThemePreference::System => None,
        ThemePreference::Light => Some("NSAppearanceNameAqua"),
        ThemePreference::Dark => Some("NSAppearanceNameDarkAqua"),
    };
    unsafe {
        let app: id = msg_send![class!(NSApplication), sharedApplication];
        let appearance: id = match name {
            None => nil,
            Some(name) => {
                let ns_name = NSString::alloc(nil).init_str(name);
                let appearance: id = msg_send![class!(NSAppearance), appearanceNamed: ns_name];
                let _: () = msg_send![ns_name, release];
                appearance
            }
        };
        let _: () = msg_send![app, setAppearance: appearance];
    }
}
