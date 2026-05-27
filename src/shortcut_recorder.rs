/// Modal shortcut recorder. Installs a transparent `MMShortcutRecorderView`
/// over the chip, makes it firstResponder of the settings window, then
/// captures the first `keyDown:` event and invokes the on_capture callback
/// with the parsed `ShortcutConfig`. `cancelOperation:` (Escape) fires the
/// callback with `None`.
///
/// Only one recorder may be active at a time, guarded by a module-level
/// `OnceLock<Mutex<Option<RecorderState>>>`. Re-entering tears down the
/// previous recorder cleanly so an abandoned recorder can't leak a view
/// or steal subsequent keystrokes.
use crate::settings::ShortcutConfig;
use cocoa::base::{id, nil};
use cocoa::foundation::NSRect;
use objc::declare::ClassDecl;
use objc::runtime::{Class, Object, Sel};
use std::os::raw::c_void;
use std::sync::{Mutex, OnceLock};

// NSEventModifierFlags raw bits, restricted to the "real" set we honor.
const NS_SHIFT: u64 = 1 << 17;
const NS_CONTROL: u64 = 1 << 18;
const NS_ALT: u64 = 1 << 19;
const NS_COMMAND: u64 = 1 << 20;

/// Recorder outcome. `Cancelled` covers Escape, programmatic teardown, and
/// unmappable keystrokes. `MissingModifier` is a bare letter/digit that
/// would steal regular typing app-wide.
pub enum CaptureResult {
    Captured(ShortcutConfig),
    Cancelled,
    MissingModifier,
}

/// The recorder API is single-threaded by construction; the callback is
/// always invoked on the main thread (NSView keyDown delivery, or the same
/// thread that calls `cancel_recording`). The `Send` bound is dropped so
/// callers can capture `id` pointers without dancing around marker traits.
type CaptureCallback = Box<dyn FnOnce(CaptureResult)>;

struct RecorderState {
    view: id,
    previous_responder: id,
    /// Box leaked into the view's ivar. Pulled back out and dropped on
    /// teardown so the callback FnOnce is invoked exactly once.
    callback_ptr: *mut c_void,
}

// The state is only ever accessed from the main thread, but Mutex<Option<_>>
// requires `Send`. The pointers inside are valid only on the main thread,
// which is where every entry point runs.
unsafe impl Send for RecorderState {}

fn state_slot() -> &'static Mutex<Option<RecorderState>> {
    static SLOT: OnceLock<Mutex<Option<RecorderState>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

/// Start recording. `window` is the settings NSWindow, `frame` is the desired
/// recorder-view frame in the content view's coordinate space (typically the
/// chip's frame). `on_capture` runs on the main thread with `Some(combo)` on
/// successful capture or `None` on Escape / pre-empted teardown.
pub fn start_recording(window: id, frame: NSRect, on_capture: CaptureCallback) {
    // Tear down any prior recorder first; its callback fires with None.
    cancel_recording();
    unsafe {
        let content_view: id = msg_send![window, contentView];
        let previous_responder: id = msg_send![window, firstResponder];

        let view: id = msg_send![recorder_class(), alloc];
        let view: id = msg_send![view, initWithFrame: frame];
        let callback_box: Box<CaptureCallback> = Box::new(on_capture);
        let callback_ptr = Box::into_raw(callback_box) as *mut c_void;
        (*view).set_ivar("_cbPtr", callback_ptr);

        let _: () = msg_send![content_view, addSubview: view];
        let _: () = msg_send![window, makeFirstResponder: view];

        *state_slot().lock().unwrap() = Some(RecorderState {
            view,
            previous_responder,
            callback_ptr,
        });
    }
}

/// Tear down the active recorder, if any. Fires its callback with `Cancelled`.
pub fn cancel_recording() {
    let prev = state_slot().lock().unwrap().take();
    if let Some(state) = prev {
        finish(state, CaptureResult::Cancelled);
    }
}

/// Internal: drain the recorder state and invoke the stored callback.
fn finish(state: RecorderState, result: CaptureResult) {
    unsafe {
        let window: id = msg_send![state.view, window];
        let _: () = msg_send![state.view, removeFromSuperview];
        if window != nil && state.previous_responder != nil {
            let _: () = msg_send![window, makeFirstResponder: state.previous_responder];
        }
        // Reclaim the callback box and clear the ivar to make double-drop impossible.
        (*state.view).set_ivar("_cbPtr", std::ptr::null_mut::<c_void>());
        let cb: Box<CaptureCallback> = Box::from_raw(state.callback_ptr as *mut CaptureCallback);
        cb(result);
    }
}

// ---------------------------------------------------------------------------
// MMShortcutRecorderView — NSView subclass that overrides keyDown/flagsChanged
// to capture the next keystroke and report it back via the stored callback.
// ---------------------------------------------------------------------------

extern "C" fn accepts_first_responder(_: &Object, _: Sel) -> bool {
    true
}

extern "C" fn key_down(this: &Object, _: Sel, event: *mut Object) {
    unsafe {
        let modifier_flags: u64 = msg_send![event, modifierFlags];
        let key_code: u16 = msg_send![event, keyCode];
        // Escape (kVK_Escape = 53) → treat as cancel.
        if key_code == 53 {
            take_and_finish(this, CaptureResult::Cancelled);
            return;
        }
        let chars: id = msg_send![event, charactersIgnoringModifiers];
        let key_label = key_label_from_code(key_code).or_else(|| key_label_from_chars(chars));
        let result = match key_label {
            None => CaptureResult::Cancelled,
            Some(key) => classify(modifier_flags, key),
        };
        take_and_finish(this, result);
    }
}

