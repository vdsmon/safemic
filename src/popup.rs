use crate::event_loop::{EventLoopMessage, EventLoopProxyMessage, Message};
use crate::popup_content::PopupContent;
use crate::utils::get_cursor_pos;
use anyhow::{Context, Result};
use async_std::task;
use cocoa::{
    appkit::{NSView, NSWindow},
    base::{id, NO},
};
use log::trace;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tao::{
    dpi::{LogicalPosition, LogicalSize, PhysicalPosition, PhysicalSize},
    monitor::MonitorHandle,
    platform::macos::{WindowBuilderExtMacOS, WindowExtMacOS},
    window::{Theme, Window, WindowBuilder},
};

const MUTED_TITLE: &str = "Muted";
const UNMUTED_TITLE: &str = "Unmuted";
const FADE_DURATION_MS: u64 = 180;

pub type WindowSize<T = f64> = LogicalSize<T>;

fn get_mute_title_text(muted: bool) -> &'static str {
    if muted {
        MUTED_TITLE
    } else {
        UNMUTED_TITLE
    }
}

fn monitor_contains_physical_position(
    position: PhysicalPosition<f64>,
    monitor_position: PhysicalPosition<f64>,
    monitor_size: PhysicalSize<f64>,
) -> bool {
    position.x >= monitor_position.x
        && position.x < monitor_position.x + monitor_size.width
        && position.y >= monitor_position.y
        && position.y < monitor_position.y + monitor_size.height
}

fn setup_window(window: id) {
    unsafe {
        window.setHasShadow_(true);
        // Transparent window; the rounded bezel shape comes from the
        // vibrancy content view's masked layer.
        let clear: id = msg_send![class!(NSColor), clearColor];
        let _: () = msg_send![window, setOpaque: NO];
        let _: () = msg_send![window, setBackgroundColor: clear];
    };
}

pub struct Popup {
    window: Window,
    content: PopupContent,
    current_monitor: Option<MonitorHandle>,
    /// How long the popup bezel stays visible after a mute/unmute event.
    /// 0 = never show.
    popup_duration_ms: u64,
    /// Bumped on every show/hide. Pending `schedule_hide` timers capture the
    /// current value at spawn time and only fire `HidePopup` if it is still
    /// the same value, so stale timers from earlier shows become no-ops.
    generation: Arc<AtomicU64>,
    proxy: EventLoopProxyMessage,
    /// Tracks the last `mic_muted` value passed to `update`, so the 200 ms
    /// enforce-mute poll (which re-emits the same state) does not re-show the
    /// popup or reset the auto-hide timer.
    last_mic_muted: Option<bool>,
}

impl Popup {
    pub fn new(
        event_loop: &EventLoopMessage,
        mic_muted: bool,
        popup_duration_ms: u64,
    ) -> Result<Self> {
        let initial_monitor = Popup::get_initial_monitor(event_loop);
        let size = Popup::get_size();
        let scale = initial_monitor
            .as_ref()
            .map_or(1.0, MonitorHandle::scale_factor);
        let mut builder = WindowBuilder::new()
            .with_title(get_mute_title_text(mic_muted))
            .with_titlebar_hidden(true)
            .with_movable_by_window_background(true)
            .with_always_on_top(true)
            .with_closable(false)
            // Protected in release so the popup never appears in screen
            // shares; debug builds stay capturable for visual QA tooling.
            .with_content_protection(!cfg!(debug_assertions))
            .with_decorations(false)
            .with_inner_size(size)
            .with_maximized(false)
            .with_minimizable(false)
            .with_resizable(false)
            .with_visible_on_all_workspaces(true)
            .with_visible(false)
            .with_has_shadow(true);
        if let Some(monitor) = initial_monitor.as_ref() {
            builder = builder.with_position(Popup::get_position(monitor, size));
        }
        let window = builder
            .build(event_loop)
            .context("Failed to build window")?;
        window.set_visible(false);
        window.set_ignore_cursor_events(true)?;

        trace!("Window scale factor {}", scale);
        let content = PopupContent::new(mic_muted, size)?;
        unsafe {
            let ns_view = window.ns_view() as id;
            ns_view.addSubview_(content.view);
            let _: () = msg_send![content.view, release];
            let ns_window = window.ns_window() as id;
            setup_window(ns_window);
        };

        let popup = Self {
            window,
            content,
            current_monitor: initial_monitor,
            popup_duration_ms,
            generation: Arc::new(AtomicU64::new(0)),
            proxy: event_loop.create_proxy(),
            last_mic_muted: Some(mic_muted),
        };
        Ok(popup)
    }

