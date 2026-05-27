use crate::about::show_about;
use crate::mic::MicController;
use crate::settings::{Settings, ShortcutConfig};
use crate::ui::UI;
use global_hotkey::GlobalHotKeyEvent;
use log::trace;
use muda::{MenuEvent, MenuId};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoop, EventLoopBuilder};
use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};

const POLL_INTERVAL_MILLIS: u64 = 200;

#[derive(Debug)]
pub enum Message {
    HidePopup,
    FinalizeHidePopup,
    /// Re-apply the in-memory settings to the live UI. `previous_shortcut`
    /// carries the value of `mic_shortcut` before the change so the event
    /// loop can roll back if `Shortcuts::reload` fails (only set when the
    /// shortcut itself changed; `None` for unrelated edits).
    ApplySettings {
        previous_shortcut: Option<ShortcutConfig>,
    },
    CloseSettings,
}

pub type EventLoopMessage = EventLoop<Message>;
pub type EventLoopProxyMessage = tao::event_loop::EventLoopProxy<Message>;

pub fn create() -> EventLoopMessage {
    EventLoopBuilder::<Message>::with_user_event().build()
}

pub struct EventIds {
    pub button_toggle_mute: MenuId,
    pub button_settings: MenuId,
    pub button_about: MenuId,
    pub button_quit: MenuId,
    pub shortcut_mic: Arc<AtomicU32>,
}

fn update_mic(ui: Arc<RwLock<UI>>, controller: Arc<RwLock<MicController>>, toggle: bool) {
    let mut controller = controller.write().unwrap();
    if toggle || controller.should_enforce_mute() {
        let state = if toggle { None } else { Some(true) };
        if let Err(err) = controller.toggle(state) {
            log::error!("Failed to update microphone mute state: {}", err);
        }
        let device_name = controller.active_device_name();
        let mut ui = ui.write().unwrap();
        ui.update_mic(controller.muted, device_name.as_deref())
            .unwrap();
    }
    // popup auto-hide is armed inside `Popup::update` via its
    // generation-token timer, no per-toggle scheduling needed here
}

pub fn restore_microphone_on_exit(controller: &Arc<RwLock<MicController>>) {
    if let Err(err) = controller.write().unwrap().restore_on_exit() {
        log::error!("Failed to restore microphone state on exit: {}", err);
    }
}

