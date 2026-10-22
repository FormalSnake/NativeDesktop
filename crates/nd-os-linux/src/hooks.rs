//! [`LinuxHooks`]: nd-os-linux behind the GPUI backend's seam (`nd_backend_gpui::OsHooks`).
//!
//! - Windows ask for server-side decorations and carry the app id, so compositors that match
//!   rules on it (Hyprland's `class`) see the app's name.
//! - The titlebar insets and the drawn controls follow [`crate::chrome::plan`]: nothing at all
//!   under server-side decorations (Hyprland), and under client-side ones only the controls
//!   `wm_capabilities` offers, on the sides the desktop's `button-layout` puts them. An embedded
//!   `window_controls()` slot draws its group where it sits (a split header's sidebar and
//!   content ends); the window overlay draws every group no slot took at the header bar's ends,
//!   so a window without a slot still has its close button, as every libadwaita window does.
//!   The layout is `org.gnome.desktop.wm.preferences button-layout`, GNOME's `appmenu:close`
//!   where it cannot be read.
//! - Frame times come from `wp_presentation` (the fork's `Window::last_presentation`): the
//!   timestamp is when the previous frame turned into light and the target is the next refresh
//!   after now on the compositor's predicted cadence, rather than "now plus one interval".
//!   XWayland and X11 windows have no presentation feedback and keep the backend's estimate.
//!
//! `ND_DIAG_WINDOW=1` prints `nd: chrome window=<id> <chrome::describe>` whenever a window's
//! chrome changes.

use std::cell::RefCell;
use std::collections::HashMap;

use nd_backend_gpui::{ChromeContext, FrameRequest, Look, OsHooks, icon_glyph};
use nd_core::{FrameTime, WindowId};
use nd_gpui::gpui::{
    self, AnyElement, InteractiveElement, IntoElement, MouseButton, ParentElement, StatefulInteractiveElement, Styled,
    div, px,
};
use nd_icons::Icon;
use nd_theme::color::with_alpha;
use nd_ui::{TitlebarInsets, WindowControlsSide};

use crate::chrome::{self, ButtonLayout, Control, ControlMetrics};
use crate::window::{chrome_state, perform, requested_decorations};

/// The header bar height the controls centre in, libadwaita's 46 px.
pub const TITLEBAR_HEIGHT: f32 = 46.;

