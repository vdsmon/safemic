/// About dialog: standard macOS mini About panel. Titled NSWindow (real
/// close button), icon + name + version + blurb + "View on GitHub" push
/// button, all semantic colors so it follows the system appearance. Modal
/// via `runModalForWindow:`.
use anyhow::Result;
use cocoa::base::{id, nil, NO, YES};
use cocoa::foundation::{NSData, NSPoint, NSRect, NSSize, NSString};
use objc::declare::ClassDecl;
use objc::runtime::{Class, Object, Sel};
use std::os::raw::c_void;
use std::process::Command;
use std::sync::OnceLock;

const REPO_URL: &str = "https://github.com/vdsmon/safemic";
const APP_ICON_PNG: &[u8] = include_bytes!("../assets/icons/256x256@2x.png");
const OCTOCAT_PNG: &[u8] = include_bytes!("../assets/icons/octocat-32.png");

const WIN_W: f64 = 280.0;
const WIN_H: f64 = 300.0;
const PAD: f64 = 24.0;

// Y positions measured from the top of the content area.
const ICON_SIZE: f64 = 96.0;
const ICON_TOP: f64 = 20.0;
const NAME_SIZE: f64 = 15.0;
const NAME_TOP: f64 = 128.0;
const VERSION_SIZE: f64 = 11.0;
const VERSION_TOP: f64 = 152.0;
const BODY_SIZE: f64 = 11.0;
const BODY_TOP: f64 = 180.0;
const BUTTON_W: f64 = 160.0;
const BUTTON_H: f64 = 32.0;
const BUTTON_BOTTOM: f64 = 20.0;

const FW_REGULAR: f64 = 0.0;
const FW_SEMIBOLD: f64 = 0.3;

// NSTextAlignment values on this AppKit version follow the UIKit numbering:
// 0=Left, 1=Center, 2=Right, 3=Justified, 4=Natural. Tested empirically by
// flipping setAlignment values and observing label position.
const ALIGN_CENTER: u64 = 1;

// NSWindowStyleMask bits.
const STYLE_TITLED_CLOSABLE: u64 = 1 | 2;

pub fn show_about() -> Result<()> {
    let code = unsafe {
        let aw = build_about_window();
        present_about_modal(aw)
    };
    if code == 1 {
        let _ = Command::new("open").arg(REPO_URL).spawn();
    }
    Ok(())
}

pub struct AboutWindow {
    pub window: id,
    pub target_frame: NSRect,
    pub tgt: id,
    pub ctx_ptr: *mut c_void,
}

