//! Browser services for the web target (ARCHITECTURE.md 12): GPUI's web platform draws the app
//! into one canvas, and this crate fills what a browser tab offers in place of an OS. Fonts are
//! bundled (a page has no system fonts GPUI can read), file pickers are a hidden
//! `<input type=file>` and saving is a download, alerts are drawn sheets and the menu bar is a
//! drawn row above the windows. The clipboard is GPUI's own (the async Clipboard API) through
//! the backend, and settings and stores keep their files in `localStorage` (`nd-settings`). The `nd` facade installs it on `wasm32`; elsewhere this crate
//! is empty.

#![cfg(target_arch = "wasm32")]

pub mod chrome;
pub mod files;
mod fonts;
pub mod storage;

pub use chrome::WebChrome;

use nd_backend_gpui::OsHooks;
use nd_gpui::gpui;

/// The web's [`OsHooks`]: registers the bundled fonts before the first window lays out text.
pub struct WebHooks;

impl OsHooks for WebHooks {
    fn app_started(&self, cx: &mut gpui::App) {
        fonts::register(cx);
    }
}

/// Where a web app's diagnostics go: the browser console.
pub(crate) fn console_error(message: &str) {
    web_sys::console::error_1(&message.into());
}
