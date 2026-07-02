/// Standalone preferences window. Editorial typographic styling, sibling to
/// the About window. Auto-apply semantics — no Save/Cancel; every control
/// change persists + fires `Message::ApplySettings`. A status dot in the
/// bottom-right pulses on each successful persist.
use crate::event_loop::{EventLoopMessage, EventLoopProxyMessage, Message};
use crate::settings::{Settings, ShortcutConfig};
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
const NS_TEXT_ALIGNMENT_RIGHT: u64 = 1;
const NS_FONT_WEIGHT_MEDIUM: f64 = 0.23;
const NS_FONT_WEIGHT_REGULAR: f64 = 0.0;

const WINDOW_SIZE: LogicalSize<f64> = LogicalSize::new(416.0, 190.0);
const DEBOUNCE_SECS: f64 = 0.4;
const STATUS_HOLD_SECS_OK: f64 = 0.6;
const STATUS_HOLD_SECS_ERR: f64 = 1.5;
const WARNING_HOLD_SECS: f64 = 3.0;
const RECORDING_PLACEHOLDER: &str = "Recording\u{2026}";

/// ivar payload for MMSettingsActions (leaked, app-lifetime valid).
struct ActionContext {
    proxy: EventLoopProxyMessage,
    settings: Arc<RwLock<Settings>>,
    launch_at_login_btn: id,
    popup_ms_field: id,
    status_label: id,
    shortcut_chip: id,
    shortcut_label: id,
    record_btn: id,
    warning_label: id,
    escape_btn: id,
    settings_window: id,
    /// Pending popup-duration commit timer (replaced on each keystroke).
    debounce_timer: Cell<id>,
    /// Pending status-dot fade-out timer (replaced on each persist).
    status_hide_timer: Cell<id>,
    /// Pending warning auto-clear timer.
    warning_hide_timer: Cell<id>,
    /// `true` between `recordShortcutAction:` firing and recorder callback.
    is_recording: Cell<bool>,
}

pub struct SettingsWindow {
    window: Window,
    launch_at_login_btn: id,
    popup_ms_field: id,
    shortcut_label: id,
    shortcut_chip: id,
    record_btn: id,
    warning_label: id,
    /// Invisible NSButton with Escape key equivalent — dispatches
    /// `Message::CloseSettings`. Native close paths (red button, Cmd-W)
    /// already route through tao's `WindowEvent::CloseRequested`.
    escape_btn: id,
    action_target: Option<id>,
    /// Visible flag, polled by the event-loop mtime path to skip live-reload
    /// while the user is editing.
    is_open: AtomicBool,
    status_label: id,
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
            shortcut_label: b.shortcut_label,
            shortcut_chip: b.shortcut_chip,
            record_btn: b.record_btn,
            warning_label: b.warning_label,
            escape_btn: b.escape_btn,
            action_target: None,
            is_open: AtomicBool::new(false),
            status_label: b.status_label,
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
            status_label: self.status_label,
            shortcut_chip: self.shortcut_chip,
            shortcut_label: self.shortcut_label,
            record_btn: self.record_btn,
            warning_label: self.warning_label,
            escape_btn: self.escape_btn,
            settings_window: ns_window,
            debounce_timer: Cell::new(nil),
            status_hide_timer: Cell::new(nil),
            warning_hide_timer: Cell::new(nil),
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
            // NSTextField fires `controlTextDidChange:` on its delegate per keystroke.
            let _: () = msg_send![self.popup_ms_field, setDelegate: target];
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
            // Don't stomp the popup-ms field while the user is editing it —
            // that would also cancel any in-flight IME composition.
            let editor: id = msg_send![self.popup_ms_field, currentEditor];
            if editor == nil {
                set_string(
                    self.popup_ms_field,
                    &format_seconds(settings.popup_duration_ms),
                );
            }
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
    /// recording / warning / status pulse appearance without a human click.
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
                    let _: () = msg_send![ctx.warning_label, setAlphaValue: 1.0_f64];
                }
                "warning" => {
                    show_warning(this, ctx, "\u{26A0} Already in use");
                    let _: () = msg_send![ctx.warning_label, setAlphaValue: 1.0_f64];
                }
                "status_ok" => {
                    show_status(this, ctx, true);
                    let _: () = msg_send![ctx.status_label, setAlphaValue: 1.0_f64];
                }
                "status_err" => {
                    show_status(this, ctx, false);
                    let _: () = msg_send![ctx.status_label, setAlphaValue: 1.0_f64];
                }
                _ => {}
            }
        }
    }

    /// In-process snapshot of the entire window (titlebar + content) to a PNG
    /// at `path`. Renders via `bitmapImageRepForCachingDisplayInRect:` +
    /// `cacheDisplayInRect:` on the window's root theme frame view, so it
    /// does NOT need Screen Recording permission and does NOT involve the
    /// window server compositor. Captures focus rings, layer-backed views,
    /// title bar, and the chip's CALayer fill correctly.
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
    persist_and_pulse(this, ctx, None, |s| s.launch_at_login = state == 1);
}

