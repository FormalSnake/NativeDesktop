//! Window chrome on Linux (ARCHITECTURE.md 3.6, 5.4): which window controls the app draws and
//! where. The compositor decides through xdg-decoration and `xdg_toplevel.wm_capabilities`; the
//! desktop's `button-layout` decides the sides. Nothing here looks at `$XDG_CURRENT_DESKTOP`.
//!
//! - Server-side decorations: the compositor owns the frame, so no controls and no insets. Hyprland
//!   always answers server-side (it ignores a client-side request), so on Hyprland this is the
//!   only outcome.
//! - Client-side decorations: close always, minimize and maximize only when `wm_capabilities`
//!   lists them. A compositor that never sends `wm_capabilities` (xdg-shell before v5) supports
//!   everything, which is what [`Capabilities::default`] says.
//! - Full screen: no controls.

/// The decoration mode the compositor configured, from `zxdg_toplevel_decoration_v1.configure`.
/// Without the protocol (GNOME Mutter, Weston) the client draws its own, so it is `Client`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decorations {
    Server,
    Client,
}

/// `xdg_toplevel.wm_capabilities`. Close is not in the protocol's set: it is always available.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capabilities {
    pub maximize: bool,
    pub minimize: bool,
    pub fullscreen: bool,
    pub window_menu: bool,
}

impl Default for Capabilities {
    fn default() -> Self {
        Self { maximize: true, minimize: true, fullscreen: true, window_menu: true }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Control {
    Minimize,
    Maximize,
    Close,
}

impl Control {
    pub fn name(self) -> &'static str {
        match self {
            Control::Minimize => "minimize",
            Control::Maximize => "maximize",
            Control::Close => "close",
        }
    }
}

/// GNOME's `button-layout` (`org.gnome.desktop.wm.preferences`): `"appmenu:minimize,maximize,close"`.
/// Names before the colon go on the leading side. Unknown names (`appmenu`, `icon`, `spacer`) and
/// repeats are skipped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ButtonLayout {
    pub leading: Vec<Control>,
    pub trailing: Vec<Control>,
}

impl Default for ButtonLayout {
    /// GNOME's own default, `appmenu:close`.
    fn default() -> Self {
        Self { leading: Vec::new(), trailing: vec![Control::Close] }
    }
}

impl ButtonLayout {
    pub fn parse(layout: &str) -> Self {
        let (leading, trailing) = layout.split_once(':').unwrap_or(("", layout));
        let mut seen = Vec::new();
        let mut side = |names: &str| {
            names
                .split(',')
                .filter_map(|name| match name.trim() {
                    "minimize" => Some(Control::Minimize),
                    "maximize" => Some(Control::Maximize),
                    "close" => Some(Control::Close),
                    _ => None,
                })
                .filter(|c| {
                    let new = !seen.contains(c);
                    seen.push(*c);
                    new
                })
                .collect::<Vec<_>>()
        };
        let leading = side(leading);
        let trailing = side(trailing);
        Self { leading, trailing }
    }
}

/// Everything the plan depends on, read from the window each frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChromeState {
    pub decorations: Decorations,
    pub capabilities: Capabilities,
    pub layout: ButtonLayout,
    pub fullscreen: bool,
}

/// Size of one drawn control and the gap between controls, in logical pixels. libadwaita's
/// titlebar buttons are 24 px circles with 9 px spacing, and the header bar pads 6 px.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ControlMetrics {
    pub size: f32,
    pub spacing: f32,
    pub padding: f32,
}

impl Default for ControlMetrics {
    fn default() -> Self {
        Self { size: 24., spacing: 9., padding: 6. }
    }
}

/// What the app draws: the controls per side, in order, and the space a custom titlebar leaves
/// for them (the `titlebar()` insets).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChromePlan {
    pub leading: Vec<Control>,
    pub trailing: Vec<Control>,
    pub leading_inset: f32,
    pub trailing_inset: f32,
}

impl ChromePlan {
    pub fn is_empty(&self) -> bool {
        self.leading.is_empty() && self.trailing.is_empty()
    }

    pub fn controls(&self) -> impl Iterator<Item = Control> + '_ {
        self.leading.iter().chain(&self.trailing).copied()
    }
}

pub fn plan(state: &ChromeState, metrics: ControlMetrics) -> ChromePlan {
    if state.decorations == Decorations::Server || state.fullscreen {
        return ChromePlan::default();
    }
    let caps = state.capabilities;
    let allowed = |c: &Control| match c {
        Control::Close => true,
        Control::Minimize => caps.minimize,
        Control::Maximize => caps.maximize,
    };
    let leading: Vec<Control> = state.layout.leading.iter().copied().filter(allowed).collect();
    let trailing: Vec<Control> = state.layout.trailing.iter().copied().filter(allowed).collect();
    let inset = |n: usize| {
        if n == 0 { 0. } else { metrics.padding * 2. + metrics.size * n as f32 + metrics.spacing * (n - 1) as f32 }
    };
    ChromePlan { leading_inset: inset(leading.len()), trailing_inset: inset(trailing.len()), leading, trailing }
}

/// One line for logs and tier 3 tests: `decorations=server controls=none caps=maximize,fullscreen`.
pub fn describe(state: &ChromeState, plan: &ChromePlan) -> String {
    let decorations = match state.decorations {
        Decorations::Server => "server",
        Decorations::Client => "client",
    };
    let controls = if plan.is_empty() {
        "none".to_owned()
    } else {
        let side = |c: &[Control]| c.iter().map(|c| c.name()).collect::<Vec<_>>().join(",");
        format!("{}:{}", side(&plan.leading), side(&plan.trailing))
    };
    let c = state.capabilities;
    let caps = [(c.maximize, "maximize"), (c.minimize, "minimize"), (c.fullscreen, "fullscreen"), (c.window_menu, "window_menu")]
        .iter()
        .filter(|(on, _)| *on)
        .map(|(_, name)| *name)
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "decorations={decorations} controls={controls} caps={} fullscreen={} insets={},{}",
        if caps.is_empty() { "none" } else { &caps },
        state.fullscreen,
        plan.leading_inset,
        plan.trailing_inset
    )
}
