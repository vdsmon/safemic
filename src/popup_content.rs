/// Popup content: a frosted vibrancy capsule (NSVisualEffectView, HUD
/// material) holding an SF Symbol mic glyph + state label. Semantic colors:
/// red only when muted, plain label color otherwise; the material and text
/// adapt to light/dark automatically, so no theme plumbing is needed.
use anyhow::Result;
use cocoa::appkit::NSTextField;
use cocoa::base::{id, nil, NO, YES};
use cocoa::foundation::{NSPoint, NSRect, NSSize, NSString};
use objc::runtime::Object;
use tao::dpi::LogicalSize;

const MUTED_DESCRIPTION: &str = "Microphone off";
const UNMUTED_DESCRIPTION: &str = "Microphone on";
const MUTED_SYMBOL: &str = "mic.slash.fill";
const UNMUTED_SYMBOL: &str = "mic.fill";

const ICON_WIDTH: f64 = 22.0;
const ICON_TEXT_GAP: f64 = 7.0;
const HORIZONTAL_PADDING: f64 = 16.0;
const SYMBOL_POINT_SIZE: f64 = 14.0;
const NS_FONT_WEIGHT_MEDIUM: f64 = 0.23;
// NSVisualEffectView raw enum values.
const MATERIAL_HUD_WINDOW: u64 = 13;
const BLENDING_BEHIND_WINDOW: u64 = 0;
const STATE_ACTIVE: u64 = 1;

pub fn get_mic_mute_description_text(muted: bool) -> &'static str {
    if muted {
        MUTED_DESCRIPTION
    } else {
        UNMUTED_DESCRIPTION
    }
}

fn symbol_name(muted: bool) -> &'static str {
    if muted {
        MUTED_SYMBOL
    } else {
        UNMUTED_SYMBOL
    }
}

/// Width of the popup capsule needed to fit both possible label states.
/// Measured once with the same system font used by the label.
pub fn max_pill_width() -> f64 {
    let muted = unsafe { measure_label_width(MUTED_DESCRIPTION) };
    let unmuted = unsafe { measure_label_width(UNMUTED_DESCRIPTION) };
    let label = muted.max(unmuted);
    (ICON_WIDTH + ICON_TEXT_GAP + label + HORIZONTAL_PADDING * 2.0).ceil()
}

unsafe fn measure_label_width(text: &str) -> f64 {
    objc::rc::autoreleasepool(|| {
        let str_ = NSString::alloc(nil).init_str(text);
        let attrs: id = msg_send![class!(NSMutableDictionary), dictionary];
        let key = NSString::alloc(nil).init_str("NSFont");
        let _: () = msg_send![attrs, setObject: label_font() forKey: key];
        let size: NSSize = msg_send![str_, sizeWithAttributes: attrs];
        let _: () = msg_send![str_, release];
        let _: () = msg_send![key, release];
        size.width
    })
}

unsafe fn label_font() -> id {
    let ns_font = class!(NSFont);
    let default_size: f64 = msg_send![ns_font, systemFontSize];
    msg_send![ns_font, systemFontOfSize: default_size + 3.0_f64]
}

/// Text/icon tint: semantic red when muted, standard label color otherwise.
/// Both are dynamic colors, correct on any appearance and wallpaper.
unsafe fn tint_color(muted: bool) -> id {
    if muted {
        msg_send![class!(NSColor), systemRedColor]
    } else {
        msg_send![class!(NSColor), labelColor]
    }
}

unsafe fn make_label(text: &str) -> id {
    let label = NSTextField::alloc(nil);
    let _: () = msg_send![label, init];
    let label_str = NSString::alloc(nil).init_str(text);
    label.setStringValue_(label_str);
    let _: () = msg_send![label_str, release];
    let _: () = msg_send![label, setBezeled: NO];
    let _: () = msg_send![label, setEditable: NO];
    let _: () = msg_send![label, setDrawsBackground: NO];
    let _: () = msg_send![label, setSelectable: NO];
    let _: () = msg_send![label, setFont: label_font()];
    label
}

unsafe fn symbol_image(muted: bool) -> id {
    let name = NSString::alloc(nil).init_str(symbol_name(muted));
    let img: id = msg_send![class!(NSImage),
        imageWithSystemSymbolName: name accessibilityDescription: nil as id];
    let _: () = msg_send![name, release];
    img
}