/// Per-keystroke; replaces any pending debounce timer with a fresh one
/// targeting `popupDurationCommit:` after `DEBOUNCE_SECS`. The timer is
/// retained while scheduled and released when fired/superseded so a stray
/// callback can't land on a dangling pointer.
extern "C" fn control_text_did_change(this: &Object, _cmd: Sel, _notification: *mut Object) {
    let ctx = unsafe { ctx_from(this) };
    unsafe {
        let timer: id = msg_send![
            class!(NSTimer),
            scheduledTimerWithTimeInterval: DEBOUNCE_SECS
            target: this selector: sel!(popupDurationCommit:)
            userInfo: nil repeats: NO
        ];
        let _: () = msg_send![timer, retain];
        let prev = ctx.debounce_timer.replace(timer);
        if prev != nil {
            let _: () = msg_send![prev, invalidate];
            let _: () = msg_send![prev, release];
        }
    }
}

extern "C" fn popup_duration_commit(this: &Object, _cmd: Sel, _timer: *mut Object) {
    let ctx = unsafe { ctx_from(this) };
    // Release + clear the fired timer slot before doing any work; releases
    // the +1 retain held while the timer was scheduled.
    let prev = ctx.debounce_timer.replace(nil);
    if prev != nil {
        unsafe {
            let _: () = msg_send![prev, release];
        }
    }
    let trimmed = unsafe {
        let raw: id = msg_send![ctx.popup_ms_field, stringValue];
        let cstr: *const i8 = msg_send![raw, UTF8String];
        if cstr.is_null() {
            String::new()
        } else {
            std::ffi::CStr::from_ptr(cstr)
                .to_string_lossy()
                .trim()
                .to_string()
        }
    };
    // parse::<f64> accepts "inf"/"nan"/"1e999"; without the finiteness and
    // range check those saturate `as u64` to u64::MAX and persist a popup
    // that never auto-hides. Cap at one day, well past any sane duration.
    let parsed = trimmed
        .parse::<f64>()
        .ok()
        .filter(|s| s.is_finite() && *s <= 86_400.0);
    match parsed {
        None => {
            // Empty / unparseable / absurd: revert to persisted value, no save.
            let current = ctx.settings.read().unwrap().popup_duration_ms;
            unsafe { set_string(ctx.popup_ms_field, &format_seconds(current)) };
        }
        Some(s) => {
            let ms = (s.max(0.0) * 1000.0).round() as u64;
            persist_and_pulse(this, ctx, None, |store| store.popup_duration_ms = ms);
        }
    }
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
    use super::format_seconds;

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
}

fn persist_and_pulse(
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
    }
    show_status(this, ctx, ok);
}

