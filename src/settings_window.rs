/// Standalone preferences window. Ventura-style grouped form: one rounded
/// card (NSBox) with four hairline-separated rows, label left / control
/// right, following the system appearance. Auto-apply semantics — no
/// Save/Cancel; every control change persists + fires
/// `Message::ApplySettings`. Success is silent; a single footer line under
/// the card carries transient text (recording help, conflicts, save errors).
use crate::event_loop::{EventLoopMessage, EventLoopProxyMessage, Message};
use crate::settings::{Settings, ShortcutConfig, ThemePreference};
use crate::shortcuts::validate_shortcut;
use crate::utils::format_shortcut;

#[path = "shortcut_recorder.rs"]
mod shortcut_recorder;
use anyhow::{Context, Result};
use cocoa::base::{id, nil, NO, YES};
use cocoa::foundation::{NSPoint, NSRect, NSSize, NSString};
use log::trace;
use objc::declare::ClassDecl;
use objc::runtime::{Class, Object, Sel};
use std::cell::Cell;
use std::os::raw::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, RwLock};
use tao::dpi::LogicalSize;
use tao::platform::macos::WindowExtMacOS;
use tao::window::{Window, WindowBuilder, WindowId};

// AppKit constants (raw ObjC enums, not exposed by cocoa 0.24 helpers).
// NSTextAlignment follows the UIKit numbering on this AppKit version:
// 0=Left, 1=Center, 2=Right (verified empirically via the duration field).
const NS_TEXT_ALIGNMENT_CENTER: u64 = 1;
const NS_TEXT_ALIGNMENT_RIGHT: u64 = 2;
const NS_FONT_WEIGHT_MEDIUM: f64 = 0.23;
const NS_FONT_WEIGHT_REGULAR: f64 = 0.0;
const NS_BOX_CUSTOM: u64 = 4;

const WINDOW_SIZE: LogicalSize<f64> = LogicalSize::new(440.0, 220.0);
const ERROR_HOLD_SECS: f64 = 1.5;
const WARNING_HOLD_SECS: f64 = 3.0;
const RECORDING_PLACEHOLDER: &str = "Recording\u{2026}";
const MAX_POPUP_SECONDS: f64 = 60.0;

/// ivar payload for MMSettingsActions (leaked, app-lifetime valid).
struct ActionContext {
    proxy: EventLoopProxyMessage,
    settings: Arc<RwLock<Settings>>,
    launch_at_login_btn: id,
    popup_ms_field: id,
    popup_stepper: id,
    theme_popup: id,
    shortcut_chip: id,
    shortcut_label: id,
    record_btn: id,
    footer_label: id,
    escape_btn: id,
    settings_window: id,
    /// Pending footer auto-clear timer.
    footer_hide_timer: Cell<id>,
    /// `true` between `recordShortcutAction:` firing and recorder callback.
    is_recording: Cell<bool>,
}

pub struct SettingsWindow {
    window: Window,
    launch_at_login_btn: id,
    popup_ms_field: id,
    popup_stepper: id,
    theme_popup: id,
    shortcut_label: id,
    shortcut_chip: id,
    record_btn: id,
    footer_label: id,
    /// Invisible NSButton with Escape key equivalent — dispatches
    /// `Message::CloseSettings`. Native close paths (red button, Cmd-W)
    /// already route through tao's `WindowEvent::CloseRequested`.
    escape_btn: id,
    action_target: Option<id>,
    /// Visible flag, polled by the event-loop mtime path to skip live-reload
    /// while the user is editing.
    is_open: AtomicBool,
}

impl SettingsWindow {
    pub fn new(event_loop: &EventLoopMessage) -> Result<Self> {
        let window = WindowBuilder::new()
            .with_title("")
            .with_inner_size(WINDOW_SIZE)
            .with_resizable(false)
            .with_visible(false)
            .with_closable(true)
            .with_minimizable(false)
            .with_maximized(false)
            .build(event_loop)
            .context("Failed to build settings window")?;
        let b = unsafe { build_content_view(&window) };
        Ok(Self {
            window,
            launch_at_login_btn: b.launch_at_login_btn,
            popup_ms_field: b.popup_ms_field,
            popup_stepper: b.popup_stepper,
            theme_popup: b.theme_popup,
            shortcut_label: b.shortcut_label,
            shortcut_chip: b.shortcut_chip,
            record_btn: b.record_btn,
            footer_label: b.footer_label,
            escape_btn: b.escape_btn,
            action_target: None,
            is_open: AtomicBool::new(false),
        })
    }

    /// Late-binds per-control selectors. `ActionContext` is leaked
    /// (app-lifetime); selectors fire on the main thread, so single-threaded
    /// mutation through the raw pointer is sound.
    pub fn bind_actions(&mut self, settings: Arc<RwLock<Settings>>, proxy: EventLoopProxyMessage) {
        debug_assert!(self.action_target.is_none(), "bind_actions called twice");
        if self.action_target.is_some() {
            return;
        }
        let ns_window: id = self.window.ns_window() as id;
        let ctx_ptr = Box::into_raw(Box::new(ActionContext {
            proxy,
            settings,
            launch_at_login_btn: self.launch_at_login_btn,
            popup_ms_field: self.popup_ms_field,
            popup_stepper: self.popup_stepper,
            theme_popup: self.theme_popup,
            shortcut_chip: self.shortcut_chip,
            shortcut_label: self.shortcut_label,
            record_btn: self.record_btn,
            footer_label: self.footer_label,
            escape_btn: self.escape_btn,
            settings_window: ns_window,
            footer_hide_timer: Cell::new(nil),
            is_recording: Cell::new(false),
        })) as *mut c_void;
        let target: id = unsafe {
            let obj: id = msg_send![actions_class(), alloc];
            let obj: id = msg_send![obj, init];
            (*obj).set_ivar("_ctxPtr", ctx_ptr);
            obj
        };
        unsafe {
            let _: () = msg_send![self.launch_at_login_btn, setTarget: target];
            let _: () = msg_send![self.launch_at_login_btn, setAction: sel!(launchAtLoginToggled:)];
            let _: () = msg_send![self.popup_stepper, setTarget: target];
            let _: () = msg_send![self.popup_stepper, setAction: sel!(stepperChanged:)];
            let _: () = msg_send![self.theme_popup, setTarget: target];
            let _: () = msg_send![self.theme_popup, setAction: sel!(themeChanged:)];
            let _: () = msg_send![self.escape_btn, setTarget: target];
            let _: () = msg_send![self.escape_btn, setAction: sel!(closeSettings:)];
            let _: () = msg_send![self.record_btn, setTarget: target];
            let _: () = msg_send![self.record_btn, setAction: sel!(recordShortcutAction:)];
        }
        self.action_target = Some(target);
    }

