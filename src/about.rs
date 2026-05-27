/// About dialog: borderless NSWindow, centered icon + wordmark + body + pill
/// CTA, dark solid card with rounded corners + hairline border. Modal via
/// `runModalForWindow:`.
use anyhow::Result;
use cocoa::base::{id, nil, NO, YES};
use cocoa::foundation::{NSData, NSPoint, NSRange, NSRect, NSSize, NSString};
use objc::declare::ClassDecl;
use objc::runtime::{Class, Object, Sel};
use std::os::raw::c_void;
use std::process::Command;
use std::sync::OnceLock;

const REPO_URL: &str = "https://github.com/vdsmon/safemic";
const APP_ICON_PNG: &[u8] = include_bytes!("../assets/icons/256x256@2x.png");
const OCTOCAT_PNG: &[u8] = include_bytes!("../assets/icons/octocat-32.png");

// Window dimensions roughly track target.png aspect (1538x1023 ≈ 3:2).
// Y-position constants are measured from the top of the card and roughly
// match where each element lands in target.png after 500/1023 scaling.
const WIN_W: f64 = 720.0;
const WIN_H: f64 = 500.0;
const CORNER_RADIUS: f64 = 14.0;
const CARD_MARGIN: f64 = 4.0;
const PAD: f64 = 28.0;

const ICON_SIZE: f64 = 86.0;
const ICON_TOP: f64 = 89.0;
const WORDMARK_SIZE: f64 = 36.0;
const WORDMARK_TOP: f64 = 195.0;
const VERSION_SIZE: f64 = 16.0;
const VERSION_TOP: f64 = 245.0;
const DIVIDER_TOP: f64 = 305.0;
const BODY_SIZE: f64 = 13.0;
const BODY_TOP: f64 = 320.0;
const BUTTON_W: f64 = 165.0;
const BUTTON_H: f64 = 41.0;
const BUTTON_RADIUS: f64 = 20.5;
const BUTTON_TOP: f64 = 380.0;
const TRAFFIC_LIGHT: f64 = 10.0;

const FW_REGULAR: f64 = 0.0;
const FW_SEMIBOLD: f64 = 0.3;
const FW_BOLD: f64 = 0.4;

