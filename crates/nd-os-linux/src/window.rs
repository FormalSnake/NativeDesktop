//! The chrome policy read from a live GPUI window, and the OS actions behind drag regions and
//! drawn controls. GPUI owns the Wayland toplevel: it binds `zxdg_toplevel_decoration_v1` at
//! creation, sends `set_mode` for the decorations the window was opened with, and stores the
//! compositor's `configure` answer and `wm_capabilities`. This module only reads those back.

use nd_gpui::gpui::{
    self, App, InteractiveElement, MouseButton, MouseDownEvent, Window, WindowButton, WindowButtonLayout,
    WindowDecorations,
};

use crate::chrome::{ButtonLayout, Capabilities, ChromePlan, ChromeState, Control, ControlMetrics, Decorations};

/// What nd asks the compositor for. Server-side wherever the compositor offers it; the answer in
/// `configure` wins either way, and [`chrome_state`] reports that answer, never this request.
pub const REQUESTED_DECORATIONS: WindowDecorations = WindowDecorations::Server;

pub fn chrome_state(window: &Window, cx: &App) -> ChromeState {
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
        layout: cx.button_layout().map(button_layout).unwrap_or_default(),
        fullscreen: window.is_fullscreen(),
    }
}

pub fn chrome_plan(window: &Window, cx: &App) -> ChromePlan {
    crate::chrome::plan(&chrome_state(window, cx), ControlMetrics::default())
}

fn button_layout(layout: WindowButtonLayout) -> ButtonLayout {
    let side = |buttons: &[Option<WindowButton>]| {
        buttons
            .iter()
            .flatten()
            .map(|b| match b {
                WindowButton::Minimize => Control::Minimize,
                WindowButton::Maximize => Control::Maximize,
                WindowButton::Close => Control::Close,
            })
            .collect()
    };
    ButtonLayout { leading: side(&layout.left), trailing: side(&layout.right) }
}

/// Makes `element` a drag region (ARCHITECTURE.md 5.4): a primary press starts the compositor's
/// interactive move (`xdg_toplevel.move`, which needs the serial of this very press, so it runs
/// inside the mouse-down handler), and a double click toggles maximize when the compositor
/// supports it. Interactive children that stop propagation occlude it.
pub fn drag_region<E: InteractiveElement>(element: E) -> E {
    element.on_mouse_down(MouseButton::Left, |event: &MouseDownEvent, window: &mut Window, _cx: &mut App| {
        if event.click_count == 2 {
            if window.window_controls().maximize {
                window.zoom_window();
            }
        } else {
            window.start_window_move();
        }
    })
}

/// The OS action of a drawn control.
pub fn perform(control: Control, window: &mut Window) {
    match control {
        Control::Minimize => window.minimize_window(),
        Control::Maximize => window.zoom_window(),
        Control::Close => window.remove_window(),
    }
}