    /// Bump the generation so any pending `schedule_hide` task becomes a no-op.
    fn invalidate_pending_hides(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
    }

    /// Spawn a deferred hide: after `popup_duration_ms`, send `Message::HidePopup`
    /// only if the generation hasn't moved on. Cheap no-op when duration is 0.
    fn schedule_hide(&self) {
        let duration_ms = self.popup_duration_ms;
        if duration_ms == 0 {
            return;
        }
        self.schedule_message(Message::HidePopup, duration_ms);
    }

    fn schedule_finalize_hide(&self) {
        self.schedule_message(Message::FinalizeHidePopup, FADE_DURATION_MS);
    }

    fn schedule_message(&self, msg: Message, delay_ms: u64) {
        let token = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let gen = self.generation.clone();
        let proxy = self.proxy.clone();
        trace!(
            "schedule_message {:?} token={} delay_ms={}",
            msg,
            token,
            delay_ms
        );
        task::spawn(async move {
            task::sleep(Duration::from_millis(delay_ms)).await;
            if gen
                .compare_exchange(token, token, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                let _ = proxy.send_event(msg);
            }
        });
    }

    fn get_size() -> WindowSize {
        LogicalSize::new(
            crate::popup_content::BEZEL_SIZE,
            crate::popup_content::BEZEL_SIZE,
        )
    }

    pub fn get_theme(&self) -> Theme {
        self.window.theme()
    }

    pub fn update(
        &mut self,
        mic_muted: bool,
        active_device_name: Option<&str>,
    ) -> Result<&mut Self> {
        self.window.set_title(get_mute_title_text(mic_muted));
        self.update_placement()?;
        self.content.update(mic_muted, active_device_name)?;
        let mic_changed = self.last_mic_muted != Some(mic_muted);
        self.last_mic_muted = Some(mic_muted);
        trace!(
            "popup.update mic_muted={} changed={} duration_ms={}",
            mic_muted,
            mic_changed,
            self.popup_duration_ms
        );

        if self.popup_duration_ms == 0 {
            self.invalidate_pending_hides();
            self.window.set_visible(false);
        } else if mic_changed {
            self.show_front();
            self.schedule_hide();
        }
        Ok(self)
    }

    /// Update the popup-duration setting at runtime.
    /// If switching to 0 while the popup is visible, also hide it immediately.
    pub fn set_popup_duration_ms(&mut self, popup_duration_ms: u64) {
        self.popup_duration_ms = popup_duration_ms;
        self.invalidate_pending_hides();
        if popup_duration_ms == 0 {
            self.window.set_visible(false);
        } else if self.window.is_visible() {
            self.schedule_hide();
        }
    }

    /// Trigger fade-out animation. Window stays visible at alpha=0 until
    /// `finalize_hide` runs (scheduled after `FADE_DURATION_MS`).
    pub fn hide(&mut self) -> Result<&mut Self> {
        self.invalidate_pending_hides();
        self.start_fade_out();
        self.schedule_finalize_hide();
        Ok(self)
    }

    /// Complete the hide: actually remove the window and reset alpha so the
    /// next `show_front` starts opaque.
    pub fn finalize_hide(&mut self) -> Result<&mut Self> {
        self.window.set_visible(false);
        unsafe {
            let ns_window = self.window.ns_window() as id;
            let _: () = msg_send![ns_window, setAlphaValue: 1.0_f64];
        }
        Ok(self)
    }

    pub fn update_placement(&mut self) -> Result<&mut Self> {
        if let Some(monitor) = self.get_current_monitor()? {
            let monitor_changed = self.current_monitor.as_ref() != Some(&monitor);
            let was_visible = monitor_changed && self.window.is_visible();
            if was_visible {
                self.window.set_visible(false);
            }

            let size = Popup::get_size();
            self.window.set_inner_size(size);
            self.window
                .set_outer_position(Popup::get_position(&monitor, size));
            self.current_monitor = Some(monitor);

            if was_visible {
                self.restore_visibility();
            }
        }
        Ok(self)
    }

    fn restore_visibility(&self) {
        self.window.set_visible(true);
        unsafe {
            let ns_window = self.window.ns_window() as id;
            let _: () = msg_send![ns_window, orderFrontRegardless];
        }
    }

