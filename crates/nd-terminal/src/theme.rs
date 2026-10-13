//! The terminal's colours, taken from the nd theme: the default background and foreground are
//! `surface.base` and `text.primary`, the selection is `selection`, and the eight ANSI hues map
//! to the semantic tokens where one exists (red `danger`, green `success`, yellow `warning`, blue
//! `accent`). Magenta and cyan have no token; they turn the accent's hue and keep its lightness,
//! so they read at the accent's contrast. The 240 cube and grey entries stay xterm's.

use nd_theme::Theme;
use nd_theme::color::{mix, over, with_alpha};
use nd_ui::prelude::{Hsla, Rgba};

/// An opaque sRGB colour, the form libghostty works in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// `color` composited over `base`, rounded to bytes.
    pub fn from_hsla(color: Hsla, base: Hsla) -> Self {
        let c = over(color, base).to_rgb();
        let byte = |v: f32| (v.clamp(0., 1.) * 255.).round() as u8;
        Self { r: byte(c.r), g: byte(c.g), b: byte(c.b) }
    }

    pub fn to_hsla(self) -> Hsla {
        Rgba { r: f32::from(self.r) / 255., g: f32::from(self.g) / 255., b: f32::from(self.b) / 255., a: 1. }.into()
    }

    /// Channels as `0..=1` floats with alpha 1, for GPU vertex data.
    pub fn to_f32(self) -> [f32; 4] {
        [f32::from(self.r) / 255., f32::from(self.g) / 255., f32::from(self.b) / 255., 1.]
    }
}

impl From<libghostty_vt::style::RgbColor> for Rgb {
    fn from(c: libghostty_vt::style::RgbColor) -> Self {
        Self { r: c.r, g: c.g, b: c.b }
    }
}

impl From<Rgb> for libghostty_vt::style::RgbColor {
    fn from(c: Rgb) -> Self {
        Self { r: c.r, g: c.g, b: c.b }
    }
}

/// What the app may override on top of the theme-derived colours.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TerminalColors {
    pub background: Option<Hsla>,
    pub foreground: Option<Hsla>,
    pub cursor: Option<Hsla>,
    pub selection: Option<Hsla>,
    /// The 16 ANSI colours, black through bright white.
    pub ansi: Option<[Hsla; 16]>,
}

/// The colours one terminal draws with, resolved from a theme and the app's overrides.
#[derive(Clone, Debug, PartialEq)]
pub struct TerminalTheme {
    pub dark: bool,
    pub background: Hsla,
    pub foreground: Hsla,
    pub cursor: Hsla,
    pub selection: Hsla,
    /// Black through bright white, as tokens (translucent ones keep their alpha).
    pub ansi: [Hsla; 16],
    /// The opaque form of each colour, what libghostty resolves cells to.
    pub background_rgb: Rgb,
    pub foreground_rgb: Rgb,
    pub ansi_rgb: [Rgb; 16],
}

impl TerminalTheme {
    pub fn from_theme(theme: &Theme, overrides: &TerminalColors) -> Self {
        let dark = theme.is_dark();
        let background = overrides.background.unwrap_or(theme.surface.base);
        let foreground = overrides.foreground.unwrap_or(theme.text.primary);
        let base = with_alpha(background, 1.);
        let ansi = overrides.ansi.unwrap_or_else(|| derive_ansi(theme, base, foreground));
        let ansi_rgb = ansi.map(|c| Rgb::from_hsla(c, base));
        Self {
            dark,
            background,
            foreground,
            cursor: overrides.cursor.unwrap_or(foreground),
            selection: overrides.selection.unwrap_or(theme.selection),
            ansi,
            background_rgb: Rgb::from_hsla(background, base),
            foreground_rgb: Rgb::from_hsla(foreground, base),
            ansi_rgb,
        }
    }

    /// The colour to draw text resolved to `rgb` in: the ANSI token it came from when it is
    /// one, so a translucent token keeps its alpha, else `rgb` itself.
    pub fn ink(&self, rgb: Rgb) -> Hsla {
        if rgb == self.foreground_rgb {
            return self.foreground;
        }
        match self.ansi_rgb.iter().position(|c| *c == rgb) {
            Some(i) => self.ansi[i],
            None => rgb.to_hsla(),
        }
    }

    /// The colour to fill a cell background resolved to `rgb` with.
    pub fn fill(&self, rgb: Rgb) -> Hsla {
        if rgb == self.background_rgb {
            return self.background;
        }
        self.ink(rgb)
    }
}

fn derive_ansi(theme: &Theme, base: Hsla, foreground: Hsla) -> [Hsla; 16] {
    let fg = over(foreground, base);
    let accent = over(theme.accent, base);
    let turn = |degrees: f32| Hsla { h: (accent.h + degrees / 360.).rem_euclid(1.), ..accent };
    let red = theme.danger;
    let green = theme.success;
    let yellow = theme.warning;
    let blue = theme.accent;
    let magenta = turn(80.);
    let cyan = turn(-40.);
    // Black and white swap ends with the scheme, as every light terminal theme does: "black" is
    // the colour farthest from the background, "white" the nearest.
    let (black, white) =
        if theme.is_dark() { (mix(base, fg, 0.18), mix(base, fg, 0.78)) } else { (fg, mix(fg, base, 0.82)) };
    let (bright_black, bright_white) =
        if theme.is_dark() { (mix(base, fg, 0.45), fg) } else { (mix(fg, base, 0.4), mix(fg, base, 0.92)) };
    let brighten = |c: Hsla| {
        let c = over(c, base);
        if theme.is_dark() { mix(c, fg, 0.25) } else { mix(c, fg, 0.2) }
    };
    [
        black,
        red,
        green,
        yellow,
        blue,
        magenta,
        cyan,
        white,
        bright_black,
        brighten(red),
        brighten(green),
        brighten(yellow),
        brighten(blue),
        brighten(magenta),
        brighten(cyan),
        bright_white,
    ]
}
