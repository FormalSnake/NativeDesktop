//! [`LinuxHooks`]: nd-os-linux behind the GPUI backend's seam (`nd_backend_gpui::OsHooks`).
//!
//! - Windows ask for server-side decorations and carry the app id, so compositors that match
//!   rules on it (Hyprland's `class`) see the app's name.
//! - The titlebar insets and the drawn controls follow [`crate::chrome::plan`]: nothing at all
//!   under server-side decorations (Hyprland), and under client-side ones only the controls
//!   `wm_capabilities` offers, on the sides the desktop's `button-layout` puts them.
//! - Frame times come from `wp_presentation` (the fork's `Window::last_presentation`): the
//!   timestamp is when the previous frame turned into light and the target is the next refresh
//!   after now on the compositor's predicted cadence, rather than "now plus one interval".
//!   XWayland and X11 windows have no presentation feedback and keep the backend's estimate.
//!
//! `ND_DIAG_WINDOW=1` prints `nd: chrome window=<id> <chrome::describe>` whenever a window's
//! chrome changes.

use std::cell::RefCell;
use std::collections::HashMap;

use nd_backend_gpui::{ChromeContext, FrameRequest, OsHooks};
use nd_core::{FrameTime, WindowId};
use nd_gpui::gpui::{
    self, AnyElement, InteractiveElement, IntoElement, MouseButton, ParentElement, StatefulInteractiveElement, Styled,
    div, px,
};
use nd_ui::TitlebarInsets;

use crate::chrome::{self, ButtonLayout, Control, ControlMetrics};
use crate::window::{REQUESTED_DECORATIONS, button_layout, chrome_state, perform};

/// The header bar height the controls centre in, libadwaita's 46 px.
pub const TITLEBAR_HEIGHT: f32 = 46.;

pub struct LinuxHooks {
    app_id: String,
    metrics: ControlMetrics,
    diag: bool,
    /// Read when a window opens; the hooks that draw are not handed GPUI's app.
    layout: RefCell<ButtonLayout>,
    reported: RefCell<HashMap<WindowId, String>>,
}

impl LinuxHooks {
    pub fn new(app_id: impl Into<String>) -> Self {
        Self {
            app_id: app_id.into(),
            metrics: ControlMetrics::default(),
            diag: std::env::var_os("ND_DIAG_WINDOW").is_some_and(|v| v == "1"),
            layout: RefCell::default(),
            reported: RefCell::default(),
        }
    }

    fn plan(&self, window: WindowId, gpui_window: &gpui::Window) -> chrome::ChromePlan {
        let state = chrome_state(gpui_window, &self.layout.borrow());
        let plan = chrome::plan(&state, self.metrics);
        if self.diag {
            let line = chrome::describe(&state, &plan);
            let mut reported = self.reported.borrow_mut();
            if reported.get(&window) != Some(&line) {
                eprintln!("nd: chrome window={} {line}", window.0);
                reported.insert(window, line);
            }
        }
        plan
    }
}

impl OsHooks for LinuxHooks {
    fn window_options(&self, _window: WindowId, options: &mut gpui::WindowOptions) {
        options.window_decorations = Some(REQUESTED_DECORATIONS);
        if options.app_id.is_none() {
            options.app_id = Some(self.app_id.clone());
        }
    }

    fn window_opened(&self, _window: WindowId, _gpui_window: &mut gpui::Window, cx: &mut gpui::App) {
        *self.layout.borrow_mut() = button_layout(cx);
    }

    fn titlebar_insets(&self, window: WindowId, gpui_window: &gpui::Window) -> Option<TitlebarInsets> {
        let plan = self.plan(window, gpui_window);
        if plan.is_empty() {
            return Some(TitlebarInsets::default());
        }
        Some(TitlebarInsets { leading: plan.leading_inset, trailing: plan.trailing_inset, height: TITLEBAR_HEIGHT })
    }

    fn window_controls(&self, cx: &ChromeContext, gpui_window: &mut gpui::Window) -> Option<AnyElement> {
        let plan = self.plan(cx.window, gpui_window);
        if plan.is_empty() {
            return Some(div().absolute().into_any_element());
        }
        let m = self.metrics;
        let look = cx.look;
        let button = move |control: Control| {
            let (glyph, hover) = match control {
                Control::Minimize => ("\u{2212}", look.control_hover),
                Control::Maximize => ("\u{25a1}", look.control_hover),
                Control::Close => ("\u{00d7}", look.close_hover),
            };
            div()
                .id(control.name())
                .size(px(m.size))
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(look.control)
                .text_color(look.text)
                .hover(move |s| s.bg(hover))
                .occlude()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(move |_, window, _| perform(control, window))
                .child(glyph)
        };
        let row = |controls: &[Control]| {
            div()
                .flex()
                .flex_row()
                .items_center()
                .h(px(TITLEBAR_HEIGHT))
                .px(px(m.padding))
                .gap(px(m.spacing))
                .children(controls.iter().map(|c| button(*c)))
        };
        let element = match cx.at {
            Some(at) => {
                let all: Vec<Control> = plan.controls().collect();
                row(&all).absolute().left(px(at.x)).top(px(at.y)).into_any_element()
            }
            None => div()
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .flex()
                .flex_row()
                .justify_between()
                .child(row(&plan.leading))
                .child(row(&plan.trailing))
                .into_any_element(),
        };
        Some(element)
    }

    fn frame_time(&self, frame: &FrameRequest, gpui_window: &gpui::Window) -> Option<FrameTime> {
        let presentation = gpui_window.last_presentation()?;
        let since = frame.now.checked_duration_since(presentation.presented)?.as_secs_f64();
        let timestamp = frame.now_seconds - since;
        let interval = presentation.refresh.map_or(frame.interval, |r| r.as_secs_f64()).max(1e-4);
        // The first refresh after now on the compositor's cadence.
        let target = timestamp + interval * ((since / interval).floor() + 1.);
        Some(FrameTime { frame: 0, timestamp, target })
    }

    fn window_closed(&self, window: WindowId) {
        self.reported.borrow_mut().remove(&window);
    }
}
