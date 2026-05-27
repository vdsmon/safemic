use crate::icons::{popup_icon_color, rasterize_svg};
use anyhow::{Context, Result};
use cocoa::appkit::{NSColor, NSImage, NSImageView, NSTextField};
use cocoa::base::{id, nil, NO};
use cocoa::foundation::{NSData, NSPoint, NSRect, NSSize, NSString};
use objc::runtime::Object;
use tao::dpi::LogicalSize;
use tao::window::Theme;

const MUTED_DESCRIPTION: &str = "Mic off";
const UNMUTED_DESCRIPTION: &str = "Mic on";

const ICON_WIDTH: f64 = 18.0;
const STACK_SPACING: f64 = 6.0;
const HORIZONTAL_PADDING: f64 = 16.0;

pub fn get_mic_mute_description_text(muted: bool) -> &'static str {
    if muted {
        MUTED_DESCRIPTION
    } else {
        UNMUTED_DESCRIPTION
    }
}

/// Width of the popup pill needed to fit both possible label states.
/// Measured once with the same system font used by `get_textfield`.
pub fn max_pill_width() -> f64 {
    let muted = unsafe { measure_label_width(MUTED_DESCRIPTION) };
    let unmuted = unsafe { measure_label_width(UNMUTED_DESCRIPTION) };
    let label = muted.max(unmuted);
    (ICON_WIDTH + STACK_SPACING + label + HORIZONTAL_PADDING * 2.0).ceil()
}

unsafe fn measure_label_width(text: &str) -> f64 {
    objc::rc::autoreleasepool(|| {
        let str_ = NSString::alloc(nil).init_str(text);
        let ns_font = class!(NSFont);
        let default_size: f64 = msg_send![ns_font, systemFontSize];
        let font: id = msg_send![ns_font, systemFontOfSize: default_size + 3.0_f64];
        let attrs: id = msg_send![class!(NSMutableDictionary), dictionary];
        let key = NSString::alloc(nil).init_str("NSFont");
        let _: () = msg_send![attrs, setObject: font forKey: key];
        let size: NSSize = msg_send![str_, sizeWithAttributes: attrs];
        let _: () = msg_send![str_, release];
        let _: () = msg_send![key, release];
        size.width
    })
}

/// Vertically-centered 18pt-tall rect spanning the full width.
/// Matches the original layout so the NSStackView stays at a fixed size
/// and does not activate Auto Layout resizing on the window.
fn get_frame_rect(size: LogicalSize<f64>) -> NSRect {
    const LINE_HEIGHT: f64 = 18.;
    NSRect::new(
        NSPoint::new(0., (size.height - LINE_HEIGHT) / 2.),
        NSSize::new(size.width, LINE_HEIGHT),
    )
}

fn get_text_color(muted: bool, theme: Theme) -> id {
    unsafe {
        // 239, 68, 68 (light mode red) - #ef4444 / 248, 113, 113 (dark mode red) - #f87171
        let dark_red = NSColor::colorWithRed_green_blue_alpha_(nil, 0.9372, 0.2666, 0.2666, 1.);
        let light_red = NSColor::colorWithRed_green_blue_alpha_(nil, 0.9725, 0.4431, 0.4431, 1.);
        let black = NSColor::colorWithRed_green_blue_alpha_(nil, 0., 0., 0., 1.);
        let white = NSColor::colorWithRed_green_blue_alpha_(nil, 1., 1., 1., 1.);
        match theme {
            Theme::Light if muted => dark_red,
            Theme::Light => black,
            Theme::Dark if muted => light_red,
            _ => white,
        }
    }
}

fn get_textfield(text: &str, color: id, frame: NSRect) -> id {
    unsafe {
        let label = NSTextField::alloc(nil);
        let _: () = msg_send![label, initWithFrame: frame];
        let label_str = NSString::alloc(nil).init_str(text);
        label.setStringValue_(label_str);
        let _: () = msg_send![label_str, release];
        let _: () = msg_send![label, setTextColor: color];
        let _: () = msg_send![label, setBezeled: NO];
        let _: () = msg_send![label, setEditable: NO];
        let _: () = msg_send![label, setDrawsBackground: NO];
        let _: () = msg_send![label, setSelectable: NO];
        const NSALIGNMENT_CENTER: i32 = 1;
        let _: () = msg_send![label, setAlignment: NSALIGNMENT_CENTER];
        let ns_font = class!(NSFont);
        let default_size: f64 = msg_send![ns_font, systemFontSize];
        let custom_font: *mut Object = msg_send![ns_font, systemFontOfSize: default_size + 3.0_f64];
        let _: () = msg_send![label, setFont: custom_font];
        label
    }
}

