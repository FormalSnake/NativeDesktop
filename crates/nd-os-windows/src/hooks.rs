//! [`WindowsHooks`]: nd-os-windows behind the GPUI backend's seam (`nd_backend_gpui::OsHooks`).
//!
//! - Every window draws its own title row over Mica: GPUI's title bar is hidden, the root paints
//!   no surface, and DWM's Mica shows wherever the app does not fill (WinUI's default).
//! - The caption buttons are drawn by the app on every window, flush with the top-right corner
//!   at Fluent metrics, and [`Chrome`] answers `WM_NCHITTEST` for them, so they behave as the
//!   system's: Snap Layouts on the maximize button, the system's move and size loops. A
//!   `window_controls()` node therefore draws nothing on Windows: the buttons have one place.
//! - The title row reserves the buttons' width as its trailing inset.
//!
//! `ND_DEBUG_EXPOSE=1` keeps the root opaque, so the resize gate's magenta stays covered.
//! `ND_DIAG_WINDOW=1` prints `nd: pen <phase> pressure=<p>` for pen contacts from `WM_POINTER`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use nd_backend_gpui::{ChromeContext, Look, OsHooks};
use nd_core::WindowId;
use nd_gpui::gpui::{
    self, AnyElement, IntoElement, ParentElement, Styled, WindowAppearance, WindowBackgroundAppearance, div, px,
};
use nd_ui::TitlebarInsets;

use crate::backdrop::{self, Backdrop};
use crate::caption::{CaptionButton, CaptionLayout, CaptionState, FLUENT_BUTTON_WIDTH, FLUENT_TITLE_HEIGHT};
use crate::chrome::{Chrome, ChromeEvent};
use crate::pointer::PointerPhase;

pub struct WindowsHooks {
    expose: bool,
    diag: bool,
    chrome: RefCell<HashMap<WindowId, Rc<Chrome>>>,
}

impl Default for WindowsHooks {
    fn default() -> Self {
        let flag = |name: &str| std::env::var_os(name).is_some_and(|v| v == "1");
        Self { expose: flag("ND_DEBUG_EXPOSE"), diag: flag("ND_DIAG_WINDOW"), chrome: RefCell::default() }
    }
}

impl WindowsHooks {
    pub fn new() -> Self {
        Self::default()
    }

    fn chrome(&self, window: WindowId) -> Option<Rc<Chrome>> {
        self.chrome.borrow().get(&window).cloned()
    }

    /// The title row for the window's current size; empty in full screen, where the app gets
    /// the whole window.
    fn layout(gpui_window: &gpui::Window) -> CaptionLayout {
        if gpui_window.is_fullscreen() {
            return CaptionLayout::default();
        }
        let mut layout = CaptionLayout::fluent(gpui_window.viewport_size().width.as_f32(), FLUENT_TITLE_HEIGHT);
        layout.title_drags = false;
        layout
    }
}

fn dark(gpui_window: &gpui::Window) -> bool {
    matches!(gpui_window.appearance(), WindowAppearance::Dark | WindowAppearance::VibrantDark)
}

impl OsHooks for WindowsHooks {
    fn window_options(&self, _window: WindowId, options: &mut gpui::WindowOptions) {
        let titlebar = options.titlebar.get_or_insert_with(Default::default);
        titlebar.appears_transparent = true;
        // Non-opaque makes GPUI clear to transparent; `window_opened` sets the material itself.
        options.window_background = WindowBackgroundAppearance::MicaBackdrop;
    }

    fn window_opened(&self, window: WindowId, gpui_window: &mut gpui::Window, cx: &mut gpui::App) {
        let chrome = match Chrome::install(gpui_window, Self::layout(gpui_window)) {
            Ok(chrome) => chrome,
            Err(e) => {
                eprintln!("nd: window {} has no Windows chrome: {e}", window.0);
                return;
            }
        };
        if let Err(e) = backdrop::apply(chrome.hwnd(), Backdrop::Mica, dark(gpui_window)) {
            eprintln!("nd: Mica unavailable: {e}");
        }
        let handle = gpui_window.window_handle();
        let async_cx = cx.to_async();
        let diag = self.diag;
        chrome.on_event(move |event| {
            if let ChromeEvent::Pointer(p) = event {
                if diag
                    && let Some(pen) = p.pen
                    && p.phase != PointerPhase::Update
                {
                    eprintln!("nd: pen {:?} pressure={:.2}", p.phase, pen.pressure.unwrap_or(-1.));
                }
                return;
            }
            // The window procedure may run inside one of GPUI's own calls: redraw on its next turn.
            async_cx
                .spawn(async move |cx| {
                    let _ = handle.update(cx, |_, window, _| window.refresh());
                })
                .detach();
        });
        self.chrome.borrow_mut().insert(window, Rc::new(chrome));
    }

    fn titlebar_insets(&self, window: WindowId, gpui_window: &gpui::Window) -> Option<TitlebarInsets> {
        let layout = Self::layout(gpui_window);
        let insets = TitlebarInsets { leading: 0., trailing: layout.trailing_inset(), height: layout.title_height };
        if let Some(chrome) = self.chrome(window) {
            chrome.set_layout(layout);
        }
        Some(insets)
    }

    fn window_controls(&self, _cx: &ChromeContext, _gpui_window: &mut gpui::Window) -> Option<AnyElement> {
        Some(div().absolute().into_any_element())
    }

    fn window_overlay(&self, window: WindowId, look: Look, gpui_window: &mut gpui::Window) -> Option<AnyElement> {
        let chrome = self.chrome(window)?;
        if gpui_window.is_fullscreen() {
            return None;
        }
        let state = chrome.caption_state();
        let maximized = gpui_window.is_maximized();
        let row = div()
            .absolute()
            .top_0()
            .right_0()
            .h(px(FLUENT_TITLE_HEIGHT))
            .flex()
            .flex_row()
            .child(caption_button(CaptionButton::Minimize, state, look, maximized))
            .child(caption_button(CaptionButton::Maximize, state, look, maximized))
            .child(caption_button(CaptionButton::Close, state, look, maximized));
        Some(row.into_any_element())
    }

    fn backdrop(&self, _window: WindowId) -> bool {
        !self.expose
    }

    fn window_closed(&self, window: WindowId) {
        self.chrome.borrow_mut().remove(&window);
    }
}

/// One Fluent caption button: Segoe Fluent Icons glyph, subtle fill on hover and press, red
/// behind close.
fn caption_button(which: CaptionButton, state: CaptionState, look: Look, maximized: bool) -> impl IntoElement {
    let hovered = state.hovered == Some(which);
    let pressed = state.pressed == Some(which);
    let close = which == CaptionButton::Close;
    let (bg, fg) = match (close, pressed, hovered) {
        (true, true, _) => (look.close_hover.opacity(0.9), gpui::white()),
        (true, false, true) => (look.close_hover, gpui::white()),
        (false, true, _) => (look.control_active, look.text),
        (false, false, true) => (look.control_hover, look.text),
        _ => (gpui::transparent_black(), look.text),
    };
    let glyph = match which {
        CaptionButton::Minimize => "\u{E921}",
        CaptionButton::Maximize if maximized => "\u{E923}",
        CaptionButton::Maximize => "\u{E922}",
        CaptionButton::Close => "\u{E8BB}",
    };
    div()
        .w(px(FLUENT_BUTTON_WIDTH))
        .h_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(bg)
        .text_color(fg)
        .font_family("Segoe Fluent Icons")
        .text_size(px(10.))
        .child(glyph)
}
