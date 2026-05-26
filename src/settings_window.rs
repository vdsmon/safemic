/// Standalone preferences window. Built once at app start (hidden) so opens
/// stay cheap. `open()` shows the window and pushes current `Settings` into
/// the controls; `close()` hides it. Save/Cancel buttons are wired via
/// NSButton target/action in `bind_actions`.
use crate::event_loop::{EventLoopMessage, EventLoopProxyMessage, Message};
use crate::settings::Settings;
use crate::utils::format_shortcut;
use anyhow::{Context, Result};
use cocoa::base::{id, nil, NO, YES};
use cocoa::foundation::{NSPoint, NSRect, NSSize, NSString};
use log::trace;
use objc::declare::ClassDecl;
use objc::runtime::{Class, Object, Sel};
use std::os::raw::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, RwLock};
use tao::dpi::LogicalSize;
use tao::platform::macos::WindowExtMacOS;
use tao::window::{Window, WindowBuilder, WindowId};

// AppKit constants (raw ObjC enums, not exposed by cocoa 0.24 helpers).
const NS_BUTTON_TYPE_SWITCH: i64 = 3;
const NS_BEZEL_STYLE_ROUNDED: i64 = 1;
const NS_USER_INTERFACE_LAYOUT_ORIENTATION_VERTICAL: i64 = 1;
const NS_USER_INTERFACE_LAYOUT_ORIENTATION_HORIZONTAL: i64 = 0;
const NS_LAYOUT_ATTRIBUTE_LEADING: i64 = 5;
const NS_LAYOUT_ATTRIBUTE_WIDTH: i64 = 7;

const WINDOW_SIZE: LogicalSize<f64> = LogicalSize::new(380.0, 260.0);

/// Pointer held in the MMSettingsActions ivar. Leaked at `bind_actions` time
/// so the pointer is valid for the entire app lifetime.
struct ActionContext {
    proxy: EventLoopProxyMessage,
    settings: Arc<RwLock<Settings>>,
    launch_at_login_btn: id,
    popup_ms_field: id,
}

pub struct SettingsWindow {
    window: Window,
    launch_at_login_btn: id,
    popup_ms_field: id,
    shortcut_label: id,
    save_btn: id,
    cancel_btn: id,
    /// `Some(_)` once `bind_actions` has been called. The boxed `ActionContext`
    /// is leaked via `Box::into_raw` and never freed; this field exists so a
    /// future code path could re-bind (e.g. on settings reload) without
    /// orphaning the previous box.
    action_target: Option<id>,
    /// True while the settings window is visible. Used by the event-loop
    /// mtime-poll path to skip live-reload while the user is editing (avoids
    /// a lost-update race between disk reload and Save).
    is_open: AtomicBool,
}

impl SettingsWindow {
    pub fn new(event_loop: &EventLoopMessage) -> Result<Self> {
        let window = WindowBuilder::new()
            .with_title("Mic Mute Settings")
            .with_inner_size(WINDOW_SIZE)
            .with_resizable(false)
            .with_visible(false)
            .with_closable(true)
            .with_minimizable(false)
            .with_maximized(false)
            .build(event_loop)
            .context("Failed to build settings window")?;

        let (launch_at_login_btn, popup_ms_field, shortcut_label, save_btn, cancel_btn) =
            unsafe { build_content_view(&window) };

        Ok(Self {
            window,
            launch_at_login_btn,
            popup_ms_field,
            shortcut_label,
            save_btn,
            cancel_btn,
            action_target: None,
            is_open: AtomicBool::new(false),
        })
    }