#[derive(Copy, Clone)]
pub struct PopupContent {
    mic_label: id,
    mic_image: id,
    size: LogicalSize<f64>,
    pub view: id,
}

impl PopupContent {
    pub fn new(mic_muted: bool, size: LogicalSize<f64>) -> Result<Self> {
        let view = unsafe {
            let effect: id = msg_send![class!(NSVisualEffectView), alloc];
            let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(size.width, size.height));
            let effect: id = msg_send![effect, initWithFrame: frame];
            let _: () = msg_send![effect, setMaterial: MATERIAL_HUD_WINDOW];
            let _: () = msg_send![effect, setBlendingMode: BLENDING_BEHIND_WINDOW];
            let _: () = msg_send![effect, setState: STATE_ACTIVE];
            let _: () = msg_send![effect, setWantsLayer: YES];
            let layer: id = msg_send![effect, layer];
            let _: () = msg_send![layer, setCornerRadius: size.height / 2.0];
            let _: () = msg_send![layer, setMasksToBounds: YES];
            effect
        };

        let (mic_label, mic_image) = unsafe {
            let label = make_label(get_mic_mute_description_text(mic_muted));
            let image_view: id = msg_send![class!(NSImageView), alloc];
            let image_view: id = msg_send![image_view, init];
            let cfg: id = msg_send![class!(NSImageSymbolConfiguration),
                configurationWithPointSize: SYMBOL_POINT_SIZE weight: NS_FONT_WEIGHT_MEDIUM];
            let _: () = msg_send![image_view, setSymbolConfiguration: cfg];
            let _: () = msg_send![view, addSubview: image_view];
            let _: () = msg_send![image_view, release];
            let _: () = msg_send![view, addSubview: label];
            let _: () = msg_send![label, release];
            (label, image_view)
        };

        let mut content = Self {
            mic_label,
            mic_image,
            size,
            view,
        };
        content.apply(mic_muted)?;
        Ok(content)
    }

    pub fn update(
        &mut self,
        mic_muted: bool,
        _active_device_name: Option<&str>,
    ) -> Result<&mut Self> {
        self.apply(mic_muted)?;
        Ok(self)
    }

    /// Set text/icon for the state and re-center the icon+label group inside
    /// the capsule (label width changes between "off" and "on").
    fn apply(&mut self, mic_muted: bool) -> Result<()> {
        unsafe {
            let text = get_mic_mute_description_text(mic_muted);
            let mic_str = NSString::alloc(nil).init_str(text);
            self.mic_label.setStringValue_(mic_str);
            let _: () = msg_send![mic_str, release];
            let tint = tint_color(mic_muted);
            let _: () = msg_send![self.mic_label, setTextColor: tint];
            let img = symbol_image(mic_muted);
            let _: () = msg_send![self.mic_image, setImage: img];
            let _: () = msg_send![self.mic_image, setContentTintColor: tint];

            let text_w = measure_label_width(text).ceil();
            let group_w = ICON_WIDTH + ICON_TEXT_GAP + text_w;
            let x0 = ((self.size.width - group_w) / 2.0).floor();
            let icon_h = 22.0;
            let icon_y = ((self.size.height - icon_h) / 2.0).floor();
            set_frame(self.mic_image, x0, icon_y, ICON_WIDTH, icon_h);
            let label_h = 20.0;
            let label_y = ((self.size.height - label_h) / 2.0).floor();
            set_frame(
                self.mic_label,
                x0 + ICON_WIDTH + ICON_TEXT_GAP,
                label_y,
                text_w + 2.0,
                label_h,
            );
        }
        Ok(())
    }
}

unsafe fn set_frame(view: *mut Object, x: f64, y: f64, w: f64, h: f64) {
    let frame = NSRect::new(NSPoint::new(x, y), NSSize::new(w, h));
    let _: () = msg_send![view, setFrame: frame];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mic_mute_description_muted() {
        assert_eq!(get_mic_mute_description_text(true), "Microphone off");
    }

    #[test]
    fn test_mic_mute_description_unmuted() {
        assert_eq!(get_mic_mute_description_text(false), "Microphone on");
    }
}
