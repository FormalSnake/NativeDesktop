//! Stand-in for `nd-theme` (M3): the token names apps read, with fixed light values, and the
//! override types (ARCHITECTURE.md 3.6, 4) with nothing behind them yet. On macOS the text and
//! accent tokens resolve from `NSColor` semantic colours.

use std::sync::LazyLock;

use nd_gpui::gpui::{Font, font};
use nd_ui::App;
use nd_ui::prelude::{Hsla, Pixels, hsla, px};

#[derive(Clone, Debug)]
pub struct TextTokens {
    pub primary: Hsla,
    pub secondary: Hsla,
    pub tertiary: Hsla,
    pub on_accent: Hsla,
}

#[derive(Clone, Debug)]
pub struct SurfaceTokens {
    /// The window background.
    pub base: Hsla,
    /// Cards, wells and anything one step above the base.
    pub raised: Hsla,
}

#[derive(Clone, Debug)]
pub struct BorderTokens {
    pub subtle: Hsla,
}

#[derive(Clone, Debug)]
pub struct SpacingTokens {
    /// The platform's margin between window edge and content.
    pub content_margin: Pixels,
    /// The widest a readable column grows.
    pub content_width: Pixels,
}

#[derive(Clone, Debug)]
pub struct TypeScale {
    pub caption: Pixels,
    pub body: Pixels,
    pub title: Pixels,
}

#[derive(Clone, Debug)]
pub struct FontTokens {
    pub body: Font,
    /// SF Pro Rounded on macOS; the body font elsewhere.
    pub rounded: Font,
}

#[derive(Clone, Debug)]
pub struct Theme {
    pub text: TextTokens,
    pub accent: Hsla,
    pub surface: SurfaceTokens,
    pub border: BorderTokens,
    pub spacing: SpacingTokens,
    pub type_scale: TypeScale,
    pub fonts: FontTokens,
}

impl Default for Theme {
    fn default() -> Self {
        Theme {
            text: TextTokens {
                primary: hsla(0., 0., 0., 0.85),
                secondary: hsla(0., 0., 0., 0.5),
                tertiary: hsla(0., 0., 0., 0.25),
                on_accent: hsla(0., 0., 1., 1.),
            },
            accent: hsla(211. / 360., 1., 0.5, 1.),
            surface: SurfaceTokens { base: hsla(0., 0., 0.98, 1.), raised: hsla(0., 0., 1., 1.) },
            border: BorderTokens { subtle: hsla(0., 0., 0., 0.1) },
            spacing: SpacingTokens { content_margin: px(20.), content_width: px(680.) },
            type_scale: TypeScale { caption: px(11.), body: px(13.), title: px(22.) },
            fonts: FontTokens { body: font(".SystemUIFont"), rounded: font(".SystemUIFont") },
        }
    }
}

static DEFAULT: LazyLock<Theme> = LazyLock::new(Theme::default);

impl Theme {
    /// The macOS preset: tokens read from `NSColor` semantic colours at launch.
    #[cfg(target_os = "macos")]
    pub(crate) fn from_system(c: nd_backend_appkit::SystemColors) -> Self {
        Theme {
            text: TextTokens {
                primary: c.label,
                secondary: c.secondary_label,
                tertiary: c.tertiary_label,
                on_accent: hsla(0., 0., 1., 1.),
            },
            accent: c.accent,
            ..Theme::default()
        }
    }
}

pub trait ActiveTheme {
    fn theme(&self) -> &Theme;
}

impl ActiveTheme for App {
    fn theme(&self) -> &Theme {
        self.try_global::<Theme>().unwrap_or(&DEFAULT)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

/// Changes to the platform preset, at app, window or subtree level. Recorded, not applied, until
/// `nd-theme` lands.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ThemeRefinement {
    pub accent: Option<Hsla>,
    pub radius_scale: Option<f32>,
    pub appearance: Option<Appearance>,
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
}

/// An app- or window-level theme override.
pub type ThemeOverride = ThemeRefinement;

/// How a structural pattern is presented: the OS's own, or drawn by nd (3.6).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Pattern {
    #[default]
    Native,
    Drawn,
}

/// App-wide pattern choices; `nativedesktop.toml` `[patterns]` sits below these.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Patterns {
    pub tooltips: Pattern,
    pub menus: Pattern,
    pub popovers: Pattern,
}