    /// Late-binds Save/Cancel target/action handlers. Must be called once,
    /// after `event_loop::start` constructs the proxy.
    ///
    /// SAFETY: `ActionContext` is leaked with `Box::into_raw`, so the ivar
    /// pointer is valid for the entire app lifetime. NSButton action
    /// selectors fire synchronously on the main thread (tao runs the run
    /// loop on main), so single-threaded mutation through the raw pointer is
    /// sound; no `Mutex` (would deadlock against `UI::apply_settings`).
    pub fn bind_actions(&mut self, settings: Arc<RwLock<Settings>>, proxy: EventLoopProxyMessage) {
        debug_assert!(self.action_target.is_none(), "bind_actions called twice");
        if self.action_target.is_some() {
            return;
        }
        let ctx = Box::new(ActionContext {
            proxy,
            settings,
            launch_at_login_btn: self.launch_at_login_btn,
            popup_ms_field: self.popup_ms_field,
        });
        let ctx_ptr = Box::into_raw(ctx) as *mut c_void;

        let cls = actions_class();
        let target: id = unsafe {
            let obj: id = msg_send![cls, alloc];
            let obj: id = msg_send![obj, init];
            (*obj).set_ivar("_ctxPtr", ctx_ptr);
            obj
        };

        unsafe {
            let _: () = msg_send![self.save_btn, setTarget: target];
            let _: () = msg_send![self.save_btn, setAction: sel!(saveAction:)];
            let _: () = msg_send![self.cancel_btn, setTarget: target];
            let _: () = msg_send![self.cancel_btn, setAction: sel!(cancelAction:)];
        }
        self.action_target = Some(target);
    }

    pub fn id(&self) -> WindowId {
        self.window.id()
    }

    pub fn open(&self, settings: &Settings) {
        trace!("Opening settings window");
        self.refresh_from(settings);
        self.window.set_visible(true);
        unsafe {
            let ns_window = self.window.ns_window() as id;
            let _: () = msg_send![ns_window, orderFrontRegardless];
            let _: () = msg_send![ns_window, makeKeyWindow];
        }
        self.is_open.store(true, Ordering::SeqCst);
    }

    pub fn close(&self) {
        trace!("Closing settings window");
        self.window.set_visible(false);
        self.is_open.store(false, Ordering::SeqCst);
    }

    pub fn is_open(&self) -> bool {
        self.is_open.load(Ordering::SeqCst)
    }

    /// Push current `Settings` values into every visible control. Used both
    /// when opening the window and when external settings changes arrive
    /// while the window is visible.
    pub fn refresh_from(&self, settings: &Settings) {
        unsafe {
            let _: () =
                msg_send![self.launch_at_login_btn, setState: settings.launch_at_login as i64];
            let ms = settings.popup_duration_ms.to_string();
            let ms_str = NSString::alloc(nil).init_str(&ms);
            let _: () = msg_send![self.popup_ms_field, setStringValue: ms_str];
            let _: () = msg_send![ms_str, release];
            let sc = format_shortcut(&settings.mic_shortcut);
            let sc_text = format!("Mute shortcut: {sc}");
            let sc_str = NSString::alloc(nil).init_str(&sc_text);
            let _: () = msg_send![self.shortcut_label, setStringValue: sc_str];
            let _: () = msg_send![sc_str, release];
        }
    }
}

// SAFETY: the underlying `tao::window::Window` is `Send + Sync` per tao's
// macOS implementation; all UI mutations happen on the main thread inside the
// event-loop closure.
unsafe impl Send for SettingsWindow {}
unsafe impl Sync for SettingsWindow {}

// ---- ClassDecl: custom NSObject with Save/Cancel selectors --------------

extern "C" fn save_action(this: &Object, _cmd: Sel, _sender: *mut Object) {
    let ctx = unsafe { ctx_from(this) };
    let launch: i64 = unsafe { msg_send![ctx.launch_at_login_btn, state] };
    let ms: i64 = unsafe { msg_send![ctx.popup_ms_field, integerValue] };

    {
        let mut s = ctx.settings.write().unwrap();
        s.launch_at_login = launch == 1;
        s.popup_duration_ms = ms.max(0) as u64;
        if let Err(e) = s.save() {
            log::error!("Failed to save settings: {}", e);
        }
    }
    let _ = ctx.proxy.send_event(Message::ApplySettings);
    let _ = ctx.proxy.send_event(Message::CloseSettings);
}

