use crate::settings::ShortcutConfig;
use cocoa::base::{id, nil};
use cocoa::foundation::NSString;
use libc::c_void;
use std::sync::{Arc, RwLock};
use tao::window::Theme;

type CGFloat = f64;

#[repr(C)]
struct CGPoint {
    pub x: CGFloat,
    pub y: CGFloat,
}

extern "C" {
    fn CFRelease(cf: *const c_void);
    fn CGEventCreate(r: *const c_void) -> *const c_void;
    fn CGEventGetLocation(e: *const c_void) -> CGPoint;
}

pub fn get_cursor_pos() -> Option<(f64, f64)> {
    unsafe {
        let event = CGEventCreate(std::ptr::null());
        if event.is_null() {
            return None;
        }

        let point = CGEventGetLocation(event);
        CFRelease(event);
        Some((point.x, point.y))
    }
}

pub fn arc_lock<T>(value: T) -> Arc<RwLock<T>> {
    let rwlock = RwLock::new(value);
    Arc::new(rwlock)
}

/// The OS-level appearance, read from user defaults rather than a window's
/// effective theme. The menu bar always follows the OS setting even when an
/// NSApp appearance override is active, so the tray icon color must come
/// from here.
pub fn system_theme() -> Theme {
    unsafe {
        let defaults: id = msg_send![class!(NSUserDefaults), standardUserDefaults];
        let key = NSString::alloc(nil).init_str("AppleInterfaceStyle");
        // "Dark" when dark mode is on; absent in light mode.
        let style: id = msg_send![defaults, stringForKey: key];
        let _: () = msg_send![key, release];
        if style == nil {
            Theme::Light
        } else {
            Theme::Dark
        }
    }
}

/// Render a `ShortcutConfig` as a single string like `⇧⌘A`.
pub fn format_shortcut(config: &ShortcutConfig) -> String {
    let mut parts = vec![];
    for modifier in &config.modifiers {
        match modifier.as_str() {
            "shift" => parts.push("⇧"),
            "meta" | "cmd" | "command" => parts.push("⌘"),
            "ctrl" | "control" => parts.push("⌃"),
            "alt" | "option" => parts.push("⌥"),
            _ => {}
        }
    }
    parts.push(config.key.as_str());
    parts.join("")
}