    pub fn detect_cursor_monitor(&mut self) -> Result<&mut Self> {
        self.update_placement()
    }

    fn get_current_monitor(&self) -> Result<Option<MonitorHandle>> {
        // CoreGraphics and `Window::monitor_from_point` both use the same global
        // display coordinate space on macOS. Prefer this path over
        // `Window::cursor_position`, which converts through the primary display's
        // scale factor and can misclassify points near monitor boundaries.
        if let Some((x, y)) = get_cursor_pos() {
            if let Some(monitor) = self.window.monitor_from_point(x, y) {
                return Ok(Some(monitor));
            }
        }

        let position = self
            .window
            .cursor_position()
            .context("Failed to read cursor position")?;
        if let Some(monitor) = self.window.monitor_from_point(position.x, position.y) {
            return Ok(Some(monitor));
        }

        Ok(self.monitor_from_physical_position(position))
    }

    fn monitor_from_physical_position(
        &self,
        position: PhysicalPosition<f64>,
    ) -> Option<MonitorHandle> {
        self.window.available_monitors().find(|monitor| {
            monitor_contains_physical_position(
                position,
                monitor.position().cast::<f64>(),
                monitor.size().cast::<f64>(),
            )
        })
    }

    fn get_initial_monitor(event_loop: &EventLoopMessage) -> Option<MonitorHandle> {
        event_loop.primary_monitor()
    }

    fn show_front(&self) {
        self.invalidate_pending_hides();
        unsafe {
            let ns_window = self.window.ns_window() as id;
            // setAlphaValue on the window directly snaps to 1.0 and cancels
            // any in-flight fade animation from a prior hide().
            let _: () = msg_send![ns_window, setAlphaValue: 1.0_f64];
        }
        self.window.set_visible(true);
        unsafe {
            let ns_window = self.window.ns_window() as id;
            let _: () = msg_send![ns_window, orderFrontRegardless];
        }
    }

    fn start_fade_out(&self) {
        let secs = FADE_DURATION_MS as f64 / 1000.0;
        unsafe {
            let ns_window = self.window.ns_window() as id;
            let ctx_class = class!(NSAnimationContext);
            let _: () = msg_send![ctx_class, beginGrouping];
            let current: id = msg_send![ctx_class, currentContext];
            let _: () = msg_send![current, setDuration: secs];
            let animator: id = msg_send![ns_window, animator];
            let _: () = msg_send![animator, setAlphaValue: 0.0_f64];
            let _: () = msg_send![ctx_class, endGrouping];
        }
    }

    fn get_position(monitor: &MonitorHandle, window_size: WindowSize) -> LogicalPosition<f64> {
        // System-bezel placement: horizontally centered, a fixed gap above
        // the bottom edge (like the volume/brightness OSD).
        const BOTTOM_GAP: f64 = 140.0;
        let scale = monitor.scale_factor();
        let monitor_position = monitor.position().to_logical::<f64>(scale);
        let monitor_size = monitor.size().to_logical::<f64>(scale);
        let x: f64 = (monitor_position.x + (monitor_size.width / 2.)) - (window_size.width / 2.);
        let y: f64 = (monitor_position.y + monitor_size.height) - window_size.height - BOTTOM_GAP;
        LogicalPosition::new(x, y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monitor_contains_fractional_positions_on_negative_edges() {
        let monitor_position = PhysicalPosition::new(-1920.0, 0.0);
        let monitor_size = PhysicalSize::new(1920.0, 1080.0);

        assert!(monitor_contains_physical_position(
            PhysicalPosition::new(-0.25, 100.0),
            monitor_position,
            monitor_size
        ));
    }

    #[test]
    fn monitor_contains_positions_until_exclusive_far_edges() {
        let monitor_position = PhysicalPosition::new(0.0, 0.0);
        let monitor_size = PhysicalSize::new(1440.0, 900.0);

        assert!(monitor_contains_physical_position(
            PhysicalPosition::new(1439.999, 899.999),
            monitor_position,
            monitor_size
        ));
        assert!(!monitor_contains_physical_position(
            PhysicalPosition::new(1440.0, 899.999),
            monitor_position,
            monitor_size
        ));
        assert!(!monitor_contains_physical_position(
            PhysicalPosition::new(1439.999, 900.0),
            monitor_position,
            monitor_size
        ));
    }
}
