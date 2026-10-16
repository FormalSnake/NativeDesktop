//! Overrides at app, window, subtree and element level (ARCHITECTURE.md 3.6). Each layer is a
//! [`ThemeRefinement`]; a deeper layer's set fields win over a shallower one's.

use nd_gpui::gpui::{Hsla, SharedString};

use crate::palette::{PaletteRefinement, Scheme};

/// Light, dark or whatever the OS (or the layer above) says.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

impl Appearance {
    /// The scheme this preference picks, given the scheme of the layer above.
    pub fn resolve(self, inherited: Scheme) -> Scheme {
        match self {
            Appearance::System => inherited,
            Appearance::Light => Scheme::Light,
            Appearance::Dark => Scheme::Dark,
        }
    }
}

/// Changes to the theme below it. Every field is optional; unset fields inherit.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ThemeRefinement {
    pub appearance: Option<Appearance>,
    pub accent: Option<Hsla>,
    /// Multiplies the radii of the layer below (so app 1.25 and subtree 1.5 give 1.875).
    pub radius_scale: Option<f32>,
    /// Token colours for light schemes; [`ThemeRefinement::token`] sets both.
    pub light: PaletteRefinement,
    pub dark: PaletteRefinement,
    /// The family text is set in, in place of the one the platform resolved (an app setting
    /// such as a user's chosen font).
    pub font: Option<SharedString>,
    /// The family monospaced text is set in.
    pub mono_font: Option<SharedString>,
}

impl ThemeRefinement {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn accent(mut self, accent: impl Into<Hsla>) -> Self {
        self.accent = Some(accent.into());
        self
    }

    pub fn radius_scale(mut self, scale: f32) -> Self {
        self.radius_scale = Some(scale);
        self
    }

    pub fn appearance(mut self, appearance: Appearance) -> Self {
        self.appearance = Some(appearance);
        self
    }

    /// Sets text in `family` instead of the platform's font. An empty name keeps the
    /// platform's, so an unset app setting can be passed straight through.
    pub fn font(mut self, family: impl Into<SharedString>) -> Self {
        self.font = Some(family.into()).filter(|f| !f.trim().is_empty());
        self
    }

    /// Sets monospaced text in `family`.
    pub fn mono_font(mut self, family: impl Into<SharedString>) -> Self {
        self.mono_font = Some(family.into()).filter(|f| !f.trim().is_empty());
        self
    }

    /// `theme` with this layer's font families in place of its own.
    pub fn with_fonts(&self, mut theme: crate::Theme) -> crate::Theme {
        if let Some(family) = &self.font {
            theme.fonts.body.family = family.clone();
            theme.fonts.rounded.family = family.clone();
        }
        if let Some(family) = &self.mono_font {
            theme.fonts.mono.family = family.clone();
        }
        theme
    }

    /// Sets one base token (`"surface.base"`, `"danger"`, see [`crate::palette::KEYS`]) for
    /// both schemes. Panics on an unknown key: a typo would otherwise silently do nothing.
    pub fn token(mut self, key: &str, color: impl Into<Hsla>) -> Self {
        let color = color.into();
        assert!(self.light.set(key, color) && self.dark.set(key, color), "unknown theme token {key:?}");
        self
    }

    /// Sets one base token for one scheme only.
    pub fn token_for(mut self, scheme: Scheme, key: &str, color: impl Into<Hsla>) -> Self {
        let palette = match scheme {
            Scheme::Light => &mut self.light,
            Scheme::Dark => &mut self.dark,
        };
        assert!(palette.set(key, color.into()), "unknown theme token {key:?}");
        self
    }

    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// This layer on top of `below`: fields set here win, radius scales multiply.
    pub fn over(&self, below: &ThemeRefinement) -> ThemeRefinement {
        ThemeRefinement {
            appearance: match self.appearance {
                None | Some(Appearance::System) => below.appearance.or(self.appearance),
                set => set,
            },
            accent: self.accent.or(below.accent),
            radius_scale: match (self.radius_scale, below.radius_scale) {
                (Some(a), Some(b)) => Some(a * b),
                (a, b) => a.or(b),
            },
            light: self.light.over(below.light),
            dark: self.dark.over(below.dark),
            font: self.font.clone().or_else(|| below.font.clone()),
            mono_font: self.mono_font.clone().or_else(|| below.mono_font.clone()),
        }
    }

    /// The colour overrides for `scheme`, with `accent` folded in.
    pub fn palette(&self, scheme: Scheme) -> PaletteRefinement {
        let mut palette = match scheme {
            Scheme::Light => self.light,
            Scheme::Dark => self.dark,
        };
        if let Some(accent) = self.accent {
            palette.accent = Some(accent);
        }
        palette
    }
}

/// An app- or window-level theme override.
pub type ThemeOverride = ThemeRefinement;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::parse_hex;

    #[test]
    fn deeper_layers_win_and_scales_multiply() {
        let app = ThemeRefinement::new().accent(parse_hex("#3d6bff").unwrap()).radius_scale(1.25);
        let subtree = ThemeRefinement::new().accent(parse_hex("#ff8a3d").unwrap()).radius_scale(1.5);
        let merged = subtree.over(&app);
        assert_eq!(merged.accent, parse_hex("#ff8a3d"));
        assert_eq!(merged.radius_scale, Some(1.875));
    }

    #[test]
    fn system_appearance_inherits() {
        let window = ThemeRefinement::new().appearance(Appearance::Dark);
        let subtree = ThemeRefinement::new().appearance(Appearance::System);
        assert_eq!(subtree.over(&window).appearance, Some(Appearance::Dark));
        assert_eq!(Appearance::System.resolve(Scheme::Dark), Scheme::Dark);
    }

    #[test]
    fn a_font_override_replaces_the_platform_family_and_deeper_layers_win() {
        use crate::palette::Palette;
        use crate::preset::{Metrics, Preset};
        let app = ThemeRefinement::new().font("Inter").mono_font("Berkeley Mono");
        let window = ThemeRefinement::new().font("Iosevka Aile");
        let merged = window.over(&app);
        assert_eq!(merged.font.as_deref(), Some("Iosevka Aile"));
        assert_eq!(merged.mono_font.as_deref(), Some("Berkeley Mono"));
        assert_eq!(ThemeRefinement::new().font("  ").font, None, "an unset setting keeps the platform's");
        let base = crate::Theme::derive(
            Preset::Adwaita,
            Scheme::Light,
            Palette::baked(Scheme::Light),
            Metrics::preset(Preset::Adwaita),
            1.,
        );
        let themed = merged.with_fonts(base.clone());
        assert_eq!(themed.fonts.body.family.as_ref(), "Iosevka Aile");
        assert_eq!(themed.fonts.mono.family.as_ref(), "Berkeley Mono");
        assert_eq!(themed.fonts.body.weight, base.fonts.body.weight, "only the family changes");
        assert_eq!(ThemeRefinement::new().with_fonts(base.clone()), base);
    }

    #[test]
    #[should_panic(expected = "unknown theme token")]
    fn unknown_token_panics() {
        let _ = ThemeRefinement::new().token("surface.nope", parse_hex("#000000").unwrap());
    }
}
