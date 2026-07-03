// Permit unused API in the included popup_content.rs (some methods are only
// called by the full safemic popup, not by the sidecar's main).
#![allow(dead_code)]

//! Standalone Popup-bezel iteration sidecar.
//!
//! Hosts only `src/popup_content.rs` plus a tiny snapshot harness. Changes to
//! popup_content.rs flow into this binary automatically (single source of
//! truth). Snapshots render in-process, so they need no Screen Recording
//! grant and are unaffected by the release popup's content protection.
//! Caveat: `cacheDisplayInRect:` renders the vibrancy material's tint plate
//! without the live backdrop blur — good for layout/tint/radius regression,
//! not for judging the blur itself.

#[macro_use]
extern crate objc;

use anyhow::Result;
use cocoa::base::{id, nil, NO, YES};
use cocoa::foundation::{NSPoint, NSRect, NSSize, NSString};
use log::info;
use tao::dpi::LogicalSize;

#[path = "../../../src/popup_content.rs"]
mod popup_content;

use popup_content::{PopupContent, BEZEL_SIZE};

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_timestamp(None)
        .init();

    let muted = match std::env::var("SAFEMIC_PREVIEW_STATE").as_deref() {
        Ok("unmuted") => false,
        Ok("muted") | Ok("") | Err(_) => true,
        Ok(other) => {
            log::error!("unknown SAFEMIC_PREVIEW_STATE: {other}");
            std::process::exit(1);
        }
    };

    unsafe {
        let app: id = msg_send![class!(NSApplication), sharedApplication];
        // NSApplicationActivationPolicyRegular = 0
        let _: () = msg_send![app, setActivationPolicy: 0i64];
        let _: bool = msg_send![app, finishLaunching];

        let frame = NSRect::new(
            NSPoint::new(400.0, 400.0),
            NSSize::new(BEZEL_SIZE, BEZEL_SIZE),
        );
        let window: id = msg_send![class!(NSWindow), alloc];
        // styleMask 0 = borderless, backing 2 = buffered
        let window: id = msg_send![window, initWithContentRect: frame
            styleMask: 0u64 backing: 2u64 defer: NO];
        let _: () = msg_send![window, setReleasedWhenClosed: NO];
        let clear: id = msg_send![class!(NSColor), clearColor];
        let _: () = msg_send![window, setOpaque: NO];
        let _: () = msg_send![window, setBackgroundColor: clear];

        let appearance = std::env::var("SAFEMIC_PREVIEW_APPEARANCE").unwrap_or_default();
        let appearance_name = match appearance.as_str() {
            "light" => "NSAppearanceNameAqua",
            "" | "dark" => "NSAppearanceNameDarkAqua",
            other => {
                log::error!("unknown SAFEMIC_PREVIEW_APPEARANCE: {other}");
                std::process::exit(1);
            }
        };
        let ns_name = NSString::alloc(nil).init_str(appearance_name);
        let ns_appearance: id = msg_send![class!(NSAppearance), appearanceNamed: ns_name];
        let _: () = msg_send![ns_name, release];
        if ns_appearance != nil {
            let _: () = msg_send![window, setAppearance: ns_appearance];
        }

        let content = PopupContent::new(muted, LogicalSize::new(BEZEL_SIZE, BEZEL_SIZE))?;
        let _: () = msg_send![window, setContentView: content.view];
        let _: () = msg_send![window, makeKeyAndOrderFront: nil];
        let _: () = msg_send![app, activateIgnoringOtherApps: YES];

        let settle_ms: u64 = std::env::var("SAFEMIC_PREVIEW_SETTLE_MS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(220);
        pump_run_loop_ms(settle_ms);

        if let Ok(path) = std::env::var("SAFEMIC_PREVIEW_SNAPSHOT") {
            snapshot_window_to_png(window, &path)?;
            info!("snapshot -> {path}");
        }
    }

    std::process::exit(0);
}

/// Mirror of the settings/about sidecar snapshot: renders the window's view
/// tree via `bitmapImageRepForCachingDisplayInRect:` — no window server, no
/// Screen Recording permission.
unsafe fn snapshot_window_to_png(ns_window: id, path: &str) -> Result<()> {
    let content_view: id = msg_send![ns_window, contentView];
    if content_view == nil {
        return Err(anyhow::anyhow!("no content view"));
    }
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
    Ok(())
}

fn pump_run_loop_ms(ms: u64) {
    use objc::runtime::Object;
    let secs = ms as f64 / 1000.0;
    unsafe {
        let date: *mut Object = msg_send![class!(NSDate), dateWithTimeIntervalSinceNow: secs];
        let run_loop: *mut Object = msg_send![class!(NSRunLoop), currentRunLoop];
        let _: bool = msg_send![run_loop, runUntilDate: date];
    }
}