    pub fn id(&self) -> WindowId {
        self.window.id()
    }
    pub fn is_open(&self) -> bool {
        self.is_open.load(Ordering::SeqCst)
    }

    pub fn open(&self, settings: &Settings) {
        trace!("Opening settings window");
        self.refresh_from(settings);
        self.window.set_visible(true);
        unsafe {
            let ns_window = self.window.ns_window() as id;
            let _: () = msg_send![ns_window, orderFrontRegardless];
            let _: () = msg_send![ns_window, makeKeyWindow];
            animate_appear(ns_window);
        }
        self.is_open.store(true, Ordering::SeqCst);
    }

    pub fn close(&self) {
        trace!("Closing settings window");
        // Tear down any in-flight recorder so the window doesn't reopen with a
        // phantom recording state (orphaned recorder view swallowing clicks,
        // escape_btn disabled). Fires handle_capture(Cancelled), which resets
        // is_recording and the chip visuals. No-op when not recording.
        shortcut_recorder::cancel_recording();
        self.window.set_visible(false);
        self.is_open.store(false, Ordering::SeqCst);
    }

    /// Push current `Settings` values into every visible control.
    pub fn refresh_from(&self, settings: &Settings) {
        unsafe {
            let _: () =
                msg_send![self.launch_at_login_btn, setState: settings.launch_at_login as i64];
            let seconds = settings.popup_duration_ms as f64 / 1000.0;
            let _: () = msg_send![self.popup_stepper, setDoubleValue: seconds];
            set_string(
                self.popup_ms_field,
                &format_seconds(settings.popup_duration_ms),
            );
            let _: () = msg_send![self.theme_popup, selectItemAtIndex: theme_index(settings.theme)];
            // Leave the chip alone while a recording is in flight: a debounced
            // popup-duration commit can land here via ApplySettings and would
            // otherwise replace the "Recording…" placeholder mid-capture.
            let recording = self
                .action_target
                .map(|t| ctx_from(&*t).is_recording.get())
                .unwrap_or(false);
            if !recording {
                set_string(
                    self.shortcut_label,
                    &format_shortcut(&settings.mic_shortcut),
                );
                resize_chip_to_fit(self.shortcut_chip, self.shortcut_label, self.record_btn);
            }
        }
    }

    /// Visual QA hook for the settings-preview sidecar. Drives the window
    /// into a named non-default state so a single screencapture captures the
    /// recording / warning / save-error appearance without a human click.
    /// No-op for unknown labels and when called before `bind_actions`.
    /// Debug-only: the sidecar builds the debug profile; release app binaries
    /// don't carry the QA scaffolding.
    #[cfg(debug_assertions)]
    #[allow(dead_code)]
    pub fn preview_state(&self, state: &str) {
        let Some(target) = self.action_target else {
            return;
        };
        unsafe {
            let this: &Object = &*target;
            let ctx = ctx_from(this);
            match state {
                "default" => {}
                "recording" => {
                    let _: () = msg_send![target,
                        performSelector: sel!(recordShortcutAction:) withObject: nil];
                    // Snapshots are taken without spinning the run loop, so
                    // animator-driven alphas may not have flushed. Force the
                    // helper text visible for a deterministic capture.
                    let _: () = msg_send![ctx.footer_label, setAlphaValue: 1.0_f64];
                }
                "warning" => {
                    show_warning(this, ctx, "\u{26A0} Already in use");
                    let _: () = msg_send![ctx.footer_label, setAlphaValue: 1.0_f64];
                }
                "status_err" => {
                    show_save_error(this, ctx);
                    let _: () = msg_send![ctx.footer_label, setAlphaValue: 1.0_f64];
                }
                _ => {}
            }
        }
    }

    /// Force the window appearance for sidecar captures
    /// (`NSAppearanceNameAqua` / `NSAppearanceNameDarkAqua`). The shipped app
    /// never calls this — it follows the system appearance.
    #[cfg(debug_assertions)]
    #[allow(dead_code)]
    pub fn set_preview_appearance(&self, name: &str) {
        unsafe {
            let ns_window = self.window.ns_window() as id;
            let ns_name = NSString::alloc(nil).init_str(name);
            let appearance: id = msg_send![class!(NSAppearance), appearanceNamed: ns_name];
            let _: () = msg_send![ns_name, release];
            if appearance != nil {
                let _: () = msg_send![ns_window, setAppearance: appearance];
            }
        }
    }

    /// In-process snapshot of the entire window (titlebar + content) to a PNG
    /// at `path`. Renders via `bitmapImageRepForCachingDisplayInRect:` +
    /// `cacheDisplayInRect:` on the window's root theme frame view, so it
    /// does NOT need Screen Recording permission and does NOT involve the
    /// window server compositor. Captures focus rings, layer-backed views,
    /// title bar, and box fills correctly.
    /// Debug-only, same rationale as `preview_state`.
    #[cfg(debug_assertions)]
    #[allow(dead_code)]
    pub fn snapshot_to_png(&self, path: &str) -> Result<()> {
        unsafe {
            let ns_window = self.window.ns_window() as id;
            let content_view: id = msg_send![ns_window, contentView];
            if content_view == nil {
                return Err(anyhow::anyhow!("no content view"));
            }
            // The contentView's superview is the window's themeFrame, the root
            // of the window's view hierarchy; it includes titlebar background.
            let root_view: id = msg_send![content_view, superview];
            let target = if root_view == nil {
                content_view
            } else {
                root_view
            };
            let bounds: NSRect = msg_send![target, bounds];
            let rep: id = msg_send![target, bitmapImageRepForCachingDisplayInRect: bounds];
            if rep == nil {
                return Err(anyhow::anyhow!(
                    "bitmapImageRepForCachingDisplayInRect returned nil"
                ));
            }
            let _: () = msg_send![target, cacheDisplayInRect: bounds toBitmapImageRep: rep];
            // NSBitmapImageFileTypePNG = 4
            let png_data: id = msg_send![rep, representationUsingType: 4_u64 properties: nil as id];
            if png_data == nil {
                return Err(anyhow::anyhow!("representationUsingType:PNG returned nil"));
            }
            let ns_path = NSString::alloc(nil).init_str(path);
            let ok: bool = msg_send![png_data, writeToFile: ns_path atomically: YES];
            let _: () = msg_send![ns_path, release];
            if !ok {
                return Err(anyhow::anyhow!("writeToFile failed for {}", path));
            }
        }
        Ok(())
    }
}

