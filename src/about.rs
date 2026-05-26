/// About dialog: version, repo link, project icon.
use anyhow::Result;
use cocoa::base::nil;
use cocoa::foundation::{NSData, NSString};
use objc::runtime::Object;
use std::process::Command;

const REPO_URL: &str = "https://github.com/vdsmon/mic-mute";

/// Project icon embedded at compile time. Uses the 256x256@2x bundle icon so
/// the NSAlert renders crisp on Retina displays.
const APP_ICON_PNG: &[u8] = include_bytes!("../assets/icons/256x256@2x.png");

/// Show the About window as an NSAlert dialog.
pub fn show_about() -> Result<()> {
    let response = unsafe {
        let alert: *mut Object = msg_send![class!(NSAlert), new];

        // project icon
        let nsdata = NSData::dataWithBytes_length_(
            nil,
            APP_ICON_PNG.as_ptr() as *const std::os::raw::c_void,
            APP_ICON_PNG.len() as u64,
        );
        let image: *mut Object = msg_send![class!(NSImage), alloc];
        let image: *mut Object = msg_send![image, initWithData: nsdata];
        let _: () = msg_send![alert, setIcon: image];
        let _: () = msg_send![image, release];

        let title = NSString::alloc(nil).init_str("Mic Mute");
        let _: () = msg_send![alert, setMessageText: title];
        let _: () = msg_send![title, release];

        let version = env!("CARGO_PKG_VERSION");
        let info = format!("Version: {version}\n\nSource: {REPO_URL}");
        let info_str = NSString::alloc(nil).init_str(&info);
        let _: () = msg_send![alert, setInformativeText: info_str];
        let _: () = msg_send![info_str, release];

        let ok_str = NSString::alloc(nil).init_str("OK");
        let _: () = msg_send![alert, addButtonWithTitle: ok_str];
        let _: () = msg_send![ok_str, release];

        let visit_str = NSString::alloc(nil).init_str("Open GitHub");
        let _: () = msg_send![alert, addButtonWithTitle: visit_str];
        let _: () = msg_send![visit_str, release];

        // 1000 = OK, 1001 = Open GitHub
        let response: i64 = msg_send![alert, runModal];
        let _: () = msg_send![alert, release];
        response
    };

    if response == 1001 {
        let _ = Command::new("open").arg(REPO_URL).spawn();
    }

    Ok(())
}
