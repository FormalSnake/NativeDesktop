//! Per-OS glue: the presenters each backend registers, how answers reach app code, and the
//! built-in engine.

use std::time::Duration;

use nd_ui::App;

#[cfg(target_os = "macos")]
pub mod appkit;
#[cfg(any(not(target_os = "macos"), feature = "gpui"))]
mod gpui;

#[cfg(target_os = "macos")]
pub(crate) const SYSTEM_ENGINE_MISSING: &str = "WKWebView failed to load";
#[cfg(target_os = "windows")]
pub(crate) const SYSTEM_ENGINE_MISSING: &str =
    "web views on Windows need the WebView2 engine, which nd does not implement yet (M8 Windows lane)";
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub(crate) const SYSTEM_ENGINE_MISSING: &str =
    "web views on Linux need the WPE WebKit engine, which nd does not implement yet (M8 Linux lane)";

/// Registers the presenters with the backends this build links. `webview()` calls it.
pub(crate) fn register_presenters() {
    thread_local!(static DONE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) });
    if DONE.with(|d| d.replace(true)) {
        return;
    }
    #[cfg(any(not(target_os = "macos"), feature = "gpui"))]
    gpui::register();
    #[cfg(target_os = "macos")]
    appkit::register();
}

/// Registers the engines this crate implements.
pub(crate) fn register_builtin() {
    thread_local!(static DONE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) });
    if DONE.with(|d| d.replace(true)) {
        return;
    }
    #[cfg(target_os = "macos")]
    crate::engine::register_engine(crate::Engine::System, appkit::wk::WkFactory);
}

#[cfg(target_os = "macos")]
pub(crate) fn defer(f: Box<dyn FnOnce(&mut App)>) {
    nd_backend_appkit::defer_with_app(f);
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn defer(_f: Box<dyn FnOnce(&mut App)>) {
    // No engine runs here yet, so no command ever completes.
}

#[cfg(target_os = "macos")]
pub(crate) fn after(delay: Duration, f: impl FnOnce() + 'static) {
    appkit::after(delay.as_secs_f64(), f);
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn after(_delay: Duration, _f: impl FnOnce() + 'static) {}