// NSTextAlignment values on this AppKit version follow the UIKit numbering:
// 0=Left, 1=Center, 2=Right, 3=Justified, 4=Natural. Tested empirically by
// flipping setAlignment values and observing label position.
const ALIGN_CENTER: u64 = 1;

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
    let target = rect(ox, oy, WIN_W, WIN_H);

    let window: id = msg_send![about_window_class(), alloc];
    let window: id = msg_send![window, initWithContentRect: rect(ox, oy - 6.0, WIN_W, WIN_H)
        styleMask: 0u64 backing: 2u64 defer: NO];
    let _: () = msg_send![window, setReleasedWhenClosed: NO];
    let clear: id = msg_send![class!(NSColor), clearColor];
    let _: () = msg_send![window, setOpaque: NO];
    let _: () = msg_send![window, setBackgroundColor: clear];
    let _: () = msg_send![window, setMovableByWindowBackground: YES];
    let _: () = msg_send![window, setHasShadow: YES];
    let _: () = msg_send![window, setAlphaValue: 0.0_f64];

    // Root: outer canvas. Slightly darker than the inner card so the
    // hairline edge reads.
    let content_rect = rect(0.0, 0.0, WIN_W, WIN_H);
    let content: id = msg_send![class!(NSView), alloc];
    let content: id = msg_send![content, initWithFrame: content_rect];
    let _: () = msg_send![content, setWantsLayer: YES];
    let layer: id = msg_send![content, layer];
    let _: () = msg_send![layer, setCornerRadius: CORNER_RADIUS];
    let _: () = msg_send![layer, setMasksToBounds: YES];
    // Sampled from target.png top-left.
    let outer_cg: id = msg_send![rgb(0.102, 0.106, 0.110), CGColor];
    let _: () = msg_send![layer, setBackgroundColor: outer_cg];

    // Inner card: a slightly lighter rounded rect inset by CARD_MARGIN.
    let card_rect = rect(
        CARD_MARGIN,
        CARD_MARGIN,
        WIN_W - 2.0 * CARD_MARGIN,
        WIN_H - 2.0 * CARD_MARGIN,
    );
    let card: id = msg_send![class!(NSView), alloc];
    let card: id = msg_send![card, initWithFrame: card_rect];
    let _: () = msg_send![card, setWantsLayer: YES];
    let card_layer: id = msg_send![card, layer];
    let _: () = msg_send![card_layer, setCornerRadius: CORNER_RADIUS - 2.0];
    let _: () = msg_send![card_layer, setMasksToBounds: YES];
    // Subtle lift over outer canvas (sampled from target card interior).
    let card_cg: id = msg_send![rgb(0.118, 0.118, 0.122), CGColor];
    let _: () = msg_send![card_layer, setBackgroundColor: card_cg];
    let border_cg: id = msg_send![rgb(0.22, 0.22, 0.24), CGColor];
    let _: () = msg_send![card_layer, setBorderColor: border_cg];
    let _: () = msg_send![card_layer, setBorderWidth: 1.0_f64];
    let _: () = msg_send![content, addSubview: card];

    let card_w = WIN_W - 2.0 * CARD_MARGIN;
    let card_h = WIN_H - 2.0 * CARD_MARGIN;

    // Action target. Leaked Box mirrors MMSettingsActions; freed after runModal.
    let ctx_ptr = Box::into_raw(Box::new(())) as *mut c_void;
    let tgt: id = msg_send![actions_class(), alloc];
    let tgt: id = msg_send![tgt, init];
    (*tgt).set_ivar("_ctxPtr", ctx_ptr);

    // Helper: convert "y measured from top of card" to NSView bottom-up coords.
    let from_top = |y_top: f64, h: f64| -> f64 { card_h - y_top - h };

    // Traffic-light close button: red filled circle, top-left of card.
    // Position chosen to match target.png (red dot center ≈ 6.6%, 7.3%).
    let tl_x = 22.0;
    let tl_y = from_top(28.0, TRAFFIC_LIGHT);
    let tl: id = msg_send![class!(NSButton), alloc];
    let tl: id = msg_send![tl, initWithFrame: rect(tl_x, tl_y, TRAFFIC_LIGHT, TRAFFIC_LIGHT)];
    let _: () = msg_send![tl, setBordered: NO];
    let _: () = msg_send![tl, setTitle: NSString::alloc(nil).init_str("")];
    let _: () = msg_send![tl, setWantsLayer: YES];
    let tl_layer: id = msg_send![tl, layer];
    let tl_cg: id = msg_send![rgb(0.99, 0.36, 0.34), CGColor];
    let _: () = msg_send![tl_layer, setBackgroundColor: tl_cg];
    let _: () = msg_send![tl_layer, setCornerRadius: TRAFFIC_LIGHT / 2.0];
    let _: () = msg_send![tl, setTarget: tgt];
    let _: () = msg_send![tl, setAction: sel!(closeAction:)];
    let esc = NSString::alloc(nil).init_str("\u{1b}");
    let _: () = msg_send![tl, setKeyEquivalent: esc];
    let _: () = msg_send![esc, release];
    let _: () = msg_send![card, addSubview: tl];
    let _: () = msg_send![tl, release];

    // Icon: centered horizontally, near top.
    let icon_x = (card_w - ICON_SIZE) / 2.0;
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
    let _: () = msg_send![card, addSubview: icon_view];
    let _: () = msg_send![icon_view, release];

    // "SafeMic" wordmark, large bold sans, centered.
    let wm_h = WORDMARK_SIZE + 10.0;
    let wm_y = from_top(WORDMARK_TOP, wm_h);
    let wm = make_label_aligned(
        "SafeMic",
        system_font(WORDMARK_SIZE, FW_BOLD),
        label_color(),
        0.0,
        false,
        None,
        ALIGN_CENTER,
    );
    set_frame(wm, 0.0, wm_y, card_w, wm_h);
    let _: () = msg_send![card, addSubview: wm];
    let _: () = msg_send![wm, release];

    // Version, gray, below wordmark.
    let vh = VERSION_SIZE + 6.0;
    let vy = from_top(VERSION_TOP, vh);
    let vl = make_label_aligned(
        "v0.5.1",
        system_font(VERSION_SIZE, FW_REGULAR),
        muted_label_color(),
        0.0,
        false,
        None,
        ALIGN_CENTER,
    );
    set_frame(vl, 0.0, vy, card_w, vh);
    let _: () = msg_send![card, addSubview: vl];
    let _: () = msg_send![vl, release];

    // Divider line. V5: keep full hairline width (matches target — divider
    // spans nearly the card interior with side margins).
    let div_w = card_w - 2.0 * PAD;
    let div_y = from_top(DIVIDER_TOP, 1.0);
    let div: id = msg_send![class!(NSView), alloc];
    let div: id = msg_send![div, initWithFrame: rect(PAD, div_y, div_w, 1.0)];
    let _: () = msg_send![div, setWantsLayer: YES];
    let div_layer: id = msg_send![div, layer];
    let div_cg: id = msg_send![rgb(0.24, 0.24, 0.26), CGColor];
    let _: () = msg_send![div_layer, setBackgroundColor: div_cg];
    let _: () = msg_send![card, addSubview: div];
    let _: () = msg_send![div, release];

    // Body copy: two-line description, centered.
    let body_text =
        "A simple menu bar app to mute your microphone\ninstantly and keep your conversations private.";
    // V5: tighter body box — 2 lines of 13pt with small line gap; trimmed
    // padding so vertical centering inside the box matches target spacing.
    let body_h = (BODY_SIZE + 4.0) * 2.0;
    let body_y = from_top(BODY_TOP, body_h);
    let body = make_label_aligned(
        body_text,
        system_font(BODY_SIZE, FW_REGULAR),
        body_color(),
        0.0,
        false,
        None,
        ALIGN_CENTER,
    );
    let _: () = msg_send![body, setUsesSingleLineMode: NO];
    let cell: id = msg_send![body, cell];
    let _: () = msg_send![cell, setWraps: YES];
    set_frame(body, PAD, body_y, card_w - 2.0 * PAD, body_h);
    let _: () = msg_send![card, addSubview: body];
    let _: () = msg_send![body, release];

    // Pill button "View on GitHub" with octocat icon on the left.
    let btn_x = (card_w - BUTTON_W) / 2.0;
    let btn_y = from_top(BUTTON_TOP, BUTTON_H);
    let btn_font = system_font(16.0, FW_SEMIBOLD);
    let ob = make_pill_button("View on GitHub", btn_font, tgt, sel!(openGitHubAction:));
    let cat_data = NSData::dataWithBytes_length_(
        nil,
        OCTOCAT_PNG.as_ptr() as *const c_void,
        OCTOCAT_PNG.len() as u64,
    );
    let cat_img: id = msg_send![class!(NSImage), alloc];
    let cat_img: id = msg_send![cat_img, initWithData: cat_data];
    let _: () = msg_send![cat_img, setSize: NSSize::new(20.0, 20.0)];
    let _: () = msg_send![ob, setImage: cat_img];
    let _: () = msg_send![ob, setImagePosition: 2_u64];
    let _: () = msg_send![cat_img, release];
    set_frame(ob, btn_x, btn_y, BUTTON_W, BUTTON_H);
    let enter = NSString::alloc(nil).init_str("\r");
    let _: () = msg_send![ob, setKeyEquivalent: enter];
    let _: () = msg_send![ob, setKeyEquivalentModifierMask: 0u64];
    let _: () = msg_send![enter, release];
    let _: () = msg_send![card, addSubview: ob];
    let _: () = msg_send![ob, release];

    let _: () = msg_send![card, release];
    let _: () = msg_send![window, setContentView: content];
    let _: () = msg_send![content, release];
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
    // Slightly off-white to match the target's anti-aliased peak brightness.
    rgb(0.93, 0.92, 0.93)
}
unsafe fn muted_label_color() -> id {
    // Sampled from target.png "v0.5.1".
    rgb(0.49, 0.49, 0.49)
}
unsafe fn body_color() -> id {
    // Sampled from target.png body copy.
    rgb(0.60, 0.61, 0.62)
}
unsafe fn accent_button_color() -> id {
    // The pill button in the target is a touch darker than the icon.
    rgb(0.855, 0.373, 0.369)
}
unsafe fn rgb(r: f64, g: f64, b: f64) -> id {
    msg_send![class!(NSColor),
        colorWithSRGBRed: r green: g blue: b alpha: 1.0_f64]
}
unsafe fn system_font(size: f64, weight: f64) -> id {
    msg_send![class!(NSFont), systemFontOfSize: size weight: weight]
}