pub unsafe fn build_about_window() -> AboutWindow {
    let screen: id = msg_send![class!(NSScreen), mainScreen];
    let sf: NSRect = msg_send![screen, frame];
    let ox = sf.origin.x + (sf.size.width - WIN_W) / 2.0;
    let oy = sf.origin.y + (sf.size.height - WIN_H) / 2.0;

    let window: id = msg_send![class!(NSWindow), alloc];
    let window: id = msg_send![window, initWithContentRect: rect(ox, oy - 6.0, WIN_W, WIN_H)
        styleMask: STYLE_TITLED_CLOSABLE backing: 2u64 defer: NO];
    // WIN_W×WIN_H is the CONTENT size; the frame is titlebar-taller. The
    // fade-in target must be a frame rect — reusing the content size there
    // would shrink the content by the titlebar height and shove the
    // bottom-anchored layout up under the titlebar.
    let wf: NSRect = msg_send![window, frame];
    let target = rect(ox, oy, wf.size.width, wf.size.height);
    let _: () = msg_send![window, setReleasedWhenClosed: NO];
    let _: () = msg_send![window, setMovableByWindowBackground: YES];
    let _: () = msg_send![window, setAlphaValue: 0.0_f64];
    // Empty title, like the standard "About <App>" panel. Only the close
    // button is in the style mask; hide minimize/zoom anyway in case AppKit
    // renders them disabled.
    let empty = NSString::alloc(nil).init_str("");
    let _: () = msg_send![window, setTitle: empty];
    let _: () = msg_send![empty, release];
    for btn_kind in [1u64, 2u64] {
        let btn: id = msg_send![window, standardWindowButton: btn_kind];
        if btn != nil {
            let _: () = msg_send![btn, setHidden: YES];
        }
    }

    let content: id = msg_send![window, contentView];
    let content_bounds: NSRect = msg_send![content, bounds];
    let cw = content_bounds.size.width;
    let ch = content_bounds.size.height;

    // Action target. Leaked Box mirrors MMSettingsActions; freed after runModal.
    let ctx_ptr = Box::into_raw(Box::new(())) as *mut c_void;
    let tgt: id = msg_send![actions_class(), alloc];
    let tgt: id = msg_send![tgt, init];
    (*tgt).set_ivar("_ctxPtr", ctx_ptr);
    // Delegate intercepts every close path (stoplight button, Cmd-W) via
    // windowShouldClose: — see window_should_close for why.
    let _: () = msg_send![window, setDelegate: tgt];

    // Helper: convert "y measured from top of content" to bottom-up coords.
    let from_top = |y_top: f64, h: f64| -> f64 { ch - y_top - h };

    // Icon: centered horizontally, near top. The icon is the one place the
    // brand red survives in this window.
    let icon_x = (cw - ICON_SIZE) / 2.0;
    let icon_y = from_top(ICON_TOP, ICON_SIZE);
    let nsdata = NSData::dataWithBytes_length_(
        nil,
        APP_ICON_PNG.as_ptr() as *const c_void,
        APP_ICON_PNG.len() as u64,
    );
    let img: id = msg_send![class!(NSImage), alloc];
    let img: id = msg_send![img, initWithData: nsdata];
    let icon_view: id = msg_send![class!(NSImageView), alloc];
    let icon_view: id =
        msg_send![icon_view, initWithFrame: rect(icon_x, icon_y, ICON_SIZE, ICON_SIZE)];
    let _: () = msg_send![icon_view, setImage: img];
    let _: () = msg_send![img, release];
    let _: () = msg_send![content, addSubview: icon_view];
    let _: () = msg_send![icon_view, release];

    // App name, semibold, centered.
    let name_h = NAME_SIZE + 6.0;
    let name = make_centered_label(
        "SafeMic",
        system_font(NAME_SIZE, FW_SEMIBOLD),
        label_color(),
    );
    set_frame(name, 0.0, from_top(NAME_TOP, name_h), cw, name_h);
    let _: () = msg_send![content, addSubview: name];
    let _: () = msg_send![name, release];

    // Version, secondary, below the name.
    // Source order:
    // 1. SAFEMIC_VERSION env var (set by about-preview's build.rs from
    //    workspace-root Cargo.toml — used in headless sidecar preview).
    // 2. CARGO_PKG_VERSION (correct when compiled as part of the main
    //    safemic crate).
    let version_str = option_env!("SAFEMIC_VERSION").unwrap_or(env!("CARGO_PKG_VERSION"));
    let version_display = format!("Version {version_str}");
    let vh = VERSION_SIZE + 5.0;
    let vl = make_centered_label(
        &version_display,
        system_font(VERSION_SIZE, FW_REGULAR),
        secondary_label_color(),
    );
    set_frame(vl, 0.0, from_top(VERSION_TOP, vh), cw, vh);
    let _: () = msg_send![content, addSubview: vl];
    let _: () = msg_send![vl, release];

    // Body copy: short description, centered, secondary, natural wrapping
    // (three lines at this width).
    let body_text =
        "A simple menu bar app to mute your microphone instantly and keep your conversations private.";
    let body_h = (BODY_SIZE + 4.0) * 3.0;
    let body = make_centered_label(
        body_text,
        system_font(BODY_SIZE, FW_REGULAR),
        secondary_label_color(),
    );
    let _: () = msg_send![body, setUsesSingleLineMode: NO];
    let cell: id = msg_send![body, cell];
    let _: () = msg_send![cell, setWraps: YES];
    set_frame(
        body,
        PAD,
        from_top(BODY_TOP, body_h),
        cw - 2.0 * PAD,
        body_h,
    );
    let _: () = msg_send![content, addSubview: body];
    let _: () = msg_send![body, release];

    // Standard push button "View on GitHub" with a template octocat glyph so
    // the icon tints with the appearance like the title does.
    let ob: id = msg_send![class!(NSButton), alloc];
    let ob: id = msg_send![ob, init];
    // Leading space: imageHugsTitle leaves no gap between glyph and text.
    let title = NSString::alloc(nil).init_str(" View on GitHub");
    let _: () = msg_send![ob, setTitle: title];
    let _: () = msg_send![title, release];
    let _: () = msg_send![ob, setBezelStyle: 1u64]; // NSBezelStyleRounded (push)
    let _: () = msg_send![ob, setTarget: tgt];
    let _: () = msg_send![ob, setAction: sel!(openGitHubAction:)];
    let cat_data = NSData::dataWithBytes_length_(
        nil,
        OCTOCAT_PNG.as_ptr() as *const c_void,
        OCTOCAT_PNG.len() as u64,
    );
    let cat_img: id = msg_send![class!(NSImage), alloc];
    let cat_img: id = msg_send![cat_img, initWithData: cat_data];
    let _: () = msg_send![cat_img, setSize: NSSize::new(14.0, 14.0)];
    let _: () = msg_send![cat_img, setTemplate: YES];
    let _: () = msg_send![ob, setImage: cat_img];
    let _: () = msg_send![ob, setImagePosition: 2_u64]; // NSImageLeft
                                                        // Without this the glyph pins to the bezel's left edge and the title
                                                        // centers alone, leaving the pair visually off-center.
    let _: () = msg_send![ob, setImageHugsTitle: YES];
    let _: () = msg_send![cat_img, release];
    set_frame(ob, (cw - BUTTON_W) / 2.0, BUTTON_BOTTOM, BUTTON_W, BUTTON_H);
    let enter = NSString::alloc(nil).init_str("\r");
    let _: () = msg_send![ob, setKeyEquivalent: enter];
    let _: () = msg_send![ob, setKeyEquivalentModifierMask: 0u64];
    let _: () = msg_send![enter, release];
    let _: () = msg_send![content, addSubview: ob];
    let _: () = msg_send![ob, release];

    // Invisible Escape button: closes the panel like the red button does.
    let esc_btn: id = msg_send![class!(NSButton), alloc];
    let esc_btn: id = msg_send![esc_btn, init];
    let _: () = msg_send![esc_btn, setBordered: NO];
    let _: () = msg_send![esc_btn, setTransparent: YES];
    let esc = NSString::alloc(nil).init_str("\u{1b}");
    let _: () = msg_send![esc_btn, setKeyEquivalent: esc];
    let _: () = msg_send![esc, release];
    let _: () = msg_send![esc_btn, setTarget: tgt];
    let _: () = msg_send![esc_btn, setAction: sel!(closeAction:)];
    set_frame(esc_btn, -10.0, -10.0, 1.0, 1.0);
    let _: () = msg_send![content, addSubview: esc_btn];
    let _: () = msg_send![esc_btn, release];

    AboutWindow {
        window,
        target_frame: target,
        tgt,
        ctx_ptr,
    }
}