pub fn start(
    mut event_loop: EventLoop<Message>,
    event_ids: EventIds,
    ui: Arc<RwLock<UI>>,
    controller: Arc<RwLock<MicController>>,
    settings: Arc<RwLock<Settings>>,
) {
    let EventIds {
        button_toggle_mute,
        button_settings,
        button_about,
        button_quit,
        shortcut_mic,
    } = event_ids;

    let poll_interval = Duration::from_millis(POLL_INTERVAL_MILLIS);
    // Start in the past so the first iteration triggers the poll immediately.
    let mut last_poll = Instant::now() - poll_interval;

    // Poll the settings file for changes every 2 seconds so edits to
    // settings.json take effect without restarting the app.
    let settings_poll_interval = Duration::from_secs(2);
    let mut last_settings_check = Instant::now();
    let mut last_settings_mtime = Settings::mtime();

    trace!("Starting event loop");
    let proxy = event_loop.create_proxy();
    ui.write()
        .unwrap()
        .bind_settings_window_actions(settings.clone(), proxy.clone());
    let settings_window_id = ui.read().unwrap().settings_window_id();
    // Tray-only app, never appears in the dock.
    event_loop.set_activation_policy(ActivationPolicy::Accessory);
    event_loop.run(move |event, _, control_flow| {
        let mut exit_requested = false;

        match event {
            Event::UserEvent(Message::HidePopup) => {
                trace!("HidePopup received");
                let mut ui = ui.write().unwrap();
                ui.hide_popup().unwrap();
            }
            Event::UserEvent(Message::FinalizeHidePopup) => {
                trace!("FinalizeHidePopup received");
                let mut ui = ui.write().unwrap();
                ui.finalize_hide_popup().unwrap();
            }
            Event::UserEvent(Message::ApplySettings { previous_shortcut }) => {
                // trust the in-memory settings: save_action may have failed to
                // write to disk, in which case reloading would silently lose
                // the user's edits.
                let new_settings = settings.read().unwrap().clone();
                let mut ui_w = ui.write().unwrap();
                match ui_w.apply_settings(&new_settings) {
                    Ok(()) => {
                        shortcut_mic.store(ui_w.mic_shortcut_id(), Ordering::Relaxed);
                        trace!("Settings applied via Save");
                    }
                    Err(e) => {
                        log::error!("Failed to apply settings: {}", e);
                        // If a shortcut change is what blew up, restore the
                        // prior shortcut in-memory + on-disk so a restart
                        // recovers, then re-apply.
                        if let Some(prev) = previous_shortcut {
                            log::error!(
                                "Rolling back mic_shortcut to {:?} after apply failure",
                                prev
                            );
                            let mut s = settings.write().unwrap();
                            s.mic_shortcut = prev;
                            if let Err(save_err) = s.save() {
                                log::error!("Failed to persist rollback: {}", save_err);
                            }
                            let restored = s.clone();
                            drop(s);
                            if let Err(retry_err) = ui_w.apply_settings(&restored) {
                                log::error!("Failed to re-apply after rollback: {}", retry_err);
                            } else {
                                shortcut_mic.store(ui_w.mic_shortcut_id(), Ordering::Relaxed);
                            }
                        }
                    }
                }
                // acknowledge the disk state regardless of whether the save
                // succeeded, so the mtime poll doesn't trigger an unnecessary
                // reload immediately after.
                last_settings_mtime = Settings::mtime();
            }
            Event::UserEvent(Message::CloseSettings) => {
                ui.read().unwrap().close_settings_window();
            }
            Event::WindowEvent {
                window_id,
                event: WindowEvent::CloseRequested,
                ..
            } if window_id == settings_window_id => {
                ui.read().unwrap().close_settings_window();
            }
            _ => {}
        };

        if let Ok(event) = MenuEvent::receiver().try_recv() {
            trace!("Tray menu event: {:?}", event);
            if event.id == button_quit {
                trace!("Exit tray menu item selected");
                exit_requested = true;
            } else if event.id == button_toggle_mute {
                trace!("Toggle mic tray menu item selected");
                update_mic(ui.clone(), controller.clone(), true);
            } else if event.id == button_settings {
                trace!("Settings tray menu item selected");
                let s = settings.read().unwrap();
                ui.read().unwrap().open_settings_window(&s);
            } else if event.id == button_about {
                trace!("About tray menu item selected");
                if let Err(e) = show_about() {
                    log::error!("About dialog error: {}", e);
                }
            }
        }

        if let Ok(event) = GlobalHotKeyEvent::receiver().try_recv() {
            // Only act on key-down; global-hotkey fires both Pressed and Released
            if event.state() == global_hotkey::HotKeyState::Pressed {
                let id = event.id();
                if shortcut_mic.load(Ordering::Relaxed) == id {
                    trace!("Toggle mic shortcut activated");
                    update_mic(ui.clone(), controller.clone(), true);
                }
            }
        }

        // Reload settings if the file has been modified since we last checked.
        // Skipped while the Settings window is open so the user's in-progress
        // edits aren't clobbered by an external write picked up mid-edit.
        if last_settings_check.elapsed() >= settings_poll_interval {
            last_settings_check = Instant::now();
            let current_mtime = Settings::mtime();
            let settings_window_open = ui.read().unwrap().is_settings_window_open();
            if current_mtime != last_settings_mtime && !settings_window_open {
                last_settings_mtime = current_mtime;
                trace!("settings.json changed on disk, reloading");
                let new_settings = Settings::load();
                let mut s = settings.write().unwrap();
                *s = new_settings.clone();
                drop(s);
                let mut ui_w = ui.write().unwrap();
                if let Err(e) = ui_w.apply_settings(&new_settings) {
                    log::error!("Failed to apply reloaded settings: {}", e);
                } else {
                    shortcut_mic.store(ui_w.mic_shortcut_id(), Ordering::Relaxed);
                    trace!("Settings reloaded from settings.json");
                }
            }
        }

        // Poll mic state and cursor-monitor position on a 200 ms interval.
        if last_poll.elapsed() >= poll_interval {
            last_poll = Instant::now();
            update_mic(ui.clone(), controller.clone(), false);
            let mut ui_w = ui.write().unwrap();
            ui_w.detect().unwrap();
        }

        if exit_requested {
            restore_microphone_on_exit(&controller);
            *control_flow = ControlFlow::Exit;
        } else {
            // Sleep until the next scheduled check rather than spinning.
            let next_poll = last_poll + poll_interval;
            let next_settings = last_settings_check + settings_poll_interval;
            *control_flow = ControlFlow::WaitUntil(next_poll.min(next_settings));
        }
    });
}
