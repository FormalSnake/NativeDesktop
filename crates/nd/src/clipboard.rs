//! The clipboard's text, on every target: `NSPasteboard` on AppKit, GPUI's platform clipboard on
//! the GPUI backend, which on the web is the async Clipboard API. Reads answer through a
//! callback, since the browser resolves them later and may ask the user first.

use nd_ui::App;

pub fn write_text(cx: &mut App, text: impl Into<String>) {
    let text = text.into();
    #[cfg(any(not(target_os = "macos"), feature = "gpui"))]
    if let Some(backend) = cx.backend_as::<nd_backend_gpui::GpuiBackend>() {
        backend.write_clipboard_text(text);
        return;
    }
    #[cfg(target_os = "macos")]
    mac::write(&text);
    let _ = (cx, text);
}

pub fn read_text(cx: &mut App, done: impl FnOnce(Option<String>, &mut App) + 'static) {
    #[cfg(any(not(target_os = "macos"), feature = "gpui"))]
    if let Some(backend) = cx.backend_as::<nd_backend_gpui::GpuiBackend>() {
        backend.read_clipboard_text(done);
        return;
    }
    #[cfg(target_os = "macos")]
    return done(mac::read(), cx);
    #[cfg(not(target_os = "macos"))]
    done(None, cx)
}

#[cfg(target_os = "macos")]
mod mac {
    use nd_backend_appkit::objc2_app_kit::{NSPasteboard, NSPasteboardTypeString};
    use nd_backend_appkit::objc2_foundation::NSString;

    pub fn write(text: &str) {
        let board = NSPasteboard::generalPasteboard();
        board.clearContents();
        // SAFETY: a framework constant, valid for the life of the process.
        let kind = unsafe { NSPasteboardTypeString };
        board.setString_forType(&NSString::from_str(text), kind);
    }

    pub fn read() -> Option<String> {
        // SAFETY: as above.
        let kind = unsafe { NSPasteboardTypeString };
        NSPasteboard::generalPasteboard().stringForType(kind).map(|s| s.to_string())
    }
}