/// Pulse the status feedback in the bottom-right gutter. ok → "✓ Saved" in
/// muted gray, hold 600ms. err → "✗ Could not save" in red, hold 1500ms.
/// Fade in 80ms, out 320ms (NSAnimationContext + NSTimer-driven fade-out so
/// the next pulse can pre-empt the prior one).
fn show_status(this: &Object, ctx: &ActionContext, ok: bool) {
    unsafe {
        let (text, color): (&str, id) = if ok {
            ("\u{2713} Saved", tertiary_label_color())
        } else {
            ("\u{2717} Could not save", warning_color())
        };
        set_string(ctx.status_label, text);
        let _: () = msg_send![ctx.status_label, setTextColor: color];
        animate_single_alpha(ctx.status_label, 1.0, 0.08);

        let hold = if ok {
            STATUS_HOLD_SECS_OK
        } else {
            STATUS_HOLD_SECS_ERR
        };
        let timer: id = msg_send![
            class!(NSTimer),
            scheduledTimerWithTimeInterval: hold
            target: this selector: sel!(statusHide:)
            userInfo: nil repeats: NO
        ];
        let _: () = msg_send![timer, retain];
        let prev = ctx.status_hide_timer.replace(timer);
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
        let layer: id = msg_send![ctx.shortcut_chip, layer];
        let _: () = msg_send![layer, setBorderColor: recording_border_cg()];
        let _: () = msg_send![layer, setBorderWidth: 2.0_f64];
        set_string(ctx.shortcut_label, RECORDING_PLACEHOLDER);
        resize_chip_to_fit_ctx(ctx);
        // Recording helper is instructional, not a conflict: widen the frame
        // to full content width so the message doesn't truncate.
        set_warning_frame(ctx, true);
        set_string(ctx.warning_label, "Press a key combination. Esc to cancel.");
        let _: () = msg_send![ctx.warning_label, setTextColor: tertiary_label_color()];
        animate_single_alpha(ctx.warning_label, 1.0, 0.10);
    }
}

unsafe fn set_warning_frame(ctx: &ActionContext, full_width: bool) {
    let chip_frame: NSRect = msg_send![ctx.shortcut_chip, frame];
    let y = chip_frame.origin.y - WARNING_OFFSET;
    let (x, w) = if full_width {
        (SIDE_PAD, WINDOW_SIZE.width - 2.0 * SIDE_PAD)
    } else {
        (CONTROL_X, WINDOW_SIZE.width - CONTROL_X - SIDE_PAD)
    };
    set_frame(ctx.warning_label, x, y, w, WARNING_H);
}

fn exit_recording_visual(ctx: &ActionContext) {
    unsafe {
        reset_chip_border(ctx);
        // Fade the recording helper text out. show_warning's red copy will
        // override this if a conflict triggered the exit.
        animate_single_alpha(ctx.warning_label, 0.0, 0.10);
    }
}

fn show_warning(this: &Object, ctx: &ActionContext, text: &str) {
    unsafe {
        set_string(ctx.warning_label, text);
        // Conflict messages bind to the chip: narrow frame under the control.
        // Long reserved-shortcut names ("Screenshot to clipboard") overflow
        // that width, so fall back to the full content width when needed.
        let intrinsic: NSSize = msg_send![ctx.warning_label, intrinsicContentSize];
        let narrow_w = WINDOW_SIZE.width - CONTROL_X - SIDE_PAD;
        set_warning_frame(ctx, intrinsic.width > narrow_w);
        let _: () = msg_send![ctx.warning_label, setTextColor: warning_color()];
        animate_single_alpha(ctx.warning_label, 1.0, 0.12);
        // Red chip border binds the warning text visually to the offending field.
        let layer: id = msg_send![ctx.shortcut_chip, layer];
        let _: () = msg_send![layer, setBorderColor: conflict_border_cg()];
        let _: () = msg_send![layer, setBorderWidth: 1.5_f64];
        let timer: id = msg_send![
            class!(NSTimer),
            scheduledTimerWithTimeInterval: WARNING_HOLD_SECS
            target: this selector: sel!(warningHide:)
            userInfo: nil repeats: NO
        ];
        let _: () = msg_send![timer, retain];
        let prev = ctx.warning_hide_timer.replace(timer);
        if prev != nil {
            let _: () = msg_send![prev, invalidate];
            let _: () = msg_send![prev, release];
        }
    }
}