extern "C" fn cancel_action(this: &Object, _cmd: Sel, _sender: *mut Object) {
    let ctx = unsafe { ctx_from(this) };
    let _ = ctx.proxy.send_event(Message::CloseSettings);
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
                sel!(saveAction:),
                save_action as extern "C" fn(&Object, Sel, *mut Object),
            );
            decl.add_method(
                sel!(cancelAction:),
                cancel_action as extern "C" fn(&Object, Sel, *mut Object),
            );
        }
        decl.register()
    })
}

// ---- View construction ---------------------------------------------------

unsafe fn build_content_view(window: &Window) -> (id, id, id, id, id) {
    let content_view = window.ns_view() as id;

    let stack = make_vstack(12.0);
    set_frame(
        stack,
        20.0,
        20.0,
        WINDOW_SIZE.width - 40.0,
        WINDOW_SIZE.height - 40.0,
    );

    let shortcut_label = make_label("Mute shortcut: ");
    let _: () = msg_send![stack, addArrangedSubview: shortcut_label];

    let launch_at_login_btn = make_checkbox("Launch at login");
    let _: () = msg_send![stack, addArrangedSubview: launch_at_login_btn];

    let (ms_row, popup_ms_field) = make_popup_ms_row();
    let _: () = msg_send![stack, addArrangedSubview: ms_row];

    let hint = make_label("0 = never show. Default: 1000.");
    let ns_font = class!(NSFont);
    let small_size: f64 = msg_send![ns_font, smallSystemFontSize];
    let small_font: id = msg_send![ns_font, systemFontOfSize: small_size];
    let _: () = msg_send![hint, setFont: small_font];
    let _: () = msg_send![stack, addArrangedSubview: hint];

    // spacer absorbs vertical slack so the action row sits at the bottom
    let spacer: id = msg_send![class!(NSView), alloc];
    let spacer: id = msg_send![spacer, init];
    let _: () = msg_send![spacer, setTranslatesAutoresizingMaskIntoConstraints: NO];
    // NSLayoutPriorityDefaultLow = 250 — let the spacer expand to fill vertical slack
    let _: () = msg_send![spacer, setContentHuggingPriority: 1.0_f32 forOrientation: 1i64];
    let _: () =
        msg_send![spacer, setContentCompressionResistancePriority: 1.0_f32 forOrientation: 1i64];
    let _: () = msg_send![stack, addArrangedSubview: spacer];

    let (action_row, save_btn, cancel_btn) = make_action_row();
    let _: () = msg_send![stack, addArrangedSubview: action_row];

    let _: () = msg_send![content_view, addSubview: stack];

    (
        launch_at_login_btn,
        popup_ms_field,
        shortcut_label,
        save_btn,
        cancel_btn,
    )
}

unsafe fn make_vstack(spacing: f64) -> id {
    let stack: id = msg_send![class!(NSStackView), alloc];
    let stack: id = msg_send![stack, init];
    let _: () = msg_send![stack, setOrientation: NS_USER_INTERFACE_LAYOUT_ORIENTATION_VERTICAL];
    let _: () = msg_send![stack, setAlignment: NS_LAYOUT_ATTRIBUTE_LEADING];
    let _: () = msg_send![stack, setSpacing: spacing];
    let _: () = msg_send![stack, setTranslatesAutoresizingMaskIntoConstraints: YES];
    stack
}

unsafe fn make_hstack(spacing: f64) -> id {
    let stack: id = msg_send![class!(NSStackView), alloc];
    let stack: id = msg_send![stack, init];
    let _: () = msg_send![stack, setOrientation: NS_USER_INTERFACE_LAYOUT_ORIENTATION_HORIZONTAL];
    let _: () = msg_send![stack, setSpacing: spacing];
    let _: () = msg_send![stack, setTranslatesAutoresizingMaskIntoConstraints: NO];
    stack
}

unsafe fn set_frame(view: id, x: f64, y: f64, w: f64, h: f64) {
    let frame = NSRect::new(NSPoint::new(x, y), NSSize::new(w, h));
    let _: () = msg_send![view, setFrame: frame];
}

