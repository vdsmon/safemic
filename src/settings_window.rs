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
const NS_VISUAL_EFFECT_MATERIAL_HUD_WINDOW: i64 = 12;
const NS_VISUAL_EFFECT_STATE_ACTIVE: i64 = 1;
const NS_VISUAL_EFFECT_BLENDING_MODE_BEHIND_WINDOW: i64 = 0;
const NS_WINDOW_STYLE_MASK_FULL_SIZE_CONTENT_VIEW: u64 = 1 << 15;
const NS_TEXT_ALIGNMENT_RIGHT: u64 = 1;
const NS_FONT_WEIGHT_MEDIUM: f64 = 0.23;
const NS_FONT_WEIGHT_SEMIBOLD: f64 = 0.3;
const NS_FONT_WEIGHT_REGULAR: f64 = 0.0;
// NSFontDescriptorSystemDesignSerif
const NS_FONT_DESCRIPTOR_SYSTEM_DESIGN_SERIF: &str = "NSCTFontUIFontDesignSerif";

const WINDOW_SIZE: LogicalSize<f64> = LogicalSize::new(480.0, 280.0);
const DEBOUNCE_SECS: f64 = 0.4;
const STATUS_HOLD_SECS_OK: f64 = 0.6;
const STATUS_HOLD_SECS_ERR: f64 = 1.5;
const WARNING_HOLD_SECS: f64 = 3.0;
const RECORDING_PLACEHOLDER: &str = "press a combo\u{2026}";

/// ivar payload for MMSettingsActions (leaked, app-lifetime valid).
struct ActionContext {
    proxy: EventLoopProxyMessage,
    settings: Arc<RwLock<Settings>>,
    launch_at_login_btn: id,
    popup_ms_field: id,
    status_dot: id,
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
    status_dot: id,
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
            status_dot: b.status_dot,
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
            status_dot: self.status_dot,
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
                set_string(self.popup_ms_field, &settings.popup_duration_ms.to_string());
            }
            set_string(
                self.shortcut_label,
                &format_shortcut(&settings.mic_shortcut),
            );
            resize_chip_to_fit(self.shortcut_chip, self.shortcut_label, self.record_btn);
        }
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
    unsafe {
        let raw: id = msg_send![ctx.popup_ms_field, stringValue];
        let cstr: *const i8 = msg_send![raw, UTF8String];
        let trimmed = if cstr.is_null() {
            String::new()
        } else {
            std::ffi::CStr::from_ptr(cstr)
                .to_string_lossy()
                .trim()
                .to_string()
        };
        if trimmed.is_empty() {
            // Empty / whitespace-only — revert the field to the persisted
            // value without saving. Plain digits parsing to 0 stay valid
            // (the help label documents "0 = never show").
            let current = ctx.settings.read().unwrap().popup_duration_ms;
            set_string(ctx.popup_ms_field, &current.to_string());
            return;
        }
    }
    let ms: i64 = unsafe { msg_send![ctx.popup_ms_field, integerValue] };
    persist_and_pulse(this, ctx, None, |s| s.popup_duration_ms = ms.max(0) as u64);
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

