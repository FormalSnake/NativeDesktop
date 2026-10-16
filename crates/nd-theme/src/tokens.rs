//! The token set apps read with `cx.theme()` (ARCHITECTURE.md 4). Built from one scheme's
//! [`Palette`] and the platform [`Metrics`]; derived tokens (hover, pressed, disabled, focus
//! ring, scrim, strong border) are computed here from the base keys, never read from a source.

use nd_gpui::gpui::{Font, Hsla, Pixels};

use crate::color::{contrast, is_dark, legible, mix, over, with_alpha};
use crate::palette::{Palette, Scheme};
use crate::preset::{ControlHeights, Materials, Metrics, Motion, Preset, Radii, Spacing, TextStyles, TypeScale};

#[derive(Clone, Debug, PartialEq)]
pub struct SurfaceTokens {
    /// The window background.
    pub base: Hsla,
    pub sidebar: Hsla,
    /// Cards, wells and anything one step above the base.
    pub raised: Hsla,
    pub raised_hover: Hsla,
    /// Popovers, menus and sheets drawn by nd.
    pub overlay: Hsla,
    /// The opaque content layer a window with a backdrop material (Mica) lays beside a
    /// see-through sidebar and title row, as Windows' own apps do (measured on Task Manager).
    pub layer: Hsla,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BorderTokens {
    pub subtle: Hsla,
    pub strong: Hsla,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextTokens {
    pub primary: Hsla,
    pub secondary: Hsla,
    pub tertiary: Hsla,
    pub on_accent: Hsla,
    /// Error text: `danger`, darkened or lightened until it reads at 4.5:1 on every surface.
    pub danger: Hsla,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AccentContainer {
    pub bg: Hsla,
    pub fg: Hsla,
}

/// Fonts by role. `body` is what text uses when it names none.
pub type FontTokens = crate::preset::Fonts;

/// The 16 ANSI colours a terminal draws program output with, black through bright white. The
/// hues are the semantic tokens (red `danger`, green `success`, yellow `warning`, blue `accent`;
/// magenta and cyan turn the accent's hue), each made legible as text on the base surface.
#[derive(Clone, Debug, PartialEq)]
pub struct TerminalTokens {
    pub ansi: [Hsla; 16],
}

#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    pub preset: Preset,
    pub scheme: Scheme,
    pub surface: SurfaceTokens,
    pub border: BorderTokens,
    pub separator: Hsla,
    pub text: TextTokens,
    pub accent: Hsla,
    /// The accent as a fill that carries `text.on_accent` (a sent chat bubble): moved just
    /// far enough from the accent for that text to read at 4.5:1. macOS's systemBlue under
    /// white text is 4.0:1.
    pub accent_fill: Hsla,
    pub accent_container: AccentContainer,
    pub selection: Hsla,
    pub danger: Hsla,
    pub success: Hsla,
    pub warning: Hsla,
    pub focus_ring: Hsla,
    pub scrim: Hsla,
    pub spacing: Spacing,
    pub radius: Radii,
    /// Multiplies every radius nd draws, including the radii apps set with `.rounded_*()`.
    pub radius_scale: f32,
    pub control: ControlHeights,
    pub type_scale: TypeScale,
    pub text_styles: TextStyles,
    pub fonts: FontTokens,
    pub motion: Motion,
    pub materials: Materials,
    pub terminal: TerminalTokens,
    /// The base colours this theme was derived from.
    pub palette: Palette,
}

impl Theme {
    /// Derives every token from `palette` for `scheme`, with `metrics` from the preset.
    pub fn derive(preset: Preset, scheme: Scheme, palette: Palette, metrics: Metrics, radius_scale: f32) -> Theme {
        let p = palette;
        // The ink derived states lean towards: the text colour, which is dark on light schemes.
        let ink = with_alpha(p.text_primary, 1.);
        let raised_hover = mix(p.surface_raised, ink, if scheme.is_dark() { 0.08 } else { 0.05 });
        let strong = with_alpha(p.text_tertiary, (p.text_tertiary.a * 1.2).min(1.));
        // Body text keeps WCAG AA (4.5:1) on every surface it sits on, whatever the source says:
        // a source's secondary label can fall short (Adwaita's 50% black, macOS's
        // secondaryLabelColor on white).
        let surfaces = [p.surface, p.surface_sidebar, p.surface_raised];
        let text_primary = legible(p.text_primary, &surfaces, 4.5);
        let text_secondary = legible(p.text_secondary, &surfaces, 4.5);
        Theme {
            preset,
            scheme,
            surface: SurfaceTokens {
                base: p.surface,
                sidebar: p.surface_sidebar,
                raised: p.surface_raised,
                raised_hover,
                overlay: p.surface_overlay,
                layer: nd_gpui::gpui::rgb(if scheme.is_dark() { 0x191919 } else { 0xf9f9f9 }).into(),
            },
            border: BorderTokens { subtle: p.separator, strong },
            separator: p.separator,
            text: TextTokens {
                primary: text_primary,
                secondary: text_secondary,
                tertiary: p.text_tertiary,
                on_accent: p.on_accent,
                danger: legible(p.danger, &surfaces, 4.5),
            },
            accent: p.accent,
            accent_fill: fill_under(p.accent, p.on_accent, 4.5),
            accent_container: AccentContainer { bg: p.accent_container, fg: p.on_accent_container },
            selection: p.selection,
            danger: p.danger,
            success: p.success,
            warning: p.warning,
            focus_ring: with_alpha(p.accent, 0.5),
            scrim: with_alpha(nd_gpui::gpui::black(), if scheme.is_dark() { 0.4 } else { 0.15 }),
            spacing: metrics.spacing,
            radius: metrics.radius.scaled(radius_scale),
            radius_scale,
            control: metrics.control,
            type_scale: metrics.type_scale,
            text_styles: metrics.text_styles,
            fonts: metrics.fonts,
            motion: metrics.motion,
            materials: metrics.materials,
            terminal: terminal_tokens(scheme, &p, text_primary),
            palette,
        }
    }

    pub fn is_dark(&self) -> bool {
        self.scheme.is_dark()
    }

    /// `color` under the pointer.
    pub fn hover(&self, color: Hsla) -> Hsla {
        self.shift(color, 0.08)
    }

    /// `color` while pressed.
    pub fn pressed(&self, color: Hsla) -> Hsla {
        self.shift(color, 0.16)
    }

    /// `color` on a disabled control.
    pub fn disabled(&self, color: Hsla) -> Hsla {
        with_alpha(color, color.a * 0.4)
    }

    /// Text that reads on `background`: the scheme's on-accent colour on dark fills, primary
    /// text on light ones.
    pub fn text_on(&self, background: Hsla) -> Hsla {
        let opaque = over(background, self.surface.base);
        if is_dark(opaque) == self.is_dark() {
            self.text.primary
        } else if is_dark(opaque) {
            self.text.on_accent
        } else {
            with_alpha(nd_gpui::gpui::black(), 0.85)
        }
    }

    /// The radius an element asked for, scaled by the theme's radius scale.
    pub fn scale_radius(&self, radius: Pixels) -> Pixels {
        radius * self.radius_scale
    }

    pub fn body_font(&self) -> &Font {
        &self.fonts.body
    }

    /// Light states darken towards the ink on light schemes and lighten on dark ones.
    fn shift(&self, color: Hsla, amount: f32) -> Hsla {
        let towards =
            if is_dark(over(color, self.surface.base)) { nd_gpui::gpui::white() } else { nd_gpui::gpui::black() };
        mix(color, with_alpha(towards, color.a), amount)
    }
}

/// See [`TerminalTokens`]. Black and white swap ends with the scheme, as every light terminal
/// theme does: black is the colour farthest from the background, white the nearest.
fn terminal_tokens(scheme: Scheme, p: &Palette, text: Hsla) -> TerminalTokens {
    let base = with_alpha(p.surface, 1.);
    let fg = over(text, base);
    let dark = scheme.is_dark();
    let text_on = |c: Hsla| legible(over(c, base), &[base], 4.5);
    let accent = over(p.accent, base);
    let turn = |degrees: f32| Hsla { h: (accent.h + degrees / 360.).rem_euclid(1.), ..accent };
    let hues = [p.danger, p.success, p.warning, p.accent, turn(80.), turn(-40.)].map(text_on);
    let bright = hues.map(|c| text_on(mix(c, fg, if dark { 0.25 } else { 0.2 })));
    let (black, white) = if dark { (mix(base, fg, 0.18), mix(base, fg, 0.78)) } else { (fg, mix(fg, base, 0.82)) };
    let (bright_black, bright_white) =
        if dark { (text_on(mix(base, fg, 0.55)), fg) } else { (text_on(mix(fg, base, 0.4)), mix(fg, base, 0.92)) };
    TerminalTokens {
        ansi: [
            black,
            hues[0],
            hues[1],
            hues[2],
            hues[3],
            hues[4],
            hues[5],
            white,
            bright_black,
            bright[0],
            bright[1],
            bright[2],
            bright[3],
            bright[4],
            bright[5],
            bright_white,
        ],
    }
}

/// `fill` moved towards black (or white, under dark text) until `text` reads on it at `ratio`.
fn fill_under(fill: Hsla, text: Hsla, ratio: f32) -> Hsla {
    let text = over(text, fill);
    if contrast(text, fill) >= ratio {
        return fill;
    }
    let ink = if is_dark(text) { nd_gpui::gpui::white() } else { nd_gpui::gpui::black() };
    for step in 1..=60 {
        let candidate = mix(fill, with_alpha(ink, fill.a), step as f32 / 100.);
        if contrast(over(text, candidate), candidate) >= ratio {
            return candidate;
        }
    }
    mix(fill, with_alpha(ink, fill.a), 0.6)
}

#[cfg(test)]
mod tests {
    use super::fill_under;
    use crate::color::{contrast, parse_hex};

    #[test]
    fn a_fill_under_white_text_darkens_until_the_text_reads() {
        let blue = parse_hex("#007aff").unwrap();
        let white = parse_hex("#ffffff").unwrap();
        assert!(contrast(white, blue) < 4.5);
        let fill = fill_under(blue, white, 4.5);
        assert!(contrast(white, fill) >= 4.5);
        let dark = parse_hex("#1c4fa0").unwrap();
        assert_eq!(fill_under(dark, white, 4.5), dark, "a fill that already passes is left alone");
    }
}