unsafe fn set_string(field: id, text: &str) {
    let s = NSString::alloc(nil).init_str(text);
    let _: () = msg_send![field, setStringValue: s];
    let _: () = msg_send![s, release];
}

// UI mutations only happen on the main thread inside the event-loop closure.
unsafe impl Send for SettingsWindow {}
unsafe impl Sync for SettingsWindow {}

extern "C" fn launch_at_login_toggled(this: &Object, _cmd: Sel, _sender: *mut Object) {
    let ctx = unsafe { ctx_from(this) };
    let state: i64 = unsafe { msg_send![ctx.launch_at_login_btn, state] };
    persist(this, ctx, None, |s| s.launch_at_login = state == 1);
}

extern "C" fn stepper_changed(this: &Object, _cmd: Sel, _sender: *mut Object) {
    let ctx = unsafe { ctx_from(this) };
    let value: f64 = unsafe { msg_send![ctx.popup_stepper, doubleValue] };
    let ms = quantize_seconds_to_ms(value);
    unsafe { set_string(ctx.popup_ms_field, &format_seconds(ms)) };
    persist(this, ctx, None, |store| store.popup_duration_ms = ms);
}

extern "C" fn theme_changed(this: &Object, _cmd: Sel, _sender: *mut Object) {
    let ctx = unsafe { ctx_from(this) };
    let index: i64 = unsafe { msg_send![ctx.theme_popup, indexOfSelectedItem] };
    let theme = theme_from_index(index);
    persist(this, ctx, None, |s| s.theme = theme);
}

/// Popup-menu item order; must match `make_theme_popup`'s titles.
fn theme_index(theme: ThemePreference) -> i64 {
    match theme {
        ThemePreference::System => 0,
        ThemePreference::Light => 1,
        ThemePreference::Dark => 2,
    }
}

fn theme_from_index(index: i64) -> ThemePreference {
    match index {
        1 => ThemePreference::Light,
        2 => ThemePreference::Dark,
        _ => ThemePreference::System,
    }
}

/// Snap a seconds value to the 0.1s grid the UI edits in.
fn quantize_seconds_to_ms(seconds: f64) -> u64 {
    ((seconds * 10.0).round() as u64) * 100
}

/// Format a millisecond duration as a seconds string (e.g. `1000` → `"1.0"`,
/// `1500` → `"1.5"`, `0` → `"0"`). Values finer than 0.1s keep their full
/// precision (`1250` → `"1.25"`) so display → parse round-trips losslessly.
fn format_seconds(ms: u64) -> String {
    if ms == 0 {
        return "0".to_string();
    }
    let s = ms as f64 / 1000.0;
    if ms.is_multiple_of(100) {
        format!("{:.1}", s)
    } else {
        format!("{}", s)
    }
}

#[cfg(test)]
mod tests {
    use super::{format_seconds, quantize_seconds_to_ms, theme_from_index, theme_index};
    use crate::settings::ThemePreference;

    #[test]
    fn test_theme_index_round_trips() {
        for theme in [
            ThemePreference::System,
            ThemePreference::Light,
            ThemePreference::Dark,
        ] {
            assert_eq!(theme_from_index(theme_index(theme)), theme);
        }
    }

    #[test]
    fn test_theme_from_unknown_index_falls_back_to_system() {
        assert_eq!(theme_from_index(-1), ThemePreference::System);
        assert_eq!(theme_from_index(99), ThemePreference::System);
    }

    #[test]
    fn test_format_seconds_round_trips_ms_values() {
        for ms in [0u64, 100, 1000, 1250, 1500, 12345, 86_400_000] {
            let displayed = format_seconds(ms);
            let parsed = displayed.parse::<f64>().unwrap();
            assert_eq!((parsed * 1000.0).round() as u64, ms, "via {:?}", displayed);
        }
    }

    #[test]
    fn test_format_seconds_display() {
        assert_eq!(format_seconds(0), "0");
        assert_eq!(format_seconds(1000), "1.0");
        assert_eq!(format_seconds(1500), "1.5");
        assert_eq!(format_seconds(1250), "1.25");
    }

    #[test]
    fn test_quantize_seconds_snaps_to_tenths() {
        assert_eq!(quantize_seconds_to_ms(0.0), 0);
        assert_eq!(quantize_seconds_to_ms(1.0), 1000);
        assert_eq!(quantize_seconds_to_ms(1.25), 1300);
        assert_eq!(quantize_seconds_to_ms(0.30000000000000004), 300);
    }
}

fn persist(
    this: &Object,
    ctx: &ActionContext,
    previous_shortcut: Option<ShortcutConfig>,
    mutate: impl FnOnce(&mut Settings),
) {
    let ok = {
        let mut s = ctx.settings.write().unwrap();
        mutate(&mut s);
        match s.save() {
            Ok(_) => true,
            Err(e) => {
                log::error!("Failed to save settings: {}", e);
                false
            }
        }
    };
    if ok {
        let _ = ctx
            .proxy
            .send_event(Message::ApplySettings { previous_shortcut });
    } else {
        // Auto-apply success is silent (the control's new state is the
        // feedback); only failures surface.
        show_save_error(this, ctx);
    }
}

fn show_save_error(this: &Object, ctx: &ActionContext) {
    let color = unsafe { error_color() };
    show_footer(this, ctx, "\u{2717} Could not save", color, ERROR_HOLD_SECS);
}

