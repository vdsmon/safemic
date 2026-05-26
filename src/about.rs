/// About dialog: borderless NSWindow, editorial typographic, dark blur,
/// fade-in. Modal via `runModalForWindow:`.
use anyhow::Result;
use cocoa::base::{id, nil, NO, YES};
use cocoa::foundation::{NSData, NSPoint, NSRange, NSRect, NSSize, NSString};
use objc::declare::ClassDecl;
use objc::runtime::{Class, Object, Sel};
use std::os::raw::c_void;
use std::process::Command;
use std::sync::OnceLock;

const REPO_URL: &str = "https://github.com/vdsmon/mic-mute";
const REPO_DISPLAY: &str = "github.com/vdsmon/mic-mute";
const APP_ICON_PNG: &[u8] = include_bytes!("../assets/icons/256x256@2x.png");

const WIN_W: f64 = 380.0;
const WIN_H: f64 = 220.0;
const MARGIN: f64 = 24.0;
const ICON: f64 = 64.0;
// NSFontWeight raw CGFloat constants.
const FW_REGULAR: f64 = 0.0;
const FW_MEDIUM: f64 = 0.23;
const FW_SEMIBOLD: f64 = 0.3;
// NSFontDescriptorSystemDesignSerif (publicly-exported NSString constant).
const NS_FONT_DESCRIPTOR_SYSTEM_DESIGN_SERIF: &str = "NSCTFontUIFontDesignSerif";

pub fn show_about() -> Result<()> {
    let code = unsafe { run_about_modal() };
    if code == 1 {
        let _ = Command::new("open").arg(REPO_URL).spawn();
    }
    Ok(())
}