fn classify(modifier_flags: u64, key: String) -> CaptureResult {
    let mut modifiers = vec![];
    if modifier_flags & NS_SHIFT != 0 {
        modifiers.push("shift".to_string());
    }
    if modifier_flags & NS_CONTROL != 0 {
        modifiers.push("ctrl".to_string());
    }
    if modifier_flags & NS_ALT != 0 {
        modifiers.push("alt".to_string());
    }
    if modifier_flags & NS_COMMAND != 0 {
        modifiers.push("meta".to_string());
    }
    if modifiers.is_empty() && key.len() == 1 && key.chars().next().unwrap().is_ascii_alphanumeric()
    {
        return CaptureResult::MissingModifier;
    }
    CaptureResult::Captured(ShortcutConfig { modifiers, key })
}

extern "C" fn flags_changed(_: &Object, _: Sel, _event: *mut Object) {
    // Pure modifier presses don't capture. The user must press a real key.
}

extern "C" fn cancel_operation(this: &Object, _: Sel, _sender: *mut Object) {
    take_and_finish(this, CaptureResult::Cancelled);
}

/// Pull the active recorder state out of the slot, only if the firing view
/// is the one we registered. Avoids a stale view racing teardown.
fn take_and_finish(view: &Object, result: CaptureResult) {
    let view_ptr = view as *const Object as id;
    let mut slot = state_slot().lock().unwrap();
    let matches = slot.as_ref().map(|s| s.view == view_ptr).unwrap_or(false);
    if !matches {
        return;
    }
    let state = slot.take().unwrap();
    drop(slot);
    finish(state, result);
}

/// Map Apple `kVK_*` virtual key codes to the labels global-hotkey understands.
/// Covers letters, digits, F1-F20, arrows, space, tab — the practical set for
/// a single-shortcut app. Punctuation falls back to charactersIgnoringModifiers.
fn key_label_from_code(code: u16) -> Option<String> {
    let s = match code {
        0 => "A",
        1 => "S",
        2 => "D",
        3 => "F",
        4 => "H",
        5 => "G",
        6 => "Z",
        7 => "X",
        8 => "C",
        9 => "V",
        11 => "B",
        12 => "Q",
        13 => "W",
        14 => "E",
        15 => "R",
        16 => "Y",
        17 => "T",
        29 => "0",
        18 => "1",
        19 => "2",
        20 => "3",
        21 => "4",
        23 => "5",
        22 => "6",
        26 => "7",
        28 => "8",
        25 => "9",
        31 => "O",
        32 => "U",
        34 => "I",
        35 => "P",
        37 => "L",
        38 => "J",
        40 => "K",
        45 => "N",
        46 => "M",
        49 => "Space",
        48 => "Tab",
        36 => "Enter",
        51 => "Backspace",
        117 => "Delete",
        115 => "Home",
        119 => "End",
        116 => "PageUp",
        121 => "PageDown",
        123 => "ArrowLeft",
        124 => "ArrowRight",
        125 => "ArrowDown",
        126 => "ArrowUp",
        122 => "F1",
        120 => "F2",
        99 => "F3",
        118 => "F4",
        96 => "F5",
        97 => "F6",
        98 => "F7",
        100 => "F8",
        101 => "F9",
        109 => "F10",
        103 => "F11",
        111 => "F12",
        105 => "F13",
        107 => "F14",
        113 => "F15",
        106 => "F16",
        64 => "F17",
        79 => "F18",
        80 => "F19",
        90 => "F20",
        _ => return None,
    };
    Some(s.to_string())
}

fn key_label_from_chars(chars: id) -> Option<String> {
    if chars == nil {
        return None;
    }
    unsafe {
        let len: usize = msg_send![chars, length];
        if len == 0 {
            return None;
        }
        let utf8: *const i8 = msg_send![chars, UTF8String];
        if utf8.is_null() {
            return None;
        }
        let s = std::ffi::CStr::from_ptr(utf8)
            .to_string_lossy()
            .into_owned();
        // Strip Unicode-PUA function-key sentinels; the keyCode path covers them.
        if s.chars().any(|c| (c as u32) >= 0xF700) {
            return None;
        }
        Some(s.to_uppercase())
    }
}

fn recorder_class() -> &'static Class {
    static CLS: OnceLock<&'static Class> = OnceLock::new();
    CLS.get_or_init(|| {
        let mut decl = ClassDecl::new("MMShortcutRecorderView", class!(NSView))
            .expect("MMShortcutRecorderView registered twice");
        decl.add_ivar::<*mut c_void>("_cbPtr");
        unsafe {
            decl.add_method(
                sel!(acceptsFirstResponder),
                accepts_first_responder as extern "C" fn(&Object, Sel) -> bool,
            );
            decl.add_method(
                sel!(keyDown:),
                key_down as extern "C" fn(&Object, Sel, *mut Object),
            );
            decl.add_method(
                sel!(flagsChanged:),
                flags_changed as extern "C" fn(&Object, Sel, *mut Object),
            );
            decl.add_method(
                sel!(cancelOperation:),
                cancel_operation as extern "C" fn(&Object, Sel, *mut Object),
            );
        }
        decl.register()
    })
}