/// Show transient text in the footer line under the card, auto-hiding after
/// `hold_secs`. Fade in 80ms, out 240ms (NSAnimationContext + NSTimer-driven
/// fade-out so the next message can pre-empt the prior one).
fn show_footer(this: &Object, ctx: &ActionContext, text: &str, color: id, hold_secs: f64) {
    unsafe {
        set_string(ctx.footer_label, text);
        let _: () = msg_send![ctx.footer_label, setTextColor: color];
        animate_single_alpha(ctx.footer_label, 1.0, 0.08);

        let timer: id = msg_send![
            class!(NSTimer),
            scheduledTimerWithTimeInterval: hold_secs
            target: this selector: sel!(footerHide:)
            userInfo: nil repeats: NO
        ];
        let _: () = msg_send![timer, retain];
        let prev = ctx.footer_hide_timer.replace(timer);
        if prev != nil {
            let _: () = msg_send![prev, invalidate];
            let _: () = msg_send![prev, release];
        }
    }
}

unsafe fn animate_single_alpha(view: id, alpha: f64, secs: f64) {
    let ctx_cls = class!(NSAnimationContext);
    let _: () = msg_send![ctx_cls, beginGrouping];
    let current: id = msg_send![ctx_cls, currentContext];
    let _: () = msg_send![current, setDuration: secs];
    let animator: id = msg_send![view, animator];
    let _: () = msg_send![animator, setAlphaValue: alpha];
    let _: () = msg_send![ctx_cls, endGrouping];
}

fn enter_recording_visual(ctx: &ActionContext) {
    unsafe {
        set_chip_border(ctx, accent_color(), 2.0);
        set_string(ctx.shortcut_label, RECORDING_PLACEHOLDER);
        resize_chip_to_fit_ctx(ctx);
        set_string(ctx.footer_label, "Press a key combination. Esc to cancel.");
        let _: () = msg_send![ctx.footer_label, setTextColor: tertiary_label_color()];
        animate_single_alpha(ctx.footer_label, 1.0, 0.10);
    }
}

fn exit_recording_visual(ctx: &ActionContext) {
    unsafe {
        reset_chip_border(ctx);
        // Fade the recording helper text out. show_warning's red copy will
        // override this if a conflict triggered the exit.
        animate_single_alpha(ctx.footer_label, 0.0, 0.10);
    }
}

fn show_warning(this: &Object, ctx: &ActionContext, text: &str) {
    unsafe {
        // Red chip border binds the warning text visually to the offending field.
        set_chip_border(ctx, error_color(), 1.5);
        let color = error_color();
        show_footer(this, ctx, text, color, WARNING_HOLD_SECS);
    }
}

fn clear_footer(ctx: &ActionContext) {
    unsafe {
        let prev = ctx.footer_hide_timer.replace(nil);
        if prev != nil {
            let _: () = msg_send![prev, invalidate];
            let _: () = msg_send![prev, release];
        }
        animate_single_alpha(ctx.footer_label, 0.0, 0.12);
        reset_chip_border(ctx);
    }
}

/// NSBox border colors are NSColor-backed, so dynamic system colors adapt to
/// appearance changes automatically (unlike raw CALayer CGColors).
unsafe fn set_chip_border(ctx: &ActionContext, color: id, width: f64) {
    let _: () = msg_send![ctx.shortcut_chip, setBorderColor: color];
    let _: () = msg_send![ctx.shortcut_chip, setBorderWidth: width];
}

fn reset_chip_border(ctx: &ActionContext) {
    unsafe {
        set_chip_border(ctx, separator_color(), 1.0);
    }
}

unsafe fn ctx_from(this: &Object) -> &ActionContext {
    let ptr: *mut c_void = *this.get_ivar("_ctxPtr");
    &*(ptr as *const ActionContext)
}

fn actions_class() -> &'static Class {
    static CLS: OnceLock<&'static Class> = OnceLock::new();
    CLS.get_or_init(|| {
        let mut decl = ClassDecl::new("MMSettingsActions", class!(NSObject))
            .expect("MMSettingsActions registered twice");
        decl.add_ivar::<*mut c_void>("_ctxPtr");
        unsafe {
            decl.add_method(
                sel!(launchAtLoginToggled:),
                launch_at_login_toggled as extern "C" fn(&Object, Sel, *mut Object),
            );
            decl.add_method(
                sel!(stepperChanged:),
                stepper_changed as extern "C" fn(&Object, Sel, *mut Object),
            );
            decl.add_method(
                sel!(themeChanged:),
                theme_changed as extern "C" fn(&Object, Sel, *mut Object),
            );
            decl.add_method(
                sel!(closeSettings:),
                close_settings as extern "C" fn(&Object, Sel, *mut Object),
            );
            decl.add_method(
                sel!(recordShortcutAction:),
                record_shortcut_action as extern "C" fn(&Object, Sel, *mut Object),
            );
            decl.add_method(
                sel!(footerHide:),
                footer_hide as extern "C" fn(&Object, Sel, *mut Object),
            );
        }
        decl.register()
    })
}

extern "C" fn close_settings(this: &Object, _cmd: Sel, _sender: *mut Object) {
    let ctx = unsafe { ctx_from(this) };
    let _ = ctx.proxy.send_event(Message::CloseSettings);
}

extern "C" fn footer_hide(this: &Object, _cmd: Sel, _timer: *mut Object) {
    let ctx = unsafe { ctx_from(this) };
    let prev = ctx.footer_hide_timer.replace(nil);
    if prev != nil {
        unsafe {
            let _: () = msg_send![prev, release];
        }
    }
    unsafe { animate_single_alpha(ctx.footer_label, 0.0, 0.24) };
    reset_chip_border(ctx);
}

/// Edit-button selector. Cancels any in-flight recorder, clears any visible
/// warning, then arms a new recorder over the chip. The capture callback
/// runs on the main thread (NSView keyDown delivery) so it can mutate UI and
/// settings without crossing thread boundaries.
extern "C" fn record_shortcut_action(this: &Object, _cmd: Sel, _sender: *mut Object) {
    let ctx = unsafe { ctx_from(this) };
    if ctx.is_recording.get() {
        // Second click while recording → cancel.
        shortcut_recorder::cancel_recording();
        return;
    }
    clear_footer(ctx);
    enter_recording_visual(ctx);
    ctx.is_recording.set(true);
    // The registered hotkey would consume its own combo before the recorder
    // sees the keyDown, toggling mute instead of re-capturing it. Suspend it
    // for the duration of the recording.
    let _ = ctx
        .proxy
        .send_event(Message::SuspendHotkey { suspended: true });
    // Disable the invisible Escape button while recording so Escape reaches
    // the recorder view's cancelOperation: instead of closing the window.
    unsafe {
        let _: () = msg_send![ctx.escape_btn, setEnabled: NO];
    }

    let this_ptr = this as *const Object as id;
    let window = ctx.settings_window;
    // The chip lives inside the card box; the recorder view is installed on
    // the window's content view, so convert the chip frame across spaces.
    let chip_frame: NSRect = unsafe {
        let content_view: id = msg_send![window, contentView];
        let bounds: NSRect = msg_send![ctx.shortcut_chip, bounds];
        msg_send![ctx.shortcut_chip, convertRect: bounds toView: content_view]
    };
    shortcut_recorder::start_recording(
        window,
        chip_frame,
        Box::new(move |result| {
            // Recorder callback runs on the main thread; ctx is leaked (app-lifetime)
            // and `this_ptr` points to the leaked MMSettingsActions instance.
            unsafe {
                let obj: &Object = &*this_ptr;
                handle_capture(obj, result);
            }
        }),
    );
}

