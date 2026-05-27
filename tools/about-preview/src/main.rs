// Permit unused API in the included about.rs (some helpers are only called by
// present_about_modal, which the sidecar deliberately does not invoke).
#![allow(dead_code)]

//! Standalone About-window iteration sidecar.
//!
//! Hosts only `src/about.rs` plus a tiny snapshot harness. Changes to
//! about.rs flow into this binary automatically (single source of truth).
//!
//! Run with `cargo run -p about-preview --target aarch64-apple-darwin` or
//! via the wrapper at `tools/about-preview/iterate.sh` which captures + SSIM.

#[macro_use]
extern crate objc;

use anyhow::Result;
use cocoa::base::{id, nil, YES};
use cocoa::foundation::{NSRect, NSString};
use log::info;

// Pull the real layout code in verbatim. include_bytes! paths in about.rs are
// resolved relative to the original src/about.rs file, so the icon include
// still works through the #[path] indirection.
#[path = "../../../src/about.rs"]
mod about;

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_timestamp(None)
        .init();

    // NSApplication needs to exist + be activated as a regular app before any
    // window will show up on screen. tao normally handles this; the sidecar
    // does it directly to keep the dep tree small.
    unsafe {
        let app: id = msg_send![class!(NSApplication), sharedApplication];
        // NSApplicationActivationPolicyRegular = 0
        let _: () = msg_send![app, setActivationPolicy: 0i64];
        let _: bool = msg_send![app, finishLaunching];

        let aw = about::build_about_window();
        let _: () = msg_send![aw.window, makeKeyAndOrderFront: nil];
        let _: () = msg_send![app, activateIgnoringOtherApps: YES];
        // Apply the target frame and full opacity directly (no animator —
        // sidecar snapshots a static state, not the fade-in transition).
        let _: () = msg_send![aw.window, setAlphaValue: 1.0_f64];
        let _: () = msg_send![aw.window, setFrame: aw.target_frame display: YES];

        let settle_ms: u64 = std::env::var("SAFEMIC_PREVIEW_SETTLE_MS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(220);
        pump_run_loop_ms(settle_ms);

        let snapshot_path = std::env::var("SAFEMIC_PREVIEW_SNAPSHOT").ok();
        if let Some(path) = snapshot_path {
            snapshot_window_to_png(aw.window, &path)?;
            info!("snapshot -> {path}");
        }
    }

    std::process::exit(0);
}

/// Mirror of `SettingsWindow::snapshot_to_png` from src/settings_window.rs.
/// Uses `bitmapImageRepForCachingDisplayInRect:` so no Screen Recording grant
/// is required and the snapshot does not go through the window server.
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
