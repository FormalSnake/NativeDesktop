//! Resolution: the palette source chain under the override layers, cached per window.
//!
//! Chain for each base colour, first hit wins (ARCHITECTURE.md 4): app or user `theme.toml`,
//! matugen `theme.json`, the OS (appearance, accent, semantic colours), the baked palettes.
//! On top: the app override, then the window override, then subtree overrides (resolved by
//! [`ThemeRegistry::resolve_subtree`] for the drawn implementations and by backends for native
//! controls through [`NativeTheme`]).

use std::collections::HashMap;

use nd_core::WindowId;
use nd_gpui::gpui::Hsla;

use crate::color::contrast;
use crate::fonts::SystemFonts;
use crate::matugen::MatugenTheme;
use crate::palette::{Palette, Scheme, SchemePalettes};
use crate::preset::{Metrics, Preset, SystemMetrics};
use crate::refinement::{Appearance, ThemeRefinement};
use crate::tokens::Theme;

/// What the OS says, read by the backend at launch and on every appearance or accent change.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OsTheme {
    /// The system's current scheme. `None` where the backend cannot tell.
    pub scheme: Option<Scheme>,
    /// Semantic colours resolved under each appearance.
    pub palettes: SchemePalettes,
    pub metrics: SystemMetrics,
    pub fonts: SystemFonts,
}

/// What native controls need from a resolved theme: only what differs from the OS, since a
/// native control draws itself with system colours unless a token is overridden (4).
#[derive(Clone, Debug, PartialEq)]
pub struct NativeTheme {
    /// Forced appearance; `None` follows the OS.
    pub appearance: Option<Scheme>,
    /// An accent that did not come from the OS: `bezelColor`/`contentTintColor` on AppKit.
    pub accent: Option<Hsla>,
    pub radius_scale: f32,
}

impl Default for NativeTheme {
    fn default() -> Self {
        Self { appearance: None, accent: None, radius_scale: 1. }
    }
}

impl NativeTheme {
    /// This theme with a subtree's refinement on top. `system` is the scheme the window
    /// resolves to when nothing forces one.
    pub fn refine(&self, refinement: &ThemeRefinement, system: Scheme) -> NativeTheme {
        let appearance = match refinement.appearance {
            Some(Appearance::Light) => Some(Scheme::Light),
            Some(Appearance::Dark) => Some(Scheme::Dark),
            Some(Appearance::System) | None => self.appearance,
        };
        let scheme = appearance.unwrap_or(system);
        NativeTheme {
            appearance,
            accent: refinement.palette(scheme).accent.or(self.accent),
            radius_scale: self.radius_scale * refinement.radius_scale.unwrap_or(1.),
        }
    }
}

/// The theme sources and override layers of one app.
#[derive(Clone, Debug)]
pub struct ThemeRegistry {
    preset: Preset,
    user: ThemeRefinement,
    matugen: Option<MatugenTheme>,
    os: OsTheme,
    app: ThemeRefinement,
    windows: HashMap<WindowId, ThemeRefinement>,
    app_theme: Theme,
    window_themes: HashMap<WindowId, Theme>,
    generation: u64,
}

impl Default for ThemeRegistry {
    fn default() -> Self {
        Self::new(Preset::current())
    }
}

impl ThemeRegistry {
    pub fn new(preset: Preset) -> Self {
        let mut this = Self {
            preset,
            user: ThemeRefinement::default(),
            matugen: None,
            os: OsTheme::default(),
            app: ThemeRefinement::default(),
            windows: HashMap::new(),
            app_theme: Theme::derive(preset, Scheme::Light, Palette::baked(Scheme::Light), Metrics::preset(preset), 1.),
            window_themes: HashMap::new(),
            generation: 0,
        };
        this.recompute();
        this
    }

    pub fn preset(&self) -> Preset {
        self.preset
    }

    /// Bumped on every change; hosts compare it to know when to redraw.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// The app-level theme: what views outside any window override read.
    pub fn app_theme(&self) -> &Theme {
        &self.app_theme
    }

    pub fn window_theme(&self, window: WindowId) -> &Theme {
        self.window_themes.get(&window).unwrap_or(&self.app_theme)
    }

    pub fn os(&self) -> &OsTheme {
        &self.os
    }