// NSTextAlignment: 0=left, 1=right, 2=center, 3=justified, 4=natural.
unsafe fn make_label_aligned(
    text: &str,
    font: id,
    color: id,
    kern: f64,
    link: bool,
    url: Option<&str>,
    alignment: u64,
) -> id {
    let label: id = msg_send![class!(NSTextField), alloc];
    let label: id = msg_send![label, init];
    let _: () = msg_send![label, setBezeled: NO];
    let _: () = msg_send![label, setBordered: NO];
    let _: () = msg_send![label, setEditable: NO];
    let _: () = msg_send![label, setDrawsBackground: NO];
    let _: () = msg_send![label, setSelectable: if link { YES } else { NO }];
    let _: () = msg_send![label, setFont: font];
    let _: () = msg_send![label, setTextColor: color];
    let s = NSString::alloc(nil).init_str(text);
    let _: () = msg_send![label, setStringValue: s];
    let _: () = msg_send![s, release];
    // Setting alignment AFTER stringValue ensures the cell's paragraph style
    // for the plain string path picks up the right alignment.
    let _: () = msg_send![label, setAlignment: alignment];
    if link {
        let _: () = msg_send![label, setAllowsEditingTextAttributes: YES];
    }
    if kern != 0.0 || url.is_some() {
        let k = if kern != 0.0 { Some(kern) } else { None };
        let attr = attr_string_aligned(text, font, color, k, url, alignment);
        let _: () = msg_send![label, setAttributedStringValue: attr];
        let _: () = msg_send![attr, release];
        let _: () = msg_send![label, setAlignment: alignment];
    }
    label
}