fn handle_capture(this: &Object, result: shortcut_recorder::CaptureResult) {
    let ctx = unsafe { ctx_from(this) };
    ctx.is_recording.set(false);
    // Queued before any ApplySettings from persist below, so the event loop
    // re-registers the old combo first, then reload swaps in the new one.
    let _ = ctx
        .proxy
        .send_event(Message::SuspendHotkey { suspended: false });
    exit_recording_visual(ctx);
    unsafe {
        let _: () = msg_send![ctx.escape_btn, setEnabled: YES];
    }

    let new_combo = match result {
        shortcut_recorder::CaptureResult::Captured(c) => c,
        shortcut_recorder::CaptureResult::Cancelled => {
            // Escape / cancel — revert chip to current settings, no warning.
            refresh_chip_from_settings(ctx);
            return;
        }
        shortcut_recorder::CaptureResult::MissingModifier => {
            refresh_chip_from_settings(ctx);
            show_warning(this, ctx, "\u{26A0} Needs a modifier key");
            return;
        }
    };

    // No-op if user re-selected the same shortcut.
    let current = ctx.settings.read().unwrap().mic_shortcut.clone();
    if shortcuts_equal(&current, &new_combo) {
        refresh_chip_from_settings(ctx);
        return;
    }

    match validate_shortcut(&new_combo) {
        Ok(()) => {
            let previous = current.clone();
            persist(this, ctx, Some(previous), |s| {
                s.mic_shortcut = new_combo.clone()
            });
            // persist already refreshed via ApplySettings → apply_settings
            // → refresh_from, but that path only runs on the next event-loop
            // turn. Update the chip immediately so the user sees the new value.
            unsafe {
                set_string(ctx.shortcut_label, &format_shortcut(&new_combo));
                resize_chip_to_fit_ctx(ctx);
            }
        }
        Err(conflict) => {
            refresh_chip_from_settings(ctx);
            show_warning(this, ctx, &conflict.to_string());
        }
    }
}

fn shortcuts_equal(a: &ShortcutConfig, b: &ShortcutConfig) -> bool {
    if a.key.to_uppercase() != b.key.to_uppercase() {
        return false;
    }
    let normalize = |list: &[String]| {
        let mut v: Vec<String> = list
            .iter()
            .map(|m| match m.as_str() {
                "cmd" | "command" => "meta".to_string(),
                "control" => "ctrl".to_string(),
                "option" => "alt".to_string(),
                other => other.to_string(),
            })
            .collect();
        v.sort();
        v.dedup();
        v
    };
    normalize(&a.modifiers) == normalize(&b.modifiers)
}

fn refresh_chip_from_settings(ctx: &ActionContext) {
    let text = format_shortcut(&ctx.settings.read().unwrap().mic_shortcut);
    unsafe {
        set_string(ctx.shortcut_label, &text);
        resize_chip_to_fit_ctx(ctx);
    }
}

unsafe fn resize_chip_to_fit_ctx(ctx: &ActionContext) {
    resize_chip_to_fit(ctx.shortcut_chip, ctx.shortcut_label, ctx.record_btn);
}

struct BuiltViews {
    launch_at_login_btn: id,
    popup_ms_field: id,
    popup_stepper: id,
    theme_popup: id,
    shortcut_label: id,
    shortcut_chip: id,
    record_btn: id,
    footer_label: id,
    escape_btn: id,
}

/// Form layout constants (logical points). Change a single value here to
/// reshape the entire window; no inline magic numbers in build_content_view.
const SIDE_PAD: f64 = 20.0;
const TOP_PAD: f64 = 18.0;
/// Height of one grouped-card row.
const ROW_H: f64 = 40.0;
const ROW_COUNT: f64 = 4.0;
const CARD_W: f64 = WINDOW_SIZE.width - 2.0 * SIDE_PAD;
const CARD_H: f64 = ROW_H * ROW_COUNT;
const CARD_RADIUS: f64 = 10.0;
/// Horizontal inset of row content (labels, separators) inside the card.
const ROW_INSET: f64 = 16.0;
/// Trailing inset of controls inside the card.
const CONTROL_INSET: f64 = 12.0;
const CHIP_MIN_W: f64 = 76.0;
const CHIP_H: f64 = 24.0;
const FIELD_W: f64 = 56.0;
const FIELD_H: f64 = 22.0;
const STEPPER_W: f64 = 19.0;
const STEPPER_H: f64 = 27.0;
const FOOTER_H: f64 = 16.0;
const FOOTER_BOTTOM: f64 = 12.0;