unsafe fn run_about_modal() -> i64 {
    let app: id = msg_send![class!(NSApplication), sharedApplication];
    let screen: id = msg_send![class!(NSScreen), mainScreen];
    let sf: NSRect = msg_send![screen, frame];
    let ox = sf.origin.x + (sf.size.width - WIN_W) / 2.0;
    let oy = sf.origin.y + (sf.size.height - WIN_H) / 2.0;
    let target = rect(ox, oy, WIN_W, WIN_H);
    // Borderless transparent NSWindow with rounded corner clip on contentView.
    // MMAboutWindow overrides canBecomeKeyWindow so a borderless window can
    // still receive keyboard input (Enter / Escape default actions).
    let window: id = msg_send![about_window_class(), alloc];
    let window: id = msg_send![window, initWithContentRect: rect(ox, oy - 6.0, WIN_W, WIN_H)
        styleMask: 0u64 backing: 2u64 defer: NO];
    // Keep the window alive across `close` so we can drain animations and
    // release explicitly below; without this, AppKit deallocs it on close.
    let _: () = msg_send![window, setReleasedWhenClosed: NO];
    let clear: id = msg_send![class!(NSColor), clearColor];
    let _: () = msg_send![window, setOpaque: NO];
    let _: () = msg_send![window, setBackgroundColor: clear];
    let _: () = msg_send![window, setMovableByWindowBackground: YES];
    let _: () = msg_send![window, setHasShadow: YES];
    let _: () = msg_send![window, setAlphaValue: 0.0_f64];
    let content_rect = rect(0.0, 0.0, WIN_W, WIN_H);
    let content: id = msg_send![class!(NSView), alloc];
    let content: id = msg_send![content, initWithFrame: content_rect];
    let _: () = msg_send![content, setWantsLayer: YES];
    let layer: id = msg_send![content, layer];
    let _: () = msg_send![layer, setCornerRadius: 12.0_f64];
    let _: () = msg_send![layer, setMasksToBounds: YES];
    // NSVisualEffectMaterial.hudWindow=12, state.active=1, behindWindow=0, autoresize W|H=18.
    let blur: id = msg_send![class!(NSVisualEffectView), alloc];
    let blur: id = msg_send![blur, initWithFrame: content_rect];
    let _: () = msg_send![blur, setMaterial: 12i64];
    let _: () = msg_send![blur, setState: 1i64];
    let _: () = msg_send![blur, setBlendingMode: 0i64];
    let _: () = msg_send![blur, setAutoresizingMask: 18u64];
    let _: () = msg_send![content, addSubview: blur];
    let _: () = msg_send![blur, release];
    let icon_y = WIN_H - MARGIN - ICON;
    let nsdata = NSData::dataWithBytes_length_(
        nil,
        APP_ICON_PNG.as_ptr() as *const c_void,
        APP_ICON_PNG.len() as u64,
    );
    let img: id = msg_send![class!(NSImage), alloc];
    let img: id = msg_send![img, initWithData: nsdata];
    let icon_view: id = msg_send![class!(NSImageView), alloc];
    let icon_view: id = msg_send![icon_view, initWithFrame: rect(MARGIN, icon_y, ICON, ICON)];
    let _: () = msg_send![icon_view, setImage: img];
    let _: () = msg_send![img, release];
    let _: () = msg_send![content, addSubview: icon_view];
    let _: () = msg_send![icon_view, release];
    let wm_x = MARGIN + ICON + 16.0;
    let wm_y = icon_y + ICON / 2.0 - 13.0;
    let tw = WIN_W - wm_x - MARGIN;
    let wm = make_label(
        "Mic Mute",
        make_serif_font(28.0, FW_SEMIBOLD),
        label_color(),
        -0.5,
        false,
        None,
    );
    set_frame(wm, wm_x, wm_y, tw, 34.0);
    let _: () = msg_send![content, addSubview: wm];
    let _: () = msg_send![wm, release];
    let ver = format!("v{}", env!("CARGO_PKG_VERSION"));
    let vl = make_label(
        &ver,
        mono_digit_font(13.0, FW_REGULAR),
        secondary_color(),
        0.0,
        false,
        None,
    );
    set_frame(vl, wm_x, wm_y - 22.0, tw, 18.0);
    let _: () = msg_send![content, addSubview: vl];
    let _: () = msg_send![vl, release];
    let link = make_label(
        REPO_DISPLAY,
        system_font(12.0, FW_REGULAR),
        accent_color(),
        0.0,
        true,
        Some(REPO_URL),
    );
    set_frame(link, MARGIN, MARGIN, 220.0, 18.0);
    let _: () = msg_send![content, addSubview: link];
    let _: () = msg_send![link, release];
    // Action target. Leaked Box mirrors MMSettingsActions; freed after runModal.
    let ctx_ptr = Box::into_raw(Box::new(())) as *mut c_void;
    let tgt: id = msg_send![actions_class(), alloc];
    let tgt: id = msg_send![tgt, init];
    (*tgt).set_ivar("_ctxPtr", ctx_ptr);
    let btn_font = system_font(13.0, FW_MEDIUM);
    let open_x = WIN_W - MARGIN - 120.0;
    let ob = make_button("Open GitHub", btn_font, true, tgt, sel!(openGitHubAction:));
    set_frame(ob, open_x, MARGIN, 120.0, 28.0);
    // Default action — Enter opens GitHub.
    let enter = NSString::alloc(nil).init_str("\r");
    let _: () = msg_send![ob, setKeyEquivalent: enter];
    let _: () = msg_send![ob, setKeyEquivalentModifierMask: 0u64];
    let _: () = msg_send![enter, release];
    let _: () = msg_send![content, addSubview: ob];
    let _: () = msg_send![ob, release];
    let cb = make_button("Close", btn_font, false, tgt, sel!(closeAction:));
    set_frame(cb, open_x - 76.0, MARGIN, 64.0, 28.0);
    let esc = NSString::alloc(nil).init_str("\u{1b}");
    let _: () = msg_send![cb, setKeyEquivalent: esc];
    let _: () = msg_send![esc, release];
    let _: () = msg_send![content, addSubview: cb];
    let _: () = msg_send![cb, release];
    let _: () = msg_send![window, setContentView: content];
    let _: () = msg_send![content, release];
    let _: () = msg_send![window, makeKeyAndOrderFront: nil];
    let _: () = msg_send![app, activateIgnoringOtherApps: YES];
    // Fade in 180ms with 6pt rise.
    let actx = class!(NSAnimationContext);
    let _: () = msg_send![actx, beginGrouping];
    let cur: id = msg_send![actx, currentContext];
    let _: () = msg_send![cur, setDuration: 0.18_f64];
    let anim: id = msg_send![window, animator];
    let _: () = msg_send![anim, setAlphaValue: 1.0_f64];
    let _: () = msg_send![anim, setFrame: target display: YES];
    let _: () = msg_send![actx, endGrouping];
    let code: i64 = msg_send![app, runModalForWindow: window];
    // setReleasedWhenClosed:NO means the window holds its +1 from alloc/init,
    // so we orderOut (not close — that would still tear down the responder
    // chain mid-animation) and release explicitly. By this point the fade-in
    // animation has been pumped to completion by the modal run loop.
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
unsafe fn secondary_color() -> id {
    msg_send![class!(NSColor), secondaryLabelColor]
}
unsafe fn accent_color() -> id {
    msg_send![class!(NSColor),
        colorWithSRGBRed: 0.949_f64 green: 0.404_f64 blue: 0.373_f64 alpha: 1.0_f64]
}
unsafe fn system_font(size: f64, weight: f64) -> id {
    msg_send![class!(NSFont), systemFontOfSize: size weight: weight]
}
unsafe fn mono_digit_font(size: f64, weight: f64) -> id {
    msg_send![class!(NSFont), monospacedDigitSystemFontOfSize: size weight: weight]
}

// New York via fontDescriptorWithDesign: NSFontDescriptorSystemDesignSerif.
// Mirrors settings_window's wordmark path. Falls back to system font when
// descriptor resolution returns nil (pre-10.15).
unsafe fn make_serif_font(size: f64, weight: f64) -> id {
    let base: id = system_font(size, weight);
    let desc: id = msg_send![base, fontDescriptor];
    let key = NSString::alloc(nil).init_str(NS_FONT_DESCRIPTOR_SYSTEM_DESIGN_SERIF);
    let new_desc: id = msg_send![desc, fontDescriptorWithDesign: key];
    let _: () = msg_send![key, release];
    if new_desc == nil {
        return base;
    }
    let serif: id = msg_send![class!(NSFont), fontWithDescriptor: new_desc size: size];
    if serif == nil {
        base
    } else {
        serif
    }
}

unsafe fn make_label(
    text: &str,
    font: id,
    color: id,
    kern: f64,
    link: bool,
    url: Option<&str>,
) -> id {
    let label: id = msg_send![class!(NSTextField), alloc];
    let label: id = msg_send![label, init];
    let _: () = msg_send![label, setBezeled: NO];
    let _: () = msg_send![label, setEditable: NO];
    let _: () = msg_send![label, setDrawsBackground: NO];
    let _: () = msg_send![label, setSelectable: if link { YES } else { NO }];
    let _: () = msg_send![label, setFont: font];
    let _: () = msg_send![label, setTextColor: color];
    if link {
        let _: () = msg_send![label, setAllowsEditingTextAttributes: YES];
    }
    let k = if kern != 0.0 { Some(kern) } else { None };
    let attr = attr_string(text, font, color, k, url);
    let _: () = msg_send![label, setAttributedStringValue: attr];
    let _: () = msg_send![attr, release];
    label
}

unsafe fn make_button(title: &str, font: id, accent: bool, target: id, action: Sel) -> id {
    let btn: id = msg_send![class!(NSButton), alloc];
    let btn: id = msg_send![btn, init];
    let _: () = msg_send![btn, setBordered: NO];
    let _: () = msg_send![btn, setFont: font];
    let s = NSString::alloc(nil).init_str(title);
    let _: () = msg_send![btn, setTitle: s];
    let _: () = msg_send![s, release];
    let color: id = if accent {
        let _: () = msg_send![btn, setWantsLayer: YES];
        let layer: id = msg_send![btn, layer];
        let cg: id = msg_send![accent_color(), CGColor];
        let _: () = msg_send![layer, setBackgroundColor: cg];
        let _: () = msg_send![layer, setCornerRadius: 6.0_f64];
        msg_send![class!(NSColor), whiteColor]
    } else {
        secondary_color()
    };
    let attr = attr_string(title, font, color, None, None);
    let _: () = msg_send![btn, setAttributedTitle: attr];
    let _: () = msg_send![attr, release];
    let _: () = msg_send![btn, setTarget: target];
    let _: () = msg_send![btn, setAction: action];
    btn
}

// NSMutableAttributedString with font+color, optional kern, optional link.
// Caller owns and must release.
unsafe fn attr_string(
    text: &str,
    font: id,
    color: id,
    kern: Option<f64>,
    link: Option<&str>,
) -> id {
    let body = NSString::alloc(nil).init_str(text);
    let attr: id = msg_send![class!(NSMutableAttributedString), alloc];
    let attr: id = msg_send![attr, initWithString: body];
    let _: () = msg_send![body, release];
    let r = NSRange::new(0, text.chars().count() as u64);
    add_attr(attr, "NSFont", font, r);
    add_attr(attr, "NSColor", color, r);
    if let Some(k) = kern {
        let n: id = msg_send![class!(NSNumber), numberWithDouble: k];
        add_attr(attr, "NSKern", n, r);
    }
    if let Some(url) = link {
        let u = NSString::alloc(nil).init_str(url);
        add_attr(attr, "NSLink", u, r);
        let _: () = msg_send![u, release];
    }
    attr
}

unsafe fn add_attr(s: id, key: &str, value: id, r: NSRange) {
    let k = NSString::alloc(nil).init_str(key);
    let _: () = msg_send![s, addAttribute: k value: value range: r];
    let _: () = msg_send![k, release];
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
        }
        decl.register()
    })
}

extern "C" fn can_become_key_window(_: &Object, _: Sel) -> bool {
    true
}

extern "C" fn can_become_main_window(_: &Object, _: Sel) -> bool {
    true
}

fn about_window_class() -> &'static Class {
    static CLS: OnceLock<&'static Class> = OnceLock::new();
    CLS.get_or_init(|| {
        let mut decl = ClassDecl::new("MMAboutWindow", class!(NSWindow))
            .expect("MMAboutWindow registered twice");
        unsafe {
            decl.add_method(
                sel!(canBecomeKeyWindow),
                can_become_key_window as extern "C" fn(&Object, Sel) -> bool,
            );
            decl.add_method(
                sel!(canBecomeMainWindow),
                can_become_main_window as extern "C" fn(&Object, Sel) -> bool,
            );
        }
        decl.register()
    })
}
