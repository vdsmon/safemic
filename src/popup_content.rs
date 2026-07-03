/// Popup content: a system-OSD-style bezel (NSVisualEffectView, HUD
/// material) holding a single large SF Symbol mic glyph — no text, like the
/// volume/brightness bezel. Semantic colors: red when muted, plain label
/// color otherwise; material and tint adapt to light/dark automatically.
use anyhow::Result;
use cocoa::base::{id, nil, YES};
use cocoa::foundation::{NSPoint, NSRect, NSSize, NSString};
use tao::dpi::LogicalSize;

const MUTED_DESCRIPTION: &str = "Microphone off";
const UNMUTED_DESCRIPTION: &str = "Microphone on";
const MUTED_SYMBOL: &str = "mic.slash.fill";
const UNMUTED_SYMBOL: &str = "mic.fill";

/// Square bezel, volume-OSD style at a slightly more discreet scale.
pub const BEZEL_SIZE: f64 = 150.0;
const CORNER_RADIUS: f64 = 22.0;
const SYMBOL_POINT_SIZE: f64 = 69.0;
/// Frame the glyph renders into; symbols are wider than tall, so give the
/// image view generous square bounds and let AppKit center the glyph.
const ICON_FRAME: f64 = 104.0;
const NS_FONT_WEIGHT_REGULAR: f64 = 0.0;
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

/// Glyph tint: semantic red when muted, standard label color otherwise.
/// Both are dynamic colors, correct on any appearance and wallpaper.
unsafe fn tint_color(muted: bool) -> id {
    if muted {
        msg_send![class!(NSColor), systemRedColor]
    } else {
        msg_send![class!(NSColor), labelColor]
    }
}

unsafe fn symbol_image(muted: bool) -> id {
    let name = NSString::alloc(nil).init_str(symbol_name(muted));
    let desc = NSString::alloc(nil).init_str(get_mic_mute_description_text(muted));
    let img: id = msg_send![class!(NSImage),
        imageWithSystemSymbolName: name accessibilityDescription: desc];
    let _: () = msg_send![name, release];
    let _: () = msg_send![desc, release];
    img
}

#[derive(Copy, Clone)]
pub struct PopupContent {
    mic_image: id,
    pub view: id,
}

impl PopupContent {
    pub fn new(mic_muted: bool, size: LogicalSize<f64>) -> Result<Self> {
        let (view, mic_image) = unsafe {
            let effect: id = msg_send![class!(NSVisualEffectView), alloc];
            let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(size.width, size.height));
            let effect: id = msg_send![effect, initWithFrame: frame];
            let _: () = msg_send![effect, setMaterial: MATERIAL_HUD_WINDOW];
            let _: () = msg_send![effect, setBlendingMode: BLENDING_BEHIND_WINDOW];
            let _: () = msg_send![effect, setState: STATE_ACTIVE];
            let _: () = msg_send![effect, setWantsLayer: YES];
            let layer: id = msg_send![effect, layer];
            let _: () = msg_send![layer, setCornerRadius: CORNER_RADIUS];
            let _: () = msg_send![layer, setMasksToBounds: YES];

            let image_view: id = msg_send![class!(NSImageView), alloc];
            let icon_xy = ((size.width - ICON_FRAME) / 2.0).floor();
            let icon_frame = NSRect::new(
                NSPoint::new(icon_xy, ((size.height - ICON_FRAME) / 2.0).floor()),
                NSSize::new(ICON_FRAME, ICON_FRAME),
            );
            let image_view: id = msg_send![image_view, initWithFrame: icon_frame];
            let cfg: id = msg_send![class!(NSImageSymbolConfiguration),
                configurationWithPointSize: SYMBOL_POINT_SIZE weight: NS_FONT_WEIGHT_REGULAR];
            let _: () = msg_send![image_view, setSymbolConfiguration: cfg];
            let _: () = msg_send![effect, addSubview: image_view];
            let _: () = msg_send![image_view, release];
            (effect, image_view)
        };

        let mut content = Self { mic_image, view };
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

    fn apply(&mut self, mic_muted: bool) -> Result<()> {
        unsafe {
            let img = symbol_image(mic_muted);
            let _: () = msg_send![self.mic_image, setImage: img];
            let _: () = msg_send![self.mic_image, setContentTintColor: tint_color(mic_muted)];
        }
        Ok(())
    }
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
