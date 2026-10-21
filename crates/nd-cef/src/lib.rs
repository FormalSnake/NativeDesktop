//! Chromium as a web engine for `nd-webview` (ARCHITECTURE.md 8), through the `cef` crate. By
//! default in off-screen mode (Alloy style), so the page is an ordinary part of the window
//! (overlays above it, resized with it) instead of a Chromium window floating over it. Input,
//! IME, `<select>` popups, context menus, drag and drop, cursors and accessibility are all ours,
//! since an off-screen browser has no view of its own to receive them.
//!
//! `[webview.cef] style = "chrome"` (or [`nd_webview::CefStyle::Chrome`] per page) hosts the page
//! in Chrome style instead, for Chromium's extension runtime and `chrome://` pages: a windowed
//! browser whose native view is a child of the page's own (`mac/chrome.rs`, `linux/chrome.rs`),
//! with Chrome's browser UI hidden and its commands routed to the app (`commands.rs`).
//!
//! - macOS: every frame arrives as an IOSurface, is copied into a pool the view owns and shown
//!   as its layer's contents. Frames are paced by the window's display link through external
//!   begin frames, so a page draws at the display's rate and an idle page draws nothing.
//! - Linux: there is no OS web engine a GPUI window on Wayland can embed, so Chromium is also
//!   the system engine there. Frames arrive as pixel buffers and are drawn as an image in the
//!   page's GPUI element, in the window's own frame.
//!
//! Chromium starts with the first page and runs on the app's main loop (`external_message_pump`).
//!
//! Where the runtime comes from:
//! - macOS packaged (`nd package mac`): `<App>.app/Contents/Frameworks/Chromium Embedded
//!   Framework.framework` and the five `<App> Helper*.app` bundles, each running `nd-cef-helper`.
//! - macOS unbundled dev runs (`cargo run`): the framework from `ND_CEF_FRAMEWORK_DIR` or the
//!   directory cef-dll-sys unpacked at build time.
//! - Linux: `libcef.so` and its resources next to the executable (cef-dll-sys copies them into
//!   the target dir; `nd package linux` and the Nix package put them beside the binary), or in
//!   `ND_CEF_DIR`.
//!
//! In both, a dev run is its own helper: Chromium launches the app's executable with `--type=`
//! for every subprocess, and [`register`], which runs first in `nd::Application::run`, turns the
//! process into that subprocess before any app code runs.

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod a11y;
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod client;
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod commands;
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod crx;
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod devtools;
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod extensions;
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod keys;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod mac;
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod menu;
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod native_messaging;
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod origin;
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod page;
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod pipe;
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod runtime;
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod scheme;
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod store_browser;

#[cfg(target_os = "linux")]
use linux as platform;
#[cfg(target_os = "macos")]
use mac as platform;
#[cfg(any(target_os = "macos", target_os = "linux"))]
use platform::pump;
#[cfg(any(target_os = "macos", target_os = "linux"))]
use runtime::{SCHEMES_SWITCH, context, text};

/// Registers the Chromium engine with `nd-webview`. `nd` calls it at launch when the app has a
/// web view engine that needs it; Chromium itself starts with the first page. In a Chromium
/// subprocess (a dev run, where the app is its own helper) this runs the subprocess and exits.
pub fn register(app_name: &str) {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    runtime::register(app_name);
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = app_name;
        nd_webview::register_engine(nd_webview::Engine::Cef, Unavailable);
    }
}

/// The helper binary's entry: runs the Chromium subprocess named on the command line and
/// returns its exit code.
pub fn run_helper() -> i32 {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    return runtime::run_helper();
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    return 1;
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
struct Unavailable;

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
impl nd_webview::EngineFactory for Unavailable {
    fn create(&self, _page: nd_webview::Page) -> Result<std::rc::Rc<dyn nd_webview::EngineView>, String> {
        Err("the Chromium engine runs on macOS and Linux so far; Windows is not written yet".into())
    }
}