fn clear_warning(ctx: &ActionContext) {
    unsafe {
        let prev = ctx.warning_hide_timer.replace(nil);
        if prev != nil {
            let _: () = msg_send![prev, invalidate];
            let _: () = msg_send![prev, release];
        }
        animate_single_alpha(ctx.warning_label, 0.0, 0.12);
        // Restore the chip's idle border color.
        reset_chip_border(ctx);
    }
}

fn reset_chip_border(ctx: &ActionContext) {
    unsafe {
        let layer: id = msg_send![ctx.shortcut_chip, layer];
        let _: () = msg_send![layer, setBorderColor: idle_border_cg()];
        let _: () = msg_send![layer, setBorderWidth: 1.0_f64];
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
                sel!(controlTextDidChange:),
                control_text_did_change as extern "C" fn(&Object, Sel, *mut Object),
            );
            decl.add_method(
                sel!(popupDurationCommit:),
                popup_duration_commit as extern "C" fn(&Object, Sel, *mut Object),
            );
            decl.add_method(
                sel!(statusHide:),
                status_hide as extern "C" fn(&Object, Sel, *mut Object),
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
                sel!(warningHide:),
                warning_hide as extern "C" fn(&Object, Sel, *mut Object),
            );
        }
        decl.register()
    })
}

extern "C" fn close_settings(this: &Object, _cmd: Sel, _sender: *mut Object) {
    let ctx = unsafe { ctx_from(this) };
    let _ = ctx.proxy.send_event(Message::CloseSettings);
}

extern "C" fn status_hide(this: &Object, _cmd: Sel, _timer: *mut Object) {
    let ctx = unsafe { ctx_from(this) };
    let prev = ctx.status_hide_timer.replace(nil);
    if prev != nil {
        unsafe {
            let _: () = msg_send![prev, release];
        }
    }
    unsafe { animate_single_alpha(ctx.status_label, 0.0, 0.32) };
}

extern "C" fn warning_hide(this: &Object, _cmd: Sel, _timer: *mut Object) {
    let ctx = unsafe { ctx_from(this) };
    let prev = ctx.warning_hide_timer.replace(nil);
    if prev != nil {
        unsafe {
            let _: () = msg_send![prev, release];
        }
    }
    unsafe { animate_single_alpha(ctx.warning_label, 0.0, 0.24) };
    reset_chip_border(ctx);
}

/// Edit-button selector. Cancels any in-flight recorder, clears any visible
/// warning, then arms a new recorder on the chip frame. The capture callback
/// runs on the main thread (NSView keyDown delivery) so it can mutate UI and
/// settings without crossing thread boundaries.
extern "C" fn record_shortcut_action(this: &Object, _cmd: Sel, _sender: *mut Object) {
    let ctx = unsafe { ctx_from(this) };
    if ctx.is_recording.get() {
        // Second click while recording → cancel.
        shortcut_recorder::cancel_recording();
        return;
    }
    clear_warning(ctx);
    enter_recording_visual(ctx);
    ctx.is_recording.set(true);
    // Disable the invisible Escape button while recording so Escape reaches
    // the recorder view's cancelOperation: instead of closing the window.
    unsafe {
        let _: () = msg_send![ctx.escape_btn, setEnabled: NO];
    }

    let this_ptr = this as *const Object as id;
    let chip_frame: NSRect = unsafe { msg_send![ctx.shortcut_chip, frame] };
    let window = ctx.settings_window;
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
            persist_and_pulse(this, ctx, Some(previous), |s| {
                s.mic_shortcut = new_combo.clone()
            });
            // persist_and_pulse already refreshed via ApplySettings → apply_settings
            // → refresh_from, but that path only runs on the next event-loop turn.
            // Update the chip immediately so the user sees the new value.
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
    shortcut_label: id,
    shortcut_chip: id,
    record_btn: id,
    warning_label: id,
    status_label: id,
    escape_btn: id,
}