unsafe fn build_content_view(window: &Window) -> BuiltViews {
    apply_window_chrome(window.ns_window() as id);
    let cv = window.ns_view() as id;
    let h = WINDOW_SIZE.height;

    // Grouped card: an NSBox so fill/border use dynamic NSColors that follow
    // the system appearance (CALayer CGColors would freeze at set-time —
    // that limitation is what forced the old dark-appearance lock).
    let card = make_card(SIDE_PAD, h - TOP_PAD - CARD_H, CARD_W, CARD_H);
    add(cv, card);
    let card_cv: id = msg_send![card, contentView];

    // Rows inside the card, top to bottom. Card content coords are bottom-up:
    // row index 0 (top) starts at CARD_H - ROW_H.
    let row_bottom = |index: f64| CARD_H - ROW_H * (index + 1.0);

    // Hairline separators between rows.
    for i in 0..3 {
        let sep = make_separator(ROW_INSET, row_bottom(i as f64), CARD_W - ROW_INSET);
        add(card_cv, sep);
    }

    // row 1: Mute shortcut
    place_row_label(card_cv, "Mute shortcut", row_bottom(0.0));
    let chip_y = vcenter_in_row(row_bottom(0.0), CHIP_H);
    let chip_x = CARD_W - CONTROL_INSET - CHIP_MIN_W;
    let shortcut_label = make_chip_label();
    let shortcut_chip = make_chip_box(chip_x, chip_y, CHIP_MIN_W, CHIP_H);
    let chip_cv: id = msg_send![shortcut_chip, contentView];
    let _: () = msg_send![chip_cv, addSubview: shortcut_label];
    add(card_cv, shortcut_chip);
    let record_btn = make_invisible_button("Click to record a new shortcut");
    add(
        card_cv,
        place(record_btn, chip_x, chip_y, CHIP_MIN_W, CHIP_H),
    );

    // row 2: Launch at login
    place_row_label(card_cv, "Launch at login", row_bottom(1.0));
    // NSSwitch draws at its natural size centered in whatever frame it gets,
    // so a hardcoded frame width misaligns its visual right edge. Size to
    // fit and pin the fitted frame's right edge to the control column.
    let launch_at_login_btn = make_switch();
    let _: () = msg_send![launch_at_login_btn, sizeToFit];
    let sw: NSRect = msg_send![launch_at_login_btn, frame];
    add(
        card_cv,
        place(
            launch_at_login_btn,
            CARD_W - CONTROL_INSET - sw.size.width,
            vcenter_in_row(row_bottom(1.0), sw.size.height),
            sw.size.width,
            sw.size.height,
        ),
    );

    // row 3: Popup duration — [field][stepper] s
    place_row_label(card_cv, "Popup duration", row_bottom(2.0));
    let unit = make_unit_label("s");
    let _: () = msg_send![unit, sizeToFit];
    let unit_frame: NSRect = msg_send![unit, frame];
    let unit_w = unit_frame.size.width;
    let unit_x = CARD_W - CONTROL_INSET - unit_w;
    place_intrinsic(
        card_cv,
        unit,
        unit_x,
        vcenter_in_row(row_bottom(2.0), unit_frame.size.height),
    );
    let popup_stepper = make_stepper();
    let stepper_x = unit_x - 6.0 - STEPPER_W;
    add(
        card_cv,
        place(
            popup_stepper,
            stepper_x,
            vcenter_in_row(row_bottom(2.0), STEPPER_H),
            STEPPER_W,
            STEPPER_H,
        ),
    );
    let popup_ms_field = make_duration_field();
    add(
        card_cv,
        place(
            popup_ms_field,
            stepper_x - 4.0 - FIELD_W,
            vcenter_in_row(row_bottom(2.0), FIELD_H),
            FIELD_W,
            FIELD_H,
        ),
    );

    // row 4: Appearance — System / Light / Dark popup button
    place_row_label(card_cv, "Appearance", row_bottom(3.0));
    let theme_popup = make_theme_popup();
    let _: () = msg_send![theme_popup, sizeToFit];
    let pf: NSRect = msg_send![theme_popup, frame];
    add(
        card_cv,
        place(
            theme_popup,
            CARD_W - CONTROL_INSET - pf.size.width,
            vcenter_in_row(row_bottom(3.0), pf.size.height),
            pf.size.width,
            pf.size.height,
        ),
    );

    // Footer: one caption line under the card for transient text (recording
    // help, conflicts, save errors). Invisible when idle.
    let footer_label = make_footer_label();
    add(
        cv,
        place(
            footer_label,
            SIDE_PAD + ROW_INSET,
            FOOTER_BOTTOM,
            WINDOW_SIZE.width - 2.0 * (SIDE_PAD + ROW_INSET),
            FOOTER_H,
        ),
    );

    let escape_btn = make_escape_button();
    add(cv, place(escape_btn, -10.0, -10.0, 1.0, 1.0));

    BuiltViews {
        launch_at_login_btn,
        popup_ms_field,
        popup_stepper,
        theme_popup,
        shortcut_label,
        shortcut_chip,
        record_btn,
        footer_label,
        escape_btn,
    }
}

/// Create + position a row label, left-aligned at ROW_INSET and vertically
/// centered in the row.
unsafe fn place_row_label(parent: id, text: &str, row_bottom: f64) {
    let label = make_plain_label(system_font(13.0, NS_FONT_WEIGHT_REGULAR), label_color());
    set_string(label, text);
    let _: () = msg_send![label, sizeToFit];
    let lf: NSRect = msg_send![label, frame];
    place_intrinsic(
        parent,
        label,
        ROW_INSET,
        vcenter_in_row(row_bottom, lf.size.height),
    );
}

unsafe fn place_intrinsic(parent: id, view: id, x: f64, y: f64) {
    let f: NSRect = msg_send![view, frame];
    set_frame(view, x, y, f.size.width, f.size.height);
    add(parent, view);
}

/// Vertically center a `content_h`-tall element inside a `ROW_H`-tall row
/// starting at `row_bottom`. Floored to integer pixels for crisp rendering.
fn vcenter_in_row(row_bottom: f64, content_h: f64) -> f64 {
    row_bottom + ((ROW_H - content_h) / 2.0).floor()
}

unsafe fn place(view: id, x: f64, y: f64, w: f64, h: f64) -> id {
    set_frame(view, x, y, w, h);
    view
}
unsafe fn add(parent: id, child: id) {
    let _: () = msg_send![parent, addSubview: child];
}

unsafe fn make_escape_button() -> id {
    let btn: id = msg_send![class!(NSButton), alloc];
    let btn: id = msg_send![btn, init];
    let _: () = msg_send![btn, setBordered: NO];
    let esc = NSString::alloc(nil).init_str("\u{1b}");
    let _: () = msg_send![btn, setKeyEquivalent: esc];
    let _: () = msg_send![esc, release];
    btn
}