pub struct LinuxHooks {
    app_id: String,
    metrics: ControlMetrics,
    diag: bool,
    /// Read when a window opens; the hooks that draw are not handed GPUI's app.
    layout: RefCell<ButtonLayout>,
    reported: RefCell<HashMap<WindowId, String>>,
    /// The groups embedded slots took, per window: in the frame being built, and in the last
    /// one, which the insets (asked before the build) go by.
    claimed: RefCell<HashMap<WindowId, (Claimed, Claimed)>>,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct Claimed {
    leading: bool,
    trailing: bool,
}

impl LinuxHooks {
    pub fn new(app_id: impl Into<String>) -> Self {
        Self {
            app_id: app_id.into(),
            metrics: ControlMetrics::default(),
            diag: std::env::var_os("ND_DIAG_WINDOW").is_some_and(|v| v == "1"),
            layout: RefCell::default(),
            reported: RefCell::default(),
            claimed: RefCell::default(),
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

    /// The groups no slot took in the last frame, at the header bar's ends.
    fn unclaimed(&self, window: WindowId, plan: &chrome::ChromePlan) -> (Vec<Control>, Vec<Control>) {
        let last = self.claimed.borrow().get(&window).map(|c| c.1).unwrap_or_default();
        let keep = |claimed: bool, side: &[Control]| if claimed { Vec::new() } else { side.to_vec() };
        (keep(last.leading, &plan.leading), keep(last.trailing, &plan.trailing))
    }

    /// One group of the plan's buttons in a row the header bar's height.
    fn controls(&self, controls: &[Control], look: Look) -> gpui::Div {
        let m = self.metrics;
        // libadwaita's window buttons: a circle of the text colour at 10% (15% hovered, 30%
        // pressed) behind a 16 px symbolic icon, close included.
        let tint = move |share: f32| with_alpha(look.text, look.text.a * share);
        let button = move |control: Control| {
            let icon = match control {
                Control::Minimize => Icon::Minus,
                Control::Maximize => Icon::Square,
                Control::Close => Icon::X,
            };
            let (hover, active) = (tint(0.15), tint(0.3));
            div()
                .id(control.name())
                .size(px(m.size))
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(tint(0.1))
                .hover(move |s| s.bg(hover))
                .active(move |s| s.bg(active))
                .occlude()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(move |_, window, _| perform(control, window))
                .child(icon_glyph(icon, px(12.), look.text, px(2.)))
        };
        div()
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .h(px(TITLEBAR_HEIGHT))
            .px(px(m.padding))
            .gap(px(m.spacing))
            .children(controls.iter().map(|c| button(*c)))
    }
}

impl OsHooks for LinuxHooks {
    fn window_options(&self, _window: WindowId, options: &mut gpui::WindowOptions) {
        options.window_decorations = Some(requested_decorations());
        if options.app_id.is_none() {
            options.app_id = Some(self.app_id.clone());
        }
    }

    /// GNOME's "Animations" switch (`org.gnome.desktop.interface enable-animations`), which
    /// libadwaita honours; a desktop without the schema gives no answer.
    fn reduce_motion(&self) -> Option<bool> {
        match interface_setting("enable-animations")?.as_str() {
            "false" => Some(true),
            "true" => Some(false),
            _ => None,
        }
    }

    /// GNOME's accent colour (`org.gnome.desktop.interface accent-color`), which libadwaita
    /// apps take; a matugen palette or `theme.toml` above it in the chain still wins.
    fn os_palettes(&self) -> nd_theme::SchemePalettes {
        interface_setting("accent-color").and_then(|name| accent_palettes(&name)).unwrap_or_default()
    }

    fn window_opened(&self, _window: WindowId, _gpui_window: &mut gpui::Window, _cx: &mut gpui::App) {
        *self.layout.borrow_mut() = button_layout();
    }

    fn titlebar_insets(&self, window: WindowId, gpui_window: &gpui::Window) -> Option<TitlebarInsets> {
        let plan = self.plan(window, gpui_window);
        if plan.is_empty() {
            return Some(TitlebarInsets::default());
        }
        let last = self.claimed.borrow().get(&window).map(|c| c.1).unwrap_or_default();
        let inset = |claimed: bool, inset: f32| if claimed { 0. } else { inset };
        Some(TitlebarInsets {
            leading: inset(last.leading, plan.leading_inset),
            trailing: inset(last.trailing, plan.trailing_inset),
            height: TITLEBAR_HEIGHT,
        })
    }

    /// An embedded slot draws its group in place; a pinned or OS-side node leaves the
    /// controls to the overlay, which keeps them at the header bar's ends.
    fn window_controls(&self, cx: &ChromeContext, gpui_window: &mut gpui::Window) -> Option<AnyElement> {
        if !cx.embedded {
            return Some(div().absolute().into_any_element());
        }
        let (leading, trailing) = match cx.side {
            Some(WindowControlsSide::Leading) => (true, false),
            Some(WindowControlsSide::Trailing) => (false, true),
            None => (true, true),
        };
        let mut claimed = self.claimed.borrow_mut();
        let now = &mut claimed.entry(cx.window).or_default().0;
        now.leading |= leading;
        now.trailing |= trailing;
        drop(claimed);
        let plan = self.plan(cx.window, gpui_window);
        let mut group = Vec::new();
        if leading {
            group.extend(&plan.leading);
        }
        if trailing {
            group.extend(&plan.trailing);
        }
        Some(self.controls(&group, cx.look).into_any_element())
    }

    fn window_overlay(
        &self,
        window: WindowId,
        look: Look,
        _title_height: f32,
        gpui_window: &mut gpui::Window,
    ) -> Option<AnyElement> {
        // The overlay is asked after the build, so this frame's slots are all in.
        if let Some((now, last)) = self.claimed.borrow_mut().get_mut(&window) {
            *last = std::mem::take(now);
        }
        let plan = self.plan(window, gpui_window);
        let (leading, trailing) = self.unclaimed(window, &plan);
        if leading.is_empty() && trailing.is_empty() {
            return None;
        }
        Some(
            div()
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .flex()
                .flex_row()
                .justify_between()
                .child(self.controls(&leading, look))
                .child(self.controls(&trailing, look))
                .into_any_element(),
        )
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
        self.claimed.borrow_mut().remove(&window);
    }
}

/// GNOME's `button-layout`, the key the settings portal serves and GTK's
/// `gtk-decoration-layout` follows. GPUI reads it too, but answers Zed's
/// `minimize,maximize,close` when it is missing, where GNOME would show close alone.
fn button_layout() -> ButtonLayout {
    setting("org.gnome.desktop.wm.preferences", "button-layout").map(|l| ButtonLayout::parse(&l)).unwrap_or_default()
}

fn interface_setting(key: &str) -> Option<String> {
    setting("org.gnome.desktop.interface", key)
}

/// One gsettings key, unquoted; `None` without the schema.
fn setting(schema: &str, key: &str) -> Option<String> {
    let out = std::process::Command::new("gsettings")
        .args(["get", schema, key])
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    let value = String::from_utf8_lossy(&out.stdout).trim().trim_matches('\'').to_owned();
    (out.status.success() && !value.is_empty()).then_some(value)
}

/// libadwaita's accent for GNOME's accent name (`_colors.scss`), as the accent fill with white
/// on it, and its 25% over the view as the selection and the container (40% on dark).
fn accent_palettes(name: &str) -> Option<nd_theme::SchemePalettes> {
    use nd_theme::color::{over, parse_hex};
    let hex = match name {
        "blue" => "#3584e4",
        "teal" => "#2190a4",
        "green" => "#3a944a",
        "yellow" => "#c88800",
        "orange" => "#ed5b00",
        "red" => "#e62d42",
        "pink" => "#d56199",
        "purple" => "#9141ac",
        "slate" => "#6f8396",
        _ => return None,
    };
    let accent = parse_hex(hex)?;
    let scheme = |view: &str, share: f32, ink: &str| {
        let tint = with_alpha(accent, share);
        nd_theme::PaletteRefinement {
            accent: Some(accent),
            on_accent: parse_hex("#ffffff"),
            selection: Some(tint),
            accent_container: parse_hex(view).map(|v| over(tint, v)),
            on_accent_container: parse_hex(ink),
            ..Default::default()
        }
    };
    Some(nd_theme::SchemePalettes {
        light: scheme("#ffffff", 0.25, "#000006cc"),
        dark: scheme("#1d1d20", 0.4, "#ffffff"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gnome_accent_names_map_to_libadwaitas_colours() {
        let teal = accent_palettes("teal").expect("teal");
        assert_eq!(teal.light.accent, nd_theme::color::parse_hex("#2190a4"));
        assert_eq!(teal.dark.accent, teal.light.accent);
        assert!(teal.light.accent_container.is_some_and(|c| c.a > 0.99), "the container is opaque");
        assert!(accent_palettes("chartreuse").is_none());
    }

    #[test]
    fn windows_carry_the_hooks_app_id() {
        let hooks = LinuxHooks::new("dev.example.app");
        let mut options = gpui::WindowOptions::default();
        hooks.window_options(WindowId(1), &mut options);
        assert_eq!(options.app_id.as_deref(), Some("dev.example.app"));
    }

    #[test]
    fn an_app_id_already_on_the_window_wins() {
        let hooks = LinuxHooks::new("dev.example.app");
        let mut options = gpui::WindowOptions { app_id: Some("other".into()), ..Default::default() };
        hooks.window_options(WindowId(1), &mut options);
        assert_eq!(options.app_id.as_deref(), Some("other"));
    }
}