    pub fn matugen(&self) -> Option<&MatugenTheme> {
        self.matugen.as_ref()
    }

    pub fn set_os(&mut self, os: OsTheme) {
        if self.os != os {
            self.os = os;
            self.recompute();
        }
    }

    /// The parsed `theme.json`, or `None` when there is none.
    pub fn set_matugen(&mut self, theme: Option<MatugenTheme>) {
        if self.matugen != theme {
            self.matugen = theme;
            self.recompute();
        }
    }

    /// The parsed `theme.toml` (app or user), top of the palette chain.
    pub fn set_user(&mut self, theme: ThemeRefinement) {
        if self.user != theme {
            self.user = theme;
            self.recompute();
        }
    }

    pub fn set_app_override(&mut self, theme: ThemeRefinement) {
        if self.app != theme {
            self.app = theme;
            self.recompute();
        }
    }

    pub fn app_override(&self) -> &ThemeRefinement {
        &self.app
    }

    /// `None` clears the window's override.
    pub fn set_window_override(&mut self, window: WindowId, theme: Option<ThemeRefinement>) {
        let changed = match theme {
            Some(theme) => self.windows.insert(window, theme.clone()).as_ref() != Some(&theme),
            None => self.windows.remove(&window).is_some(),
        };
        if changed {
            self.recompute();
        }
    }