/// Form layout constants (logical points). Change a single value here to
/// reshape the entire window; no inline magic numbers in build_content_view.
const SIDE_PAD: f64 = 28.0;
const TOP_PAD: f64 = 18.0;
const ROW_H: f64 = 32.0;
const ROW_GAP: f64 = 14.0;
/// Wider gap between row 1 (shortcut) and row 2 (launch-at-login). Reserves
/// room for the inline collision warning beneath the shortcut chip.
const WARNING_ROW_GAP: f64 = 22.0;
/// Right edge x where labels end. Labels are right-aligned at this gutter.
const LABEL_GUTTER: f64 = 184.0;
/// Left edge x where controls begin. Gap from LABEL_GUTTER = label/control spacing.
const CONTROL_X: f64 = 196.0;
/// Width shared by chip + field. Column rhythm anchor.
const CONTROL_W: f64 = 170.0;
const CONTROL_H: f64 = 28.0;
const SWITCH_W: f64 = 38.0;
const SWITCH_H: f64 = 22.0;
const HELP_LEFT_GAP: f64 = 8.0;
const STATUS_LABEL_W: f64 = 130.0;
const STATUS_LABEL_H: f64 = 16.0;
const STATUS_BOTTOM_MARGIN: f64 = 18.0;
const WARNING_H: f64 = 16.0;
/// Distance from the chip's bottom edge down to the warning label's top.
const WARNING_OFFSET: f64 = 18.0;

unsafe fn build_content_view(window: &Window) -> BuiltViews {
    apply_window_chrome(window.ns_window() as id);
    let cv = window.ns_view() as id;
    let (w, h) = (WINDOW_SIZE.width, WINDOW_SIZE.height);

    let mut y = h - TOP_PAD - ROW_H;

    // row 1: Mute shortcut
    place_form_label(cv, "Mute shortcut:", y);
    let chip_h = ROW_H - 2.0;
    let chip_y = vcenter_in_row(y, chip_h);
    let shortcut_label = make_chip_label();
    let shortcut_chip = make_chip_view(CONTROL_X, chip_y, CONTROL_W, chip_h);
    let _: () = msg_send![shortcut_chip, addSubview: shortcut_label];
    let _: () = msg_send![cv, addSubview: shortcut_chip];
    let record_btn = make_invisible_button("Click to record a new shortcut");
    add(cv, place(record_btn, CONTROL_X, chip_y, CONTROL_W, chip_h));
    let warning_label = make_warning_label();
    add(
        cv,
        place(
            warning_label,
            CONTROL_X,
            y - WARNING_OFFSET,
            w - CONTROL_X - SIDE_PAD,
            WARNING_H,
        ),
    );
    // Row 1 → row 2 uses a wider gap than `ROW_GAP` so the inline warning
    // (alpha-0 by default) sits cleanly between the two rows when it pulses
    // visible, without overlapping "Launch at login" below.
    y -= ROW_H + WARNING_ROW_GAP;

    // row 2: Launch at login
    place_form_label(cv, "Launch at login:", y);
    let launch_at_login_btn = make_switch();
    // Right-align toggle to match chip + field right edge.
    let toggle_x = CONTROL_X + CONTROL_W - SWITCH_W;
    add(
        cv,
        place(
            launch_at_login_btn,
            toggle_x,
            vcenter_in_row(y, SWITCH_H),
            SWITCH_W,
            SWITCH_H,
        ),
    );
    y -= ROW_H + ROW_GAP;

    // row 3: Popup duration
    place_form_label(cv, "Popup duration:", y);
    let popup_ms_field = make_mono_text_field();
    add(
        cv,
        place(
            popup_ms_field,
            CONTROL_X,
            vcenter_in_row(y, CONTROL_H),
            CONTROL_W,
            CONTROL_H,
        ),
    );
    let help_x = CONTROL_X + CONTROL_W + HELP_LEFT_GAP;
    place_help_label(cv, "s", help_x, y);

    // Status text: invisible by default, animates in on save with a ✓/✗ glyph.
    let status_label = make_status_label();
    let label_x = w - SIDE_PAD - STATUS_LABEL_W;
    let label_y = STATUS_BOTTOM_MARGIN;
    add(
        cv,
        place(
            status_label,
            label_x,
            label_y,
            STATUS_LABEL_W,
            STATUS_LABEL_H,
        ),
    );

    let escape_btn = make_escape_button();
    add(cv, place(escape_btn, -10.0, -10.0, 1.0, 1.0));

    BuiltViews {
        launch_at_login_btn,
        popup_ms_field,
        shortcut_label,
        shortcut_chip,
        record_btn,
        warning_label,
        status_label,
        escape_btn,
    }
}

