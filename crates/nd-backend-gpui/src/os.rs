//! The seam `nd-os-linux` and `nd-os-windows` plug into (ARCHITECTURE.md 2, 5). GPUI owns the OS
//! connections, so an OS crate never opens a window or a protocol object of its own: it adjusts
//! the window GPUI opens, reports the room the OS controls take, may replace the drawn controls,
//! and may supply each frame's display time where the OS measures presentation
//! (`wp_presentation`, a per-monitor DXGI or compositor clock). Every hook has a default that is
//! right for a plain GPUI window, so [`NoOsHooks`] is a complete implementation.
//!
//! An OS crate implements [`OsHooks`] and hands it to [`crate::GpuiHost::builder`] (the `nd`
//! facade does this per target).

use nd_core::time::Instant;

use nd_core::{FrameTime, Point, WindowId};
use nd_gpui::gpui::{self, AnyElement};
use nd_ui::TitlebarInsets;

use crate::render::Look;

/// What a `window_controls()` node asks for, handed to [`OsHooks::window_controls`].
pub struct ChromeContext {
    pub window: WindowId,
    /// Pinned position (`window_controls().at(..)`); `None` means the OS side.
    pub at: Option<Point>,
    pub look: Look,
}

/// One frame about to be drawn, handed to [`OsHooks::frame_time`].
pub struct FrameRequest {
    pub window: WindowId,
    /// When the backend started this frame.
    pub now: Instant,
    /// `now` on the clock's scale: seconds since the app started, which `FrameTime` counts in.
    pub now_seconds: f64,
    /// The display interval GPUI reports for the window's current display, in seconds.
    pub interval: f64,
    /// The clock's previous frame.
    pub last: FrameTime,
}

pub trait OsHooks: 'static {
    /// GPUI's app is up and the nd app is about to be built: fonts and app-wide platform state
    /// that windows read are set up here (the web registers its bundled fonts).
    fn app_started(&self, _cx: &mut gpui::App) {}

    /// Adjusts the options of a window before GPUI opens it: decorations, backdrop, app id,
    /// titlebar. The backend has set title, size, `inactive_frame_interval: None` (5.3: only
    /// hidden windows are throttled) and a transparent titlebar where `titlebar()` draws one.
    fn window_options(&self, _window: WindowId, _options: &mut gpui::WindowOptions) {}

    /// The window is open and about to draw its first frame. Backdrops (Mica), caption hit
    /// testing and per-window pacing sources are set up here.
    fn window_opened(&self, _window: WindowId, _gpui_window: &mut gpui::Window, _cx: &mut gpui::App) {}

    /// Where the OS controls sit. `None` keeps the backend's answer: traffic lights leading on
    /// macOS, drawn controls trailing under client-side decorations, nothing under server-side
    /// decorations (Hyprland).
    fn titlebar_insets(&self, _window: WindowId, _gpui_window: &gpui::Window) -> Option<TitlebarInsets> {
        None
    }

    /// Replaces the controls a `window_controls()` node draws. `None` keeps the backend's
    /// ([`crate::render::drawn_controls`] under client-side decorations, nothing otherwise).
    fn window_controls(&self, _cx: &ChromeContext, _gpui_window: &mut gpui::Window) -> Option<AnyElement> {
        None
    }

    /// The time of the frame about to be drawn. `None` uses the backend's estimate: now, with
    /// the target one display interval later. An OS crate that measures presentation returns
    /// the measured previous present as `timestamp` and its prediction as `target`.
    fn frame_time(&self, _frame: &FrameRequest, _gpui_window: &gpui::Window) -> Option<FrameTime> {
        None
    }

    /// The appearance the window draws in. `None` follows the OS (`Window::appearance`); tests
    /// pin light or dark here.
    fn appearance(&self, _window: WindowId, _gpui_window: &gpui::Window) -> Option<gpui::WindowAppearance> {
        None
    }

    /// The window closed; per-window state can go.
    fn window_closed(&self, _window: WindowId) {}
}

/// Plain GPUI windows: every hook at its default.
pub struct NoOsHooks;

impl OsHooks for NoOsHooks {}