pub unsafe fn present_about_modal(aw: AboutWindow) -> i64 {
    let AboutWindow {
        window,
        target_frame: target,
        tgt,
        ctx_ptr,
    } = aw;
    let app: id = msg_send![class!(NSApplication), sharedApplication];
    let _: () = msg_send![window, makeKeyAndOrderFront: nil];
    let _: () = msg_send![app, activateIgnoringOtherApps: YES];
    let actx = class!(NSAnimationContext);
    let _: () = msg_send![actx, beginGrouping];
    let cur: id = msg_send![actx, currentContext];
    let _: () = msg_send![cur, setDuration: 0.18_f64];
    let anim: id = msg_send![window, animator];
    let _: () = msg_send![anim, setAlphaValue: 1.0_f64];
    let _: () = msg_send![anim, setFrame: target display: YES];
    let _: () = msg_send![actx, endGrouping];
    let code: i64 = msg_send![app, runModalForWindow: window];
    let _: () = msg_send![window, orderOut: nil];
    let _: () = msg_send![window, release];
    drop(Box::from_raw(ctx_ptr as *mut ()));
    let _: () = msg_send![tgt, release];
    code
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}
unsafe fn set_frame(view: id, x: f64, y: f64, w: f64, h: f64) {
    let _: () = msg_send![view, setFrame: rect(x, y, w, h)];
}
unsafe fn label_color() -> id {
    msg_send![class!(NSColor), labelColor]
}
unsafe fn secondary_label_color() -> id {
    msg_send![class!(NSColor), secondaryLabelColor]
}
unsafe fn system_font(size: f64, weight: f64) -> id {
    msg_send![class!(NSFont), systemFontOfSize: size weight: weight]
}