unsafe fn make_pill_button(title: &str, font: id, target: id, action: Sel) -> id {
    let btn: id = msg_send![class!(NSButton), alloc];
    let btn: id = msg_send![btn, init];
    let _: () = msg_send![btn, setBordered: NO];
    let _: () = msg_send![btn, setFont: font];
    let s = NSString::alloc(nil).init_str(title);
    let _: () = msg_send![btn, setTitle: s];
    let _: () = msg_send![s, release];
    let _: () = msg_send![btn, setWantsLayer: YES];
    let layer: id = msg_send![btn, layer];
    let cg: id = msg_send![accent_button_color(), CGColor];
    let _: () = msg_send![layer, setBackgroundColor: cg];
    let _: () = msg_send![layer, setCornerRadius: BUTTON_RADIUS];
    let white: id = msg_send![class!(NSColor), whiteColor];
    let attr = attr_string(title, font, white, None, None);
    let _: () = msg_send![btn, setAttributedTitle: attr];
    let _: () = msg_send![attr, release];
    let _: () = msg_send![btn, setTarget: target];
    let _: () = msg_send![btn, setAction: action];
    btn
}

unsafe fn attr_string(
    text: &str,
    font: id,
    color: id,
    kern: Option<f64>,
    link: Option<&str>,
) -> id {
    attr_string_aligned(text, font, color, kern, link, 0u64)
}

unsafe fn attr_string_aligned(
    text: &str,
    font: id,
    color: id,
    kern: Option<f64>,
    link: Option<&str>,
    alignment: u64,
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
    if alignment != 0 {
        let ps: id = msg_send![class!(NSMutableParagraphStyle), alloc];
        let ps: id = msg_send![ps, init];
        let _: () = msg_send![ps, setAlignment: alignment];
        add_attr(attr, "NSParagraphStyle", ps, r);
        let _: () = msg_send![ps, release];
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