/// Create + position a form label, right-aligned at LABEL_GUTTER and
/// vertically centered in the row.
unsafe fn place_form_label(cv: id, text: &str, row_top: f64) {
    place_intrinsic_label(cv, make_form_label(text), |w| LABEL_GUTTER - w, row_top);
}

/// Place a help label (e.g. "s") pinned to `x` with its intrinsic width, so
/// it neither truncates nor stretches into the right margin.
unsafe fn place_help_label(cv: id, text: &str, x: f64, row_top: f64) {
    place_intrinsic_label(cv, make_help_label(text), |_| x, row_top);
}

/// sizeToFit gives the label its intrinsic width/height; `x_for_width` maps
/// that width to the label's left edge (right-aligned gutter or fixed pin).
unsafe fn place_intrinsic_label(
    cv: id,
    lbl: id,
    x_for_width: impl FnOnce(f64) -> f64,
    row_top: f64,
) {
    let _: () = msg_send![lbl, sizeToFit];
    let lf: NSRect = msg_send![lbl, frame];
    let lw = lf.size.width;
    let lh = lf.size.height;
    let ly = vcenter_in_row(row_top, lh);
    set_frame(lbl, x_for_width(lw), ly, lw, lh);
    add(cv, lbl);
}

/// Vertically center a `content_h`-tall element inside a `ROW_H`-tall row
/// starting at `row_top`. Floored to integer pixels for crisp rendering.
fn vcenter_in_row(row_top: f64, content_h: f64) -> f64 {
    row_top + ((ROW_H - content_h) / 2.0).floor()
}