unsafe fn apply_window_chrome(ns_window: id) {
    // Standard macOS prefs window: visible titlebar with "Settings" title,
    // following the system appearance.
    let title = NSString::alloc(nil).init_str("Settings");
    let _: () = msg_send![ns_window, setTitle: title];
    let _: () = msg_send![title, release];
    let _: () = msg_send![ns_window, setTitleVisibility: 0i64]; // NSWindowTitleVisible
    let _: () = msg_send![ns_window, setTitlebarAppearsTransparent: NO];

    // Hide the disabled-looking minimize/zoom traffic lights (window chrome
    // is non-resizable + non-minimizable, so showing them greyed reads as
    // broken). Leave the red close button intact.
    let miniaturize_btn: id = msg_send![ns_window, standardWindowButton: 1u64]; // NSWindowMiniaturizeButton
    let zoom_btn: id = msg_send![ns_window, standardWindowButton: 2u64]; // NSWindowZoomButton
    if miniaturize_btn != nil {
        let _: () = msg_send![miniaturize_btn, setHidden: YES];
    }
    if zoom_btn != nil {
        let _: () = msg_send![zoom_btn, setHidden: YES];
    }
}

/// The grouped form card: custom NSBox with rounded corners, hairline border
/// and a content-background fill. Dynamic NSColors keep it appearance-correct.
unsafe fn make_card(x: f64, y: f64, w: f64, h: f64) -> id {
    let card: id = msg_send![class!(NSBox), alloc];
    let card: id = msg_send![card, init];
    let _: () = msg_send![card, setBoxType: NS_BOX_CUSTOM];
    let _: () = msg_send![card, setTitlePosition: 0u64]; // NSNoTitle
    let _: () = msg_send![card, setContentViewMargins: NSSize::new(0.0, 0.0)];
    let _: () = msg_send![card, setCornerRadius: CARD_RADIUS];
    let _: () = msg_send![card, setBorderWidth: 1.0_f64];
    let _: () = msg_send![card, setBorderColor: separator_color()];
    let fill: id = msg_send![class!(NSColor), controlBackgroundColor];
    let _: () = msg_send![card, setFillColor: fill];
    set_frame(card, x, y, w, h);
    card
}

/// 1pt hairline. Custom fill box, not NSBoxSeparator: the separator box
/// type draws its line off-center in the frame, drifting ~1.5pt off the
/// row grid.
unsafe fn make_separator(x: f64, y: f64, w: f64) -> id {
    let sep: id = msg_send![class!(NSBox), alloc];
    let sep: id = msg_send![sep, init];
    let _: () = msg_send![sep, setBoxType: NS_BOX_CUSTOM];
    let _: () = msg_send![sep, setTitlePosition: 0u64];
    let _: () = msg_send![sep, setBorderWidth: 0.0_f64];
    let _: () = msg_send![sep, setFillColor: separator_color()];
    set_frame(sep, x, y, w, 1.0);
    sep
}

unsafe fn make_unit_label(text: &str) -> id {
    let label = make_plain_label(
        system_font(13.0, NS_FONT_WEIGHT_REGULAR),
        secondary_label_color(),
    );
    set_string(label, text);
    label
}

unsafe fn make_plain_label(font: id, color: id) -> id {
    let label = make_bare_label();
    let _: () = msg_send![label, setFont: font];
    let _: () = msg_send![label, setTextColor: color];
    label
}

unsafe fn make_footer_label() -> id {
    let label = make_plain_label(
        system_font(11.0, NS_FONT_WEIGHT_REGULAR),
        tertiary_label_color(),
    );
    let _: () = msg_send![label, setAlphaValue: 0.0_f64];
    // Single line so long conflict names ("Screenshot to clipboard") truncate
    // instead of wrapping over the window edge.
    let cell: id = msg_send![label, cell];
    let _: () = msg_send![cell, setUsesSingleLineMode: YES];
    let _: () = msg_send![cell, setWraps: NO];
    let _: () = msg_send![cell, setScrollable: YES];
    label
}

unsafe fn make_chip_label() -> id {
    // 13pt matches the row label size; monospaced so keyboard glyphs align.
    let font: id = msg_send![class!(NSFont), monospacedSystemFontOfSize: 13.0_f64 weight: NS_FONT_WEIGHT_MEDIUM];
    let label = make_plain_label(font, label_color());
    let _: () = msg_send![label, setAlignment: NS_TEXT_ALIGNMENT_CENTER];
    // Force single-line + no wrap so intrinsicContentSize reports the full
    // string width (NSTextField defaults wrap on intrinsic measurement).
    let cell: id = msg_send![label, cell];
    let _: () = msg_send![cell, setUsesSingleLineMode: YES];
    let _: () = msg_send![cell, setWraps: NO];
    let _: () = msg_send![cell, setScrollable: YES];
    label
}

/// The shortcut chip: a small NSBox well that reads as an input, with a
/// border that flips to accent (recording) or red (conflict). NSBox because
/// its fill/border colors are dynamic NSColors that track appearance.
unsafe fn make_chip_box(x: f64, y: f64, w: f64, h: f64) -> id {
    let chip: id = msg_send![class!(NSBox), alloc];
    let chip: id = msg_send![chip, init];
    let _: () = msg_send![chip, setBoxType: NS_BOX_CUSTOM];
    let _: () = msg_send![chip, setTitlePosition: 0u64];
    let _: () = msg_send![chip, setContentViewMargins: NSSize::new(0.0, 0.0)];
    let _: () = msg_send![chip, setCornerRadius: 6.0_f64];
    let _: () = msg_send![chip, setBorderWidth: 1.0_f64];
    let _: () = msg_send![chip, setBorderColor: separator_color()];
    let fill: id = msg_send![class!(NSColor), quaternaryLabelColor];
    let _: () = msg_send![chip, setFillColor: fill];
    set_frame(chip, x, y, w, h);
    chip
}

unsafe fn resize_chip_to_fit(chip: id, inner_label: id, record_btn: id) {
    // Measure the label's intrinsic content size, pad horizontally, and keep
    // the chip's right edge pinned at the card's control inset.
    //
    // Recording state widens the chip leftward toward the row label; the
    // label text is redundant context while recording, so approaching it
    // reads as the chip "taking focus" rather than a bug.
    //
    // intrinsicContentSize, not sizeToFit: sizeToFit wraps to the current
    // frame width on multi-line cells, which underreports the natural width.
    let intrinsic: NSSize = msg_send![inner_label, intrinsicContentSize];
    let text_w = intrinsic.width.max(28.0);
    let text_h = intrinsic.height;
    let chip_frame: NSRect = msg_send![chip, frame];
    let chip_w = (text_w + 18.0).max(CHIP_MIN_W);
    let chip_h = chip_frame.size.height;
    let chip_y = chip_frame.origin.y;
    let target_right = CARD_W - CONTROL_INSET;
    let chip_x = (target_right - chip_w).max(ROW_INSET);
    set_frame(chip, chip_x, chip_y, chip_w, chip_h);
    // Center the label as a tightly-bounded view inside the chip. NSTextField's
    // internal centerAlignment isn't reliable for keyboard-symbol glyphs, so we
    // size to fit + position the label frame ourselves.
    let label_x = ((chip_w - text_w) / 2.0).floor();
    let label_y = ((chip_h - text_h) / 2.0).floor();
    set_frame(inner_label, label_x, label_y, text_w, text_h);

    if record_btn != nil {
        // overlay button = chip frame (whole chip is the click affordance)
        set_frame(record_btn, chip_x, chip_y, chip_w, chip_h);
    }
}

