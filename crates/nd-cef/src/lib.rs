//! Chromium as a web engine for `nd-webview` (ARCHITECTURE.md 8), through the `cef` crate in
//! off-screen mode: every frame arrives as an IOSurface, is copied into a pool the view owns and
//! shown as its layer's contents, so the page is an ordinary view of the window (overlays above
//! it, resized in the window's transaction) instead of a Chromium window floating over it.
//! Input, IME, `<select>` popups, context menus, drag and drop, cursors and accessibility are
//! all ours, since an off-screen browser has no view of its own to receive them.
//!
//! Chromium starts with the first page and runs on the app's main run loop
//! (`external_message_pump`). Frames are paced by the window's display link through external
//! begin frames, so a page draws at the display's rate and an idle page draws nothing.
//!
//! Where the framework and helpers come from:
//! - Packaged (`nd package mac`): `<App>.app/Contents/Frameworks/Chromium Embedded
//!   Framework.framework` and the five `<App> Helper*.app` bundles, each running `nd-cef-helper`.
//! - Unbundled dev runs (`cargo run`): the framework from `ND_CEF_FRAMEWORK_DIR` or the
//!   directory cef-dll-sys unpacked at build time, and the app's own executable as the helper for
//!   every subprocess type (Chromium picks the `(GPU)`/`(Renderer)` variants only inside a
//!   bundle). [`register`] runs first in `nd::Application::run`, sees `--type=` and turns the
//!   process into that subprocess before any app code runs.

#[cfg(target_os = "macos")]
mod mac;

/// Registers the Chromium engine with `nd-webview`. `nd` calls it at launch when its `cef`
/// feature is on; Chromium itself starts with the first page. In a Chromium subprocess (a dev
/// run, where the app is its own helper) this runs the subprocess and exits.
pub fn register(app_name: &str) {
    #[cfg(target_os = "macos")]
    mac::register(app_name);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app_name;
        nd_webview::register_engine(nd_webview::Engine::Cef, Unavailable);
    }
}

/// The helper binary's entry: runs the Chromium subprocess named on the command line and
/// returns its exit code.
pub fn run_helper() -> i32 {
    #[cfg(target_os = "macos")]
    return mac::run_helper();
    #[cfg(not(target_os = "macos"))]
    return 1;
}

#[cfg(not(target_os = "macos"))]
struct Unavailable;

#[cfg(not(target_os = "macos"))]
impl nd_webview::EngineFactory for Unavailable {
    fn create(&self, _page: nd_webview::Page) -> Result<std::rc::Rc<dyn nd_webview::EngineView>, String> {
        Err("the Chromium engine runs on macOS only so far; Linux (Wayland off-screen) and Windows are not written yet"
            .into())
    }
}