unsafe fn make_centered_label(text: &str, font: id, color: id) -> id {
    let label: id = msg_send![class!(NSTextField), alloc];
    let label: id = msg_send![label, init];
    let _: () = msg_send![label, setBezeled: NO];
    let _: () = msg_send![label, setBordered: NO];
    let _: () = msg_send![label, setEditable: NO];
    let _: () = msg_send![label, setDrawsBackground: NO];
    let _: () = msg_send![label, setSelectable: NO];
    let _: () = msg_send![label, setFont: font];
    let _: () = msg_send![label, setTextColor: color];
    let s = NSString::alloc(nil).init_str(text);
    let _: () = msg_send![label, setStringValue: s];
    let _: () = msg_send![s, release];
    // Setting alignment AFTER stringValue ensures the cell's paragraph style
    // for the plain string path picks up the right alignment.
    let _: () = msg_send![label, setAlignment: ALIGN_CENTER];
    label
}

unsafe fn stop_modal(code: i64) {
    let app: id = msg_send![class!(NSApplication), sharedApplication];
    let _: () = msg_send![app, stopModalWithCode: code];
}
extern "C" fn close_action(_t: &Object, _c: Sel, _s: *mut Object) {
    unsafe { stop_modal(0) }
}
extern "C" fn open_github_action(_t: &Object, _c: Sel, _s: *mut Object) {
    unsafe { stop_modal(1) }
}

fn actions_class() -> &'static Class {
    static CLS: OnceLock<&'static Class> = OnceLock::new();
    CLS.get_or_init(|| {
        let mut decl = ClassDecl::new("MMAboutActions", class!(NSObject))
            .expect("MMAboutActions registered twice");
        decl.add_ivar::<*mut c_void>("_ctxPtr");
        unsafe {
            decl.add_method(
                sel!(closeAction:),
                close_action as extern "C" fn(&Object, Sel, *mut Object),
            );
            decl.add_method(
                sel!(openGitHubAction:),
                open_github_action as extern "C" fn(&Object, Sel, *mut Object),
            );
            decl.add_method(
                sel!(windowShouldClose:),
                window_should_close as extern "C" fn(&Object, Sel, *mut Object) -> bool,
            );
        }
        decl.register()
    })
}

/// Closing the window during `runModalForWindow:` would leave the modal
/// session spinning forever with no window on screen (the app then reads as
/// hung — the stoplight button does NOT go through `performClose:`, so an
/// override there never fires). Deny the close and end the modal instead;
/// `present_about_modal` orders the window out after `runModal` returns.
extern "C" fn window_should_close(_t: &Object, _c: Sel, _sender: *mut Object) -> bool {
    unsafe { stop_modal(0) };
    false
}