/// Rasterizes an SVG and returns PNG-encoded bytes plus source dimensions.
fn svg_to_png(svg_bytes: &[u8], muted: bool, theme: Theme) -> Result<(Vec<u8>, u32, u32)> {
    let color = popup_icon_color(muted, theme);
    let (rgba, w, h) = rasterize_svg(svg_bytes, &color)?;
    let img = image::RgbaImage::from_raw(w, h, rgba).context("Failed to create RgbaImage")?;
    let mut png = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .context("Failed to encode PNG")?;
    Ok((png, w, h))
}

fn svg_to_ns_image(svg_bytes: &[u8], muted: bool, theme: Theme) -> Result<id> {
    let (png, w, h) = svg_to_png(svg_bytes, muted, theme)?;
    const ICON_HEIGHT: f64 = 16.;
    let icon_width = (w as f64) / (h as f64 / ICON_HEIGHT);
    let ns_image = unsafe {
        let nsdata = NSData::dataWithBytes_length_(
            nil,
            png.as_ptr() as *const std::os::raw::c_void,
            png.len() as u64,
        );
        let ns_image = NSImage::initWithData_(NSImage::alloc(nil), nsdata);
        let _: () = msg_send![ns_image, setSize: NSSize::new(icon_width, ICON_HEIGHT)];
        let _: () = msg_send![ns_image, setTemplate: NO];
        ns_image
    };
    Ok(ns_image)
}

fn get_mic_image(muted: bool, theme: Theme) -> Result<id> {
    const MIC_ON: &[u8] = include_bytes!("../assets/mic.svg");
    const MIC_OFF: &[u8] = include_bytes!("../assets/mic-off.svg");
    svg_to_ns_image(if muted { MIC_OFF } else { MIC_ON }, muted, theme)
}

fn make_image_view(image: id, frame: NSRect) -> id {
    unsafe {
        let view = NSImageView::alloc(nil);
        let _: () = msg_send![view, initWithFrame: frame];
        view.setImage_(image);
        view
    }
}

#[derive(Copy, Clone)]
pub struct PopupContent {
    mic_label: id,
    mic_image: id,
    pub view: id,
}

impl PopupContent {
    pub fn new(mic_muted: bool, size: LogicalSize<f64>, theme: Theme) -> Result<Self> {
        let frame = get_frame_rect(size);

        let mic_label = get_textfield(
            get_mic_mute_description_text(mic_muted),
            get_text_color(mic_muted, theme),
            frame,
        );
        let mic_ns_image = get_mic_image(mic_muted, theme)?;
        let mic_image = make_image_view(mic_ns_image, frame);
        unsafe {
            let _: () = msg_send![mic_ns_image, release];
        }

        let view = unsafe {
            let stack: *mut Object = msg_send![class!(NSStackView), alloc];
            let _: () = msg_send![stack, initWithFrame: frame];
            const GRAVITY_CENTER: i32 = 2;
            let _: () = msg_send![stack, addView: mic_image inGravity: GRAVITY_CENTER];
            let _: () = msg_send![mic_image, release];
            let _: () = msg_send![stack, addView: mic_label inGravity: GRAVITY_CENTER];
            let _: () = msg_send![mic_label, release];
            stack
        };

        Ok(Self {
            mic_label,
            mic_image,
            view,
        })
    }

    pub fn update(
        &mut self,
        mic_muted: bool,
        theme: Theme,
        _active_device_name: Option<&str>,
    ) -> Result<&mut Self> {
        let mic_img = get_mic_image(mic_muted, theme)?;
        unsafe {
            let mic_str = NSString::alloc(nil).init_str(get_mic_mute_description_text(mic_muted));
            self.mic_label.setStringValue_(mic_str);
            let _: () = msg_send![mic_str, release];
            let _: () = msg_send![self.mic_label, setTextColor: get_text_color(mic_muted, theme)];
            self.mic_image.setImage_(mic_img);
            let _: () = msg_send![mic_img, release];
        }
        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mic_mute_description_muted() {
        assert_eq!(get_mic_mute_description_text(true), "Mic off");
    }

    #[test]
    fn test_mic_mute_description_unmuted() {
        assert_eq!(get_mic_mute_description_text(false), "Mic on");
    }
}