unsafe fn make_label(text: &str) -> id {
    let label: id = msg_send![class!(NSTextField), alloc];
    let label: id = msg_send![label, init];
    let str_ = NSString::alloc(nil).init_str(text);
    let _: () = msg_send![label, setStringValue: str_];
    let _: () = msg_send![str_, release];
    let _: () = msg_send![label, setBezeled: NO];
    let _: () = msg_send![label, setEditable: NO];
    let _: () = msg_send![label, setSelectable: NO];
    let _: () = msg_send![label, setDrawsBackground: NO];
    label
}

unsafe fn make_checkbox(title: &str) -> id {
    let btn: id = msg_send![class!(NSButton), alloc];
    let btn: id = msg_send![btn, init];
    let _: () = msg_send![btn, setButtonType: NS_BUTTON_TYPE_SWITCH];
    let title_str = NSString::alloc(nil).init_str(title);
    let _: () = msg_send![btn, setTitle: title_str];
    let _: () = msg_send![title_str, release];
    btn
}

unsafe fn make_popup_ms_row() -> (id, id) {
    let row = make_hstack(8.0);
    let label = make_label("Popup duration (ms):");
    let _: () = msg_send![row, addArrangedSubview: label];

    let field: id = msg_send![class!(NSTextField), alloc];
    let field: id = msg_send![field, init];
    let _: () = msg_send![field, setBezeled: YES];
    let _: () = msg_send![field, setEditable: YES];
    let _: () = msg_send![field, setSelectable: YES];
    let _: () = msg_send![field, setDrawsBackground: YES];
    let placeholder = NSString::alloc(nil).init_str("1000");
    let _: () = msg_send![field, setPlaceholderString: placeholder];
    let _: () = msg_send![placeholder, release];

    let formatter: id = msg_send![class!(NSNumberFormatter), alloc];
    let formatter: id = msg_send![formatter, init];
    let _: () = msg_send![formatter, setAllowsFloats: NO];
    let zero: id = msg_send![class!(NSNumber), numberWithInt: 0i32];
    let _: () = msg_send![formatter, setMinimum: zero];
    let _: () = msg_send![field, setFormatter: formatter];
    let _: () = msg_send![formatter, release];

    pin_width(field, 80.0);

    let _: () = msg_send![row, addArrangedSubview: field];
    (row, field)
}

unsafe fn make_action_row() -> (id, id, id) {
    let row = make_hstack(12.0);

    let cancel_btn: id = msg_send![class!(NSButton), alloc];
    let cancel_btn: id = msg_send![cancel_btn, init];
    let _: () = msg_send![cancel_btn, setBezelStyle: NS_BEZEL_STYLE_ROUNDED];
    let cancel_title = NSString::alloc(nil).init_str("Cancel");
    let _: () = msg_send![cancel_btn, setTitle: cancel_title];
    let _: () = msg_send![cancel_title, release];
    let esc = NSString::alloc(nil).init_str("\u{1b}");
    let _: () = msg_send![cancel_btn, setKeyEquivalent: esc];
    let _: () = msg_send![esc, release];

    let save_btn: id = msg_send![class!(NSButton), alloc];
    let save_btn: id = msg_send![save_btn, init];
    let _: () = msg_send![save_btn, setBezelStyle: NS_BEZEL_STYLE_ROUNDED];
    let save_title = NSString::alloc(nil).init_str("Save");
    let _: () = msg_send![save_btn, setTitle: save_title];
    let _: () = msg_send![save_title, release];
    let cr = NSString::alloc(nil).init_str("\r");
    let _: () = msg_send![save_btn, setKeyEquivalent: cr];
    let _: () = msg_send![cr, release];

    let _: () = msg_send![row, addArrangedSubview: cancel_btn];
    let _: () = msg_send![row, addArrangedSubview: save_btn];

    (row, save_btn, cancel_btn)
}

unsafe fn pin_width(view: id, width: f64) {
    let _: () = msg_send![view, setTranslatesAutoresizingMaskIntoConstraints: NO];
    let c: id = msg_send![class!(NSLayoutConstraint),
        constraintWithItem: view attribute: NS_LAYOUT_ATTRIBUTE_WIDTH relatedBy: 0i64
        toItem: nil attribute: 0i64 multiplier: 1.0_f64 constant: width];
    let _: () = msg_send![view, addConstraint: c];
}
