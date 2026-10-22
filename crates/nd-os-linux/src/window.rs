//! The chrome policy read from a live GPUI window, and the OS action behind each drawn control.
//! GPUI owns the Wayland toplevel: it binds `zxdg_toplevel_decoration_v1` at creation, sends
//! `set_mode` for the decorations the window was opened with, and stores the compositor's
//! `configure` answer and `wm_capabilities`. This module only reads those back.

use nd_gpui::gpui::{self, Window, WindowDecorations};

use crate::chrome::{ButtonLayout, Capabilities, ChromeState, Control, Decorations};

/// What nd asks the compositor for. Server-side wherever the compositor offers it, unless
/// `ND_CLIENT_DECORATIONS=1` asks for the frame GNOME gets (docs captures, chrome checks under a
/// compositor that would otherwise decorate). The answer in `configure` wins either way, and
/// [`chrome_state`] reports that answer, never this request.
pub fn requested_decorations() -> WindowDecorations {
    match std::env::var_os("ND_CLIENT_DECORATIONS").is_some_and(|v| v == "1") {
        true => WindowDecorations::Client,
        false => WindowDecorations::Server,
    }
}

pub fn chrome_state(window: &Window, layout: &ButtonLayout) -> ChromeState {
    let decorations = match window.window_decorations() {
        gpui::Decorations::Server => Decorations::Server,
        gpui::Decorations::Client { .. } => Decorations::Client,
    };
    let c = window.window_controls();
    ChromeState {
        decorations,
        capabilities: Capabilities {
            maximize: c.maximize,
            minimize: c.minimize,
            fullscreen: c.fullscreen,
            window_menu: c.window_menu,
        },
        layout: layout.clone(),
        fullscreen: window.is_fullscreen(),
    }
}

/// The OS action of a drawn control.
pub fn perform(control: Control, window: &mut Window) {
    match control {
        Control::Minimize => window.minimize_window(),
        Control::Maximize => window.zoom_window(),
        Control::Close => window.remove_window(),
    }
}