unsafe fn make_bare_label() -> id {
    let label: id = msg_send![class!(NSTextField), alloc];
    let label: id = msg_send![label, init];
    let _: () = msg_send![label, setBezeled: NO];
    let _: () = msg_send![label, setEditable: NO];
    let _: () = msg_send![label, setSelectable: NO];
    let _: () = msg_send![label, setDrawsBackground: NO];
    label
}

unsafe fn make_switch() -> id {
    let sw: id = msg_send![class!(NSSwitch), alloc];
    msg_send![sw, init]
}

/// Item order must match `theme_index` / `theme_from_index`.
unsafe fn make_theme_popup() -> id {
    let popup: id = msg_send![class!(NSPopUpButton), alloc];
    let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(100.0, 25.0));
    let popup: id = msg_send![popup, initWithFrame: frame pullsDown: NO];
    for title in ["System", "Light", "Dark"] {
        let t = NSString::alloc(nil).init_str(title);
        let _: () = msg_send![popup, addItemWithTitle: t];
        let _: () = msg_send![t, release];
    }
    popup
}

unsafe fn make_stepper() -> id {
    let stepper: id = msg_send![class!(NSStepper), alloc];
    let stepper: id = msg_send![stepper, init];
    let _: () = msg_send![stepper, setMinValue: 0.0_f64];
    let _: () = msg_send![stepper, setMaxValue: MAX_POPUP_SECONDS];
    let _: () = msg_send![stepper, setIncrement: 0.1_f64];
    let _: () = msg_send![stepper, setValueWraps: NO];
    let _: () = msg_send![stepper, setAutorepeat: YES];
    stepper
}

unsafe fn make_invisible_button(tooltip: &str) -> id {
    let btn: id = msg_send![class!(NSButton), alloc];
    let btn: id = msg_send![btn, init];
    let empty = NSString::alloc(nil).init_str("");
    let _: () = msg_send![btn, setTitle: empty];
    let _: () = msg_send![empty, release];
    let _: () = msg_send![btn, setBordered: NO];
    let _: () = msg_send![btn, setTransparent: YES];
    let tt = NSString::alloc(nil).init_str(tooltip);
    let _: () = msg_send![btn, setToolTip: tt];
    // Tooltips surface as accessibilityHelp, not the label; without this the
    // empty-title button is announced as an unnamed control by VoiceOver.
    let _: () = msg_send![btn, setAccessibilityLabel: tt];
    let _: () = msg_send![tt, release];
    btn
}

/// Semantic red for warnings/errors — adapts to appearance and matches the
/// system's meaning of "something is wrong".
unsafe fn error_color() -> id {
    msg_send![class!(NSColor), systemRedColor]
}

/// The user's system accent color, for the recording focus border.
unsafe fn accent_color() -> id {
    msg_send![class!(NSColor), controlAccentColor]
}

unsafe fn separator_color() -> id {
    msg_send![class!(NSColor), separatorColor]
}

/// Read-only value display for the popup duration; the stepper is the sole
/// edit affordance (free-text editing raced auto-apply refreshes and was
/// dropped deliberately).
unsafe fn make_duration_field() -> id {
    let field: id = msg_send![class!(NSTextField), alloc];
    let field: id = msg_send![field, init];
    let _: () = msg_send![field, setBezeled: YES];
    let _: () = msg_send![field, setEditable: NO];
    let _: () = msg_send![field, setSelectable: NO];
    let _: () = msg_send![field, setDrawsBackground: YES];
    let _: () = msg_send![field, setAlignment: NS_TEXT_ALIGNMENT_RIGHT];
    let font: id = msg_send![class!(NSFont), monospacedDigitSystemFontOfSize: 13.0_f64 weight: NS_FONT_WEIGHT_REGULAR];
    let _: () = msg_send![field, setFont: font];
    let tt = NSString::alloc(nil).init_str("Seconds. 0 = never show.");
    let _: () = msg_send![field, setToolTip: tt];
    let _: () = msg_send![tt, release];
    field
}

unsafe fn set_frame(view: id, x: f64, y: f64, w: f64, h: f64) {
    let frame = NSRect::new(NSPoint::new(x, y), NSSize::new(w, h));
    let _: () = msg_send![view, setFrame: frame];
}

unsafe fn system_font(size: f64, weight: f64) -> id {
    msg_send![class!(NSFont), systemFontOfSize: size weight: weight]
}

unsafe fn label_color() -> id {
    msg_send![class!(NSColor), labelColor]
}
unsafe fn secondary_label_color() -> id {
    msg_send![class!(NSColor), secondaryLabelColor]
}
unsafe fn tertiary_label_color() -> id {
    msg_send![class!(NSColor), tertiaryLabelColor]
}

unsafe fn animate_appear(ns_window: id) {
    let frame: NSRect = msg_send![ns_window, frame];
    let start = NSRect::new(
        NSPoint::new(frame.origin.x, frame.origin.y - 6.0),
        frame.size,
    );
    let _: () = msg_send![ns_window, setFrame: start display: YES];
    let _: () = msg_send![ns_window, setAlphaValue: 0.0_f64];
    let ctx_cls = class!(NSAnimationContext);
    let _: () = msg_send![ctx_cls, beginGrouping];
    let current: id = msg_send![ctx_cls, currentContext];
    let _: () = msg_send![current, setDuration: 0.18_f64];
    let animator: id = msg_send![ns_window, animator];
    let _: () = msg_send![animator, setAlphaValue: 1.0_f64];
    let _: () = msg_send![animator, setFrame: frame display: YES];
    let _: () = msg_send![ctx_cls, endGrouping];
}