    /// Windows that have an override of their own.
    pub fn overridden_windows(&self) -> impl Iterator<Item = WindowId> + '_ {
        self.windows.keys().copied()
    }

    pub fn window_override(&self, window: WindowId) -> Option<&ThemeRefinement> {
        self.windows.get(&window)
    }

    pub fn forget_window(&mut self, window: WindowId) {
        self.windows.remove(&window);
        self.window_themes.remove(&window);
    }

    /// The scheme a layer with no appearance of its own gets.
    pub fn system_scheme(&self) -> Scheme {
        self.matugen.as_ref().and_then(|m| m.mode).or(self.os.scheme).unwrap_or_default()
    }

    /// The layers above the palette chain for `window`: user `theme.toml`, app, window.
    pub fn stack(&self, window: Option<WindowId>) -> ThemeRefinement {
        let app = self.app.over(&self.user);
        match window.and_then(|w| self.windows.get(&w)) {
            Some(w) => w.over(&app),
            None => app,
        }
    }

    /// The theme of a subtree with `refinement` set, inside `window`.
    pub fn resolve_subtree(&self, window: Option<WindowId>, refinement: &ThemeRefinement) -> Theme {
        self.resolve(&refinement.over(&self.stack(window)))
    }

    /// Resolves one complete override stack against the source chain.
    pub fn resolve(&self, stack: &ThemeRefinement) -> Theme {
        let scheme = stack.appearance.unwrap_or(Appearance::System).resolve(self.system_scheme());
        let mut palette = stack.palette(scheme);
        // An app's own accent brings no text colour for it; the platform's would be tuned to the
        // platform's accent. White or black, whichever reads better on it.
        if let (Some(accent), None) = (palette.accent, palette.on_accent) {
            let (white, black) = (nd_gpui::gpui::white(), nd_gpui::gpui::black());
            palette.on_accent = Some(if contrast(white, accent) >= contrast(black, accent) { white } else { black });
        }
        if let Some(matugen) = &self.matugen {
            palette = palette.over(*matugen.palettes.get(scheme));
        }
        let palette = palette.over(*self.os.palettes.get(scheme)).complete(&Palette::baked(scheme));
        let metrics = Metrics::preset(self.preset).with_system(&self.os.metrics).with_fonts(&self.os.fonts);
        Theme::derive(self.preset, scheme, palette, metrics, stack.radius_scale.unwrap_or(1.))
    }

    /// What native controls take from `stack`.
    pub fn native(&self, stack: &ThemeRefinement) -> NativeTheme {
        let scheme = stack.appearance.and_then(|a| match a {
            Appearance::System => None,
            Appearance::Light => Some(Scheme::Light),
            Appearance::Dark => Some(Scheme::Dark),
        });
        let resolved = scheme.unwrap_or_else(|| self.system_scheme());
        let from_matugen = self.matugen.as_ref().and_then(|m| m.palettes.get(resolved).accent);
        // A matugen file names a mode too; on a system with no matching OS appearance that
        // mode is forced so native controls agree with composed content.
        let matugen_mode = self.matugen.as_ref().and_then(|m| m.mode).filter(|m| Some(*m) != self.os.scheme);
        NativeTheme {
            appearance: scheme.or(matugen_mode),
            accent: stack.palette(resolved).accent.or(from_matugen),
            radius_scale: stack.radius_scale.unwrap_or(1.),
        }
    }

    pub fn window_native(&self, window: WindowId) -> NativeTheme {
        self.native(&self.stack(Some(window)))
    }

    pub fn app_native(&self) -> NativeTheme {
        self.native(&self.stack(None))
    }

    fn recompute(&mut self) {
        self.generation += 1;
        self.app_theme = self.resolve(&self.stack(None));
        let windows: Vec<WindowId> = self.windows.keys().copied().collect();
        self.window_themes = windows.into_iter().map(|w| (w, self.resolve(&self.stack(Some(w))))).collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::parse_hex;

    fn hex(s: &str) -> Hsla {
        parse_hex(s).unwrap()
    }

    #[test]
    fn chain_order_user_matugen_os_baked() {
        let mut r = ThemeRegistry::new(Preset::Adwaita);
        let baked = Palette::baked(Scheme::Light);
        assert_eq!(r.app_theme().accent, baked.accent);

        let mut os = OsTheme { scheme: Some(Scheme::Light), ..Default::default() };
        os.palettes.light.accent = Some(hex("#111111"));
        os.palettes.light.danger = Some(hex("#222222"));
        r.set_os(os);
        assert_eq!(r.app_theme().accent, hex("#111111"));

        let matugen = MatugenTheme::parse(r##"{"colors":{"primary":{"light":"#333333","dark":"#444444"}}}"##).unwrap();
        r.set_matugen(Some(matugen));
        assert_eq!(r.app_theme().accent, hex("#333333"));
        assert_eq!(r.app_theme().danger, hex("#222222"));

        r.set_user(ThemeRefinement::new().token("accent", hex("#555555")));
        assert_eq!(r.app_theme().accent, hex("#555555"));
        assert_eq!(r.app_theme().surface.base, baked.surface);
    }

    #[test]
    fn window_override_is_per_window() {
        let mut r = ThemeRegistry::new(Preset::MacOs);
        r.set_app_override(ThemeRefinement::new().accent(hex("#3d6bff")).radius_scale(1.25));
        let (a, b) = (WindowId(1), WindowId(2));
        r.set_window_override(a, Some(ThemeRefinement::new().appearance(Appearance::Dark)));
        assert_eq!(r.window_theme(a).scheme, Scheme::Dark);
        assert_eq!(r.window_theme(b).scheme, Scheme::Light);
        assert_eq!(r.window_theme(a).accent, hex("#3d6bff"));
        assert_eq!(r.window_theme(a).radius_scale, 1.25);
        let warm = ThemeRefinement::new().accent(hex("#ff8a3d")).radius_scale(1.5);
        let sub = r.resolve_subtree(Some(a), &warm);
        assert_eq!(sub.accent, hex("#ff8a3d"));
        assert_eq!(sub.radius_scale, 1.875);
        assert_eq!(sub.scheme, Scheme::Dark);
        let native = r.window_native(a);
        assert_eq!(native.appearance, Some(Scheme::Dark));
        assert_eq!(native.accent, Some(hex("#3d6bff")));
    }

    #[test]
    fn os_accent_is_not_a_native_override() {
        let mut r = ThemeRegistry::new(Preset::MacOs);
        let mut os = OsTheme { scheme: Some(Scheme::Dark), ..Default::default() };
        os.palettes.dark.accent = Some(hex("#0a84ff"));
        r.set_os(os);
        assert_eq!(r.app_theme().accent, hex("#0a84ff"));
        assert_eq!(r.app_native(), NativeTheme { appearance: None, accent: None, radius_scale: 1. });
    }

    #[test]
    fn generation_moves_only_on_change() {
        let mut r = ThemeRegistry::new(Preset::MacOs);
        let g = r.generation();
        r.set_matugen(None);
        r.set_app_override(ThemeRefinement::new());
        assert_eq!(r.generation(), g);
        r.set_app_override(ThemeRefinement::new().radius_scale(2.));
        assert_eq!(r.generation(), g + 1);
    }
}