/// Pulse the status dot. ok → accent #F2675F, hold 600ms. err → systemRed,
/// hold 1500ms. Fade in 80ms, out 320ms (NSAnimationContext + NSTimer-driven
/// fade-out so the next pulse can pre-empt the prior one).
fn show_status(this: &Object, ctx: &ActionContext, ok: bool) {
    unsafe {
        let dot_color: id = if ok {
            msg_send![class!(NSColor),
                colorWithSRGBRed: 0.949_f64 green: 0.404_f64 blue: 0.373_f64 alpha: 1.0_f64]
        } else {
            msg_send![class!(NSColor), systemRedColor]
        };
        let cg: *mut c_void = msg_send![dot_color, CGColor];
        let layer: id = msg_send![ctx.status_dot, layer];
        let _: () = msg_send![layer, setBackgroundColor: cg];
        let label_str = NSString::alloc(nil).init_str(if ok { "saved" } else { "error" });
        let _: () = msg_send![ctx.status_label, setStringValue: label_str];
        let _: () = msg_send![label_str, release];
        animate_alpha(ctx.status_dot, ctx.status_label, 1.0, 0.08);

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

unsafe fn animate_alpha(a: id, b: id, alpha: f64, secs: f64) {
    let ctx_cls = class!(NSAnimationContext);
    let _: () = msg_send![ctx_cls, beginGrouping];
    let current: id = msg_send![ctx_cls, currentContext];
    let _: () = msg_send![current, setDuration: secs];
    let aa: id = msg_send![a, animator];
    let ba: id = msg_send![b, animator];
    let _: () = msg_send![aa, setAlphaValue: alpha];
    let _: () = msg_send![ba, setAlphaValue: alpha];
    let _: () = msg_send![ctx_cls, endGrouping];
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
        let _: () = msg_send![layer, setBorderColor: accent_cg()];
        let _: () = msg_send![layer, setBorderWidth: 1.5_f64];
        set_string(ctx.shortcut_label, RECORDING_PLACEHOLDER);
        resize_chip_to_fit_ctx(ctx);
    }
}

fn exit_recording_visual(ctx: &ActionContext) {
    unsafe {
        let layer: id = msg_send![ctx.shortcut_chip, layer];
        let c: id = msg_send![class!(NSColor),
            colorWithSRGBRed: 1.0_f64 green: 1.0_f64 blue: 1.0_f64 alpha: 0.15_f64];
        let cg: *mut c_void = msg_send![c, CGColor];
        let _: () = msg_send![layer, setBorderColor: cg];
        let _: () = msg_send![layer, setBorderWidth: 1.0_f64];
    }
}

fn show_warning(this: &Object, ctx: &ActionContext, text: &str) {
    unsafe {
        set_string(ctx.warning_label, text);
        animate_single_alpha(ctx.warning_label, 1.0, 0.12);
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
    unsafe { animate_alpha(ctx.status_dot, ctx.status_label, 0.0, 0.32) };
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
            show_warning(this, ctx, "Shortcut needs at least one modifier");
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
    status_dot: id,
    status_label: id,
    escape_btn: id,
}

unsafe fn build_content_view(window: &Window) -> BuiltViews {
    apply_window_chrome(window.ns_window() as id);
    let cv = window.ns_view() as id;
    let (w, h) = (WINDOW_SIZE.width, WINDOW_SIZE.height);

    add(
        cv,
        place(make_visual_effect_view(0.0, 0.0, w, h), 0.0, 0.0, w, h),
    );
    add(
        cv,
        place(make_serif_wordmark("Settings"), 24.0, h - 52.0, 200.0, 28.0),
    );
    let divider_y = h - 76.0;
    add(
        cv,
        place(
            make_divider(24.0, divider_y, w - 48.0),
            24.0,
            divider_y,
            w - 48.0,
            1.0,
        ),
    );

    let (rh, rg) = (28.0, 20.0);
    let mut row_top = divider_y - rh;

    // row 1: mute shortcut chip + Edit button
    add(
        cv,
        place(
            make_section_label("MUTE SHORTCUT"),
            24.0,
            row_top,
            180.0,
            rh,
        ),
    );
    let shortcut_label = make_chip_label();
    let shortcut_chip = make_chip_view(0.0, row_top + 2.0, 64.0, rh - 4.0);
    let _: () = msg_send![shortcut_chip, addSubview: shortcut_label];
    let _: () = msg_send![cv, addSubview: shortcut_chip];
    // Edit affordance: small text button positioned left of the chip with an
    // 8pt gap. resize_chip_to_fit re-runs whenever the chip text changes, so
    // we re-pin record_btn there too.
    let (bw, bh) = (44.0, rh - 4.0);
    let record_btn = make_text_button("Edit");
    add(cv, place(record_btn, 0.0, row_top + 2.0, bw, bh));
    // Inline warning row: sits 4pt under the chip baseline, hidden until used.
    let warning_label = make_warning_label();
    add(
        cv,
        place(warning_label, 24.0, row_top - 18.0, w - 48.0, 14.0),
    );
    row_top -= rh + rg;

    // row 2: launch at login (NSSwitch right)
    add(
        cv,
        place(
            make_section_label("LAUNCH AT LOGIN"),
            24.0,
            row_top,
            200.0,
            rh,
        ),
    );
    let (sw, sh) = (38.0, 22.0);
    let launch_at_login_btn = make_switch();
    add(
        cv,
        place(
            launch_at_login_btn,
            w - 24.0 - sw,
            row_top + (rh - sh) / 2.0,
            sw,
            sh,
        ),
    );
    row_top -= rh + rg;

    // row 3: popup duration (ms) + help below
    add(
        cv,
        place(
            make_section_label("POPUP DURATION (MS)"),
            24.0,
            row_top,
            220.0,
            rh,
        ),
    );
    let (fw, fh) = (96.0, 22.0);
    let popup_ms_field = make_mono_text_field();
    add(
        cv,
        place(
            popup_ms_field,
            w - 24.0 - fw,
            row_top + (rh - fh) / 2.0,
            fw,
            fh,
        ),
    );
    add(
        cv,
        place(
            make_help_label("0 = never show \u{B7} default 1000"),
            24.0,
            row_top - 20.0,
            w - 48.0,
            16.0,
        ),
    );

    // status indicator: 6pt dot + label, bottom-right (16pt margins)
    let (ds, dx, dy) = (6.0, w - 22.0, 16.0);
    let status_dot = make_status_dot(dx, dy, ds);
    let _: () = msg_send![cv, addSubview: status_dot];
    let (lw, lh) = (64.0, 14.0);
    let status_label = make_status_label();
    add(
        cv,
        place(status_label, dx - 6.0 - lw, dy + (ds - lh) / 2.0, lw, lh),
    );

    // Invisible Escape-key button: zero-size, no border.
    let escape_btn = make_escape_button();
    add(cv, place(escape_btn, -10.0, -10.0, 1.0, 1.0));

    BuiltViews {
        launch_at_login_btn,
        popup_ms_field,
        shortcut_label,
        shortcut_chip,
        record_btn,
        warning_label,
        status_dot,
        status_label,
        escape_btn,
    }
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
    // transparent titlebar so the HUD blur reaches the top edge; vibrant dark
    // appearance matches the HUD regardless of system theme.
    let _: () = msg_send![ns_window, setTitlebarAppearsTransparent: YES];
    let _: () = msg_send![ns_window, setTitleVisibility: 1i64]; // NSWindowTitleHidden
    let current: u64 = msg_send![ns_window, styleMask];
    let _: () =
        msg_send![ns_window, setStyleMask: current | NS_WINDOW_STYLE_MASK_FULL_SIZE_CONTENT_VIEW];
    let _: () = msg_send![ns_window, setMovableByWindowBackground: YES];
    let appearance: id = msg_send![class!(NSAppearance), appearanceNamed: NSString::alloc(nil).init_str("NSAppearanceNameVibrantDark")];
    let _: () = msg_send![ns_window, setAppearance: appearance];
}

unsafe fn make_visual_effect_view(x: f64, y: f64, w: f64, h: f64) -> id {
    let v: id = msg_send![class!(NSVisualEffectView), alloc];
    let v: id = msg_send![v, init];
    set_frame(v, x, y, w, h);
    let _: () = msg_send![v, setMaterial: NS_VISUAL_EFFECT_MATERIAL_HUD_WINDOW];
    let _: () = msg_send![v, setState: NS_VISUAL_EFFECT_STATE_ACTIVE];
    let _: () = msg_send![v, setBlendingMode: NS_VISUAL_EFFECT_BLENDING_MODE_BEHIND_WINDOW];
    let _: () = msg_send![v, setAutoresizingMask: 2u64 | 16u64]; // width + height sizable
    v
}

unsafe fn make_serif_wordmark(text: &str) -> id {
    make_attr_label(
        text,
        serif_font(22.0, NS_FONT_WEIGHT_SEMIBOLD),
        -0.4,
        label_color(),
    )
}

unsafe fn make_section_label(text: &str) -> id {
    make_attr_label(
        text,
        system_font(12.0, NS_FONT_WEIGHT_MEDIUM),
        1.5,
        secondary_label_color(),
    )
}

unsafe fn make_attr_label(text: &str, font: id, kern: f64, color: id) -> id {
    let label = make_bare_label();
    let s = NSString::alloc(nil).init_str(text);
    let attr = attributed_string_with_kern(s, kern, font, color);
    let _: () = msg_send![label, setAttributedStringValue: attr];
    let _: () = msg_send![s, release];
    let _: () = msg_send![attr, release];
    label
}

unsafe fn make_help_label(text: &str) -> id {
    let label = make_plain_label(
        system_font(11.0, NS_FONT_WEIGHT_REGULAR),
        tertiary_label_color(),
    );
    let s = NSString::alloc(nil).init_str(text);
    let _: () = msg_send![label, setStringValue: s];
    let _: () = msg_send![s, release];
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
        system_font(10.0, NS_FONT_WEIGHT_REGULAR),
        tertiary_label_color(),
    );
    let _: () = msg_send![label, setAlignment: NS_TEXT_ALIGNMENT_RIGHT];
    let _: () = msg_send![label, setAlphaValue: 0.0_f64];
    label
}

unsafe fn make_status_dot(x: f64, y: f64, size: f64) -> id {
    let v = make_layer_view(x, y, size, size);
    let layer: id = msg_send![v, layer];
    let _: () = msg_send![layer, setCornerRadius: size / 2.0];
    let _: () = msg_send![v, setAlphaValue: 0.0_f64];
    v
}

unsafe fn make_chip_label() -> id {
    let font: id = msg_send![class!(NSFont), monospacedSystemFontOfSize: 12.0_f64 weight: NS_FONT_WEIGHT_MEDIUM];
    let label = make_plain_label(font, label_color());
    let _: () = msg_send![label, setAlignment: 2u64]; // center
    label
}

unsafe fn make_chip_view(x: f64, y: f64, w: f64, h: f64) -> id {
    let v = make_layer_view(x, y, w, h);
    let layer: id = msg_send![v, layer];
    let _: () = msg_send![layer, setBackgroundColor: white_cg(0.10)];
    let _: () = msg_send![layer, setBorderColor: white_cg(0.15)];
    let _: () = msg_send![layer, setBorderWidth: 1.0_f64];
    let _: () = msg_send![layer, setCornerRadius: 6.0_f64];
    v
}

unsafe fn make_layer_view(x: f64, y: f64, w: f64, h: f64) -> id {
    let v: id = msg_send![class!(NSView), alloc];
    let v: id = msg_send![v, init];
    set_frame(v, x, y, w, h);
    let _: () = msg_send![v, setWantsLayer: YES];
    v
}

unsafe fn white_cg(alpha: f64) -> *mut c_void {
    let c: id = msg_send![class!(NSColor),
        colorWithSRGBRed: 1.0_f64 green: 1.0_f64 blue: 1.0_f64 alpha: alpha];
    msg_send![c, CGColor]
}

unsafe fn resize_chip_to_fit(chip: id, inner_label: id, record_btn: id) {
    // measure the label's intrinsic content size, pad horizontally, and pin
    // the chip's right edge to the window's right margin.
    let _: () = msg_send![inner_label, sizeToFit];
    let label_frame: NSRect = msg_send![inner_label, frame];
    let text_w = label_frame.size.width.max(28.0);
    let chip_frame: NSRect = msg_send![chip, frame];
    let chip_w = text_w + 18.0;
    let chip_h = chip_frame.size.height;
    let chip_y = chip_frame.origin.y;
    let chip_x = WINDOW_SIZE.width - 24.0 - chip_w;
    set_frame(chip, chip_x, chip_y, chip_w, chip_h);
    let label_y = (chip_h - label_frame.size.height) / 2.0;
    set_frame(inner_label, 0.0, label_y, chip_w, label_frame.size.height);

    if record_btn != nil {
        let btn_frame: NSRect = msg_send![record_btn, frame];
        let bw = btn_frame.size.width;
        let bh = btn_frame.size.height;
        let btn_y = chip_y + (chip_h - bh) / 2.0;
        let btn_x = chip_x - 8.0 - bw;
        set_frame(record_btn, btn_x, btn_y, bw, bh);
    }
}

unsafe fn make_divider(x: f64, y: f64, w: f64) -> id {
    let v = make_layer_view(x, y, w, 1.0);
    let layer: id = msg_send![v, layer];
    let _: () = msg_send![layer, setBackgroundColor: white_cg(0.15)];
    v
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

unsafe fn make_text_button(text: &str) -> id {
    let btn: id = msg_send![class!(NSButton), alloc];
    let btn: id = msg_send![btn, init];
    let title = NSString::alloc(nil).init_str(text);
    let _: () = msg_send![btn, setTitle: title];
    let _: () = msg_send![title, release];
    // NSBezelStyleInline (12) reads as a compact secondary control on macOS 11+.
    let _: () = msg_send![btn, setBezelStyle: 12i64];
    let _: () = msg_send![btn, setBordered: YES];
    let font: id =
        msg_send![class!(NSFont), systemFontOfSize: 11.0_f64 weight: NS_FONT_WEIGHT_MEDIUM];
    let _: () = msg_send![btn, setFont: font];
    btn
}

unsafe fn make_warning_label() -> id {
    let label = make_plain_label(system_font(11.0, NS_FONT_WEIGHT_REGULAR), warning_color());
    let _: () = msg_send![label, setAlphaValue: 0.0_f64];
    label
}

unsafe fn warning_color() -> id {
    // Accent #F2675F at full opacity — matches the status-dot pulse.
    msg_send![class!(NSColor),
        colorWithSRGBRed: 0.949_f64 green: 0.404_f64 blue: 0.373_f64 alpha: 1.0_f64]
}

unsafe fn accent_cg() -> *mut c_void {
    let c: id = msg_send![class!(NSColor),
        colorWithSRGBRed: 0.949_f64 green: 0.404_f64 blue: 0.373_f64 alpha: 1.0_f64];
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
    let font: id = msg_send![class!(NSFont), monospacedDigitSystemFontOfSize: 13.0_f64 weight: NS_FONT_WEIGHT_REGULAR];
    let _: () = msg_send![field, setFont: font];
    let placeholder = NSString::alloc(nil).init_str("1000");
    let _: () = msg_send![field, setPlaceholderString: placeholder];
    let _: () = msg_send![placeholder, release];
    field
}

unsafe fn set_frame(view: id, x: f64, y: f64, w: f64, h: f64) {
    let frame = NSRect::new(NSPoint::new(x, y), NSSize::new(w, h));
    let _: () = msg_send![view, setFrame: frame];
}

unsafe fn system_font(size: f64, weight: f64) -> id {
    msg_send![class!(NSFont), systemFontOfSize: size weight: weight]
}

unsafe fn serif_font(size: f64, weight: f64) -> id {
    let base: id = msg_send![class!(NSFont), systemFontOfSize: size weight: weight];
    let descriptor: id = msg_send![base, fontDescriptor];
    let design = NSString::alloc(nil).init_str(NS_FONT_DESCRIPTOR_SYSTEM_DESIGN_SERIF);
    let serif_desc: id = msg_send![descriptor, fontDescriptorWithDesign: design];
    let _: () = msg_send![design, release];
    if serif_desc == nil {
        return base;
    }
    let serif: id = msg_send![class!(NSFont), fontWithDescriptor: serif_desc size: size];
    if serif == nil {
        base
    } else {
        serif
    }
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

unsafe fn attributed_string_with_kern(s: id, kern: f64, font: id, color: id) -> id {
    let attrs: id = msg_send![class!(NSMutableDictionary), dictionary];
    let kern_num: id = msg_send![class!(NSNumber), numberWithDouble: kern];
    dict_set(attrs, "NSKern", kern_num);
    dict_set(attrs, "NSFont", font);
    dict_set(attrs, "NSColor", color);
    let attr: id = msg_send![class!(NSAttributedString), alloc];
    msg_send![attr, initWithString: s attributes: attrs]
}

unsafe fn dict_set(dict: id, key: &str, value: id) {
    let k = NSString::alloc(nil).init_str(key);
    let _: () = msg_send![dict, setObject: value forKey: k];
    let _: () = msg_send![k, release];
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