unsafe fn make_form_label(text: &str) -> id {
    let label = make_plain_label(
        system_font(14.0, NS_FONT_WEIGHT_REGULAR),
        secondary_label_color(),
    );
    set_string(label, text);
    let _: () = msg_send![label, setAlignment: NS_TEXT_ALIGNMENT_RIGHT];
    label
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
    // Standard macOS prefs window: visible titlebar with "Settings" title.
    // Lock to dark appearance so the chip's CALayer colors (which don't auto-
    // adapt) stay visually consistent regardless of system theme.
    let title = NSString::alloc(nil).init_str("Settings");
    let _: () = msg_send![ns_window, setTitle: title];
    let _: () = msg_send![title, release];
    let _: () = msg_send![ns_window, setTitleVisibility: 0i64]; // NSWindowTitleVisible
    let _: () = msg_send![ns_window, setTitlebarAppearsTransparent: NO];
    let name = NSString::alloc(nil).init_str("NSAppearanceNameDarkAqua");
    let appearance: id = msg_send![class!(NSAppearance), appearanceNamed: name];
    let _: () = msg_send![name, release];
    let _: () = msg_send![ns_window, setAppearance: appearance];

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

unsafe fn make_help_label(text: &str) -> id {
    let label = make_plain_label(
        system_font(12.0, NS_FONT_WEIGHT_REGULAR),
        tertiary_label_color(),
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

unsafe fn make_status_label() -> id {
    let label = make_plain_label(
        system_font(11.0, NS_FONT_WEIGHT_REGULAR),
        tertiary_label_color(),
    );
    let _: () = msg_send![label, setAlignment: NS_TEXT_ALIGNMENT_RIGHT];
    let _: () = msg_send![label, setAlphaValue: 0.0_f64];
    label
}

unsafe fn make_chip_label() -> id {
    // 14pt matches the popup-duration field font so both controls in the form
    // have the same type weight.
    let font: id = msg_send![class!(NSFont), monospacedSystemFontOfSize: 14.0_f64 weight: NS_FONT_WEIGHT_MEDIUM];
    let label = make_plain_label(font, label_color());
    let _: () = msg_send![label, setAlignment: 2u64]; // center
                                                      // Force single-line + no wrap so intrinsicContentSize reports the full
                                                      // string width (NSTextField defaults wrap on intrinsic measurement).
    let cell: id = msg_send![label, cell];
    let _: () = msg_send![cell, setUsesSingleLineMode: YES];
    let _: () = msg_send![cell, setWraps: NO];
    let _: () = msg_send![cell, setScrollable: YES];
    label
}

unsafe fn make_chip_view(x: f64, y: f64, w: f64, h: f64) -> id {
    // Match the bordered look of NSTextField in dark mode so both controls in
    // this form read as the same "input" register. Approximated:
    //   bg     ≈ NSColor.controlBackgroundColor (dark) ≈ rgb(30,30,30)
    //   border ≈ NSColor.separatorColor (dark)         ≈ rgb(86,86,86, alpha 0.6)
    let v = make_layer_view(x, y, w, h);
    let layer: id = msg_send![v, layer];
    let bg: id = msg_send![class!(NSColor),
        colorWithSRGBRed: 0.117_f64 green: 0.117_f64 blue: 0.117_f64 alpha: 1.0_f64];
    let bg_cg: *mut c_void = msg_send![bg, CGColor];
    let _: () = msg_send![layer, setBackgroundColor: bg_cg];
    let _: () = msg_send![layer, setBorderColor: idle_border_cg()];
    let _: () = msg_send![layer, setBorderWidth: 1.0_f64];
    let _: () = msg_send![layer, setCornerRadius: 8.0_f64];
    v
}

/// Idle chip border, shared by the chip factory and every reset path so the
/// resting border can't drift between a fresh window and a post-recording one.
unsafe fn idle_border_cg() -> *mut c_void {
    let c: id = msg_send![class!(NSColor),
        colorWithSRGBRed: 0.337_f64 green: 0.337_f64 blue: 0.337_f64 alpha: 0.6_f64];
    msg_send![c, CGColor]
}

unsafe fn make_layer_view(x: f64, y: f64, w: f64, h: f64) -> id {
    let v: id = msg_send![class!(NSView), alloc];
    let v: id = msg_send![v, init];
    set_frame(v, x, y, w, h);
    let _: () = msg_send![v, setWantsLayer: YES];
    v
}

unsafe fn resize_chip_to_fit(chip: id, inner_label: id, record_btn: id) {
    // Measure the label's intrinsic content size, pad horizontally, and pick
    // the chip's right edge so the chip never overflows the window margin.
    //
    // Default state (chip_w == CONTROL_W): right edge at CONTROL_X + CONTROL_W,
    // flush with the field/switch column below.
    // Recording state (chip_w > CONTROL_W): right edge at the window's right
    // padding (row 1 has no help-rail, so this space is free).
    //
    // intrinsicContentSize, not sizeToFit: sizeToFit wraps to the current
    // frame width on multi-line cells, which underreports the natural width.
    let intrinsic: NSSize = msg_send![inner_label, intrinsicContentSize];
    let text_w = intrinsic.width.max(28.0);
    let text_h = intrinsic.height;
    let label_frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(text_w, text_h));
    let chip_frame: NSRect = msg_send![chip, frame];
    let chip_w = (text_w + 18.0).max(CONTROL_W);
    let chip_h = chip_frame.size.height;
    let chip_y = chip_frame.origin.y;
    let target_right = if chip_w > CONTROL_W {
        // Recording / overflow state: extend leftward into the label region.
        // The "Mute shortcut:" label is redundant context while recording, so
        // accepting overlap reads as the chip "taking focus" rather than a bug.
        WINDOW_SIZE.width - SIDE_PAD
    } else {
        CONTROL_X + CONTROL_W
    };
    let chip_x = (target_right - chip_w).max(SIDE_PAD);
    set_frame(chip, chip_x, chip_y, chip_w, chip_h);
    // Center the label as a tightly-bounded view inside the chip. NSTextField's
    // internal centerAlignment isn't reliable for keyboard-symbol glyphs, so we
    // size to fit + position the label frame ourselves.
    let label_w = label_frame.size.width;
    let label_h = label_frame.size.height;
    let label_x = ((chip_w - label_w) / 2.0).floor();
    let label_y = ((chip_h - label_h) / 2.0).floor();
    set_frame(inner_label, label_x, label_y, label_w, label_h);

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

unsafe fn make_warning_label() -> id {
    let label = make_plain_label(system_font(12.0, NS_FONT_WEIGHT_REGULAR), warning_color());
    let _: () = msg_send![label, setAlphaValue: 0.0_f64];
    // Single line so intrinsicContentSize reports the natural text width
    // (show_warning uses it to pick the narrow or full-width frame).
    let cell: id = msg_send![label, cell];
    let _: () = msg_send![cell, setUsesSingleLineMode: YES];
    let _: () = msg_send![cell, setWraps: NO];
    let _: () = msg_send![cell, setScrollable: YES];
    label
}

unsafe fn warning_color() -> id {
    // Accent #F2675F at full opacity — matches the status-dot pulse.
    msg_send![class!(NSColor),
        colorWithSRGBRed: 0.949_f64 green: 0.404_f64 blue: 0.373_f64 alpha: 1.0_f64]
}

/// Red used for the conflict-state chip border (matches the warning text).
unsafe fn conflict_border_cg() -> *mut c_void {
    msg_send![warning_color(), CGColor]
}

/// Macos system accent color (adapts to user pref). Used for the recording
/// focus border on the shortcut chip.
unsafe fn recording_border_cg() -> *mut c_void {
    let c: id = msg_send![class!(NSColor), controlAccentColor];
    msg_send![c, CGColor]
}

unsafe fn make_mono_text_field() -> id {
    let field: id = msg_send![class!(NSTextField), alloc];
    let field: id = msg_send![field, init];
    let _: () = msg_send![field, setBezeled: YES];
    let _: () = msg_send![field, setEditable: YES];
    let _: () = msg_send![field, setSelectable: YES];
    let _: () = msg_send![field, setDrawsBackground: YES];
    let _: () = msg_send![field, setAlignment: NS_TEXT_ALIGNMENT_RIGHT];
    let font: id = msg_send![class!(NSFont), monospacedDigitSystemFontOfSize: 14.0_f64 weight: NS_FONT_WEIGHT_REGULAR];
    let _: () = msg_send![field, setFont: font];
    // Placeholder mirrors format_seconds(default 1000ms); the field edits
    // seconds, not milliseconds.
    let placeholder = NSString::alloc(nil).init_str("1.0");
    let _: () = msg_send![field, setPlaceholderString: placeholder];
    let _: () = msg_send![placeholder, release];
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
