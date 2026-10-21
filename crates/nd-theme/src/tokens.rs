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
    /// see-through sidebar and title row, as Windows' own apps do (measured: `#191919` dark on
    /// Task Manager, `#f9f9f9` light on Notepad).
    pub layer: Hsla,
    /// Toasts and tooltips: libadwaita's dark grey (`#505053`) under white text in both
    /// schemes; the overlay surface on the other looks.
    pub osd: Hsla,
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
    /// Text on `surface.osd`.
    pub on_osd: Hsla,
    /// Error text: `danger`, darkened or lightened until it reads at 4.5:1 on every surface.
    pub danger: Hsla,
    /// Links and mentions: the accent, made legible on every surface the same way.
    pub accent: Hsla,
    pub success: Hsla,
    pub warning: Hsla,
}

/// See [`Theme::role_fills`].
#[derive(Clone, Debug, PartialEq)]
pub struct RoleFills {
    pub success: Hsla,
    pub warning: Hsla,
    pub danger: Hsla,
}

/// What a filled surface means, which picks its fill and the ink on it from the theme. Apps
/// name the role; the colour is always a token.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum FillRole {
    /// The raised surface with primary text: a received message.
    Neutral,
    /// The accent: a sent message.
    #[default]
    Accent,
    /// `success`: an SMS message's green.
    Success,
    Warning,
    Danger,
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
    /// `success`, `warning` and `danger` as fills carrying `text.on_accent`, as
    /// [`Theme::accent_fill`] is for the accent: an SMS bubble's green, a destructive badge.
    pub role_fills: RoleFills,
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
        // Coloured text: libadwaita's standalone colours on its own look (lightened or darkened
        // only as far as 4.5:1 on a 15% tint of themselves, the destructive button), elsewhere
        // the colour made legible on every surface.
        let role_text = |c: Hsla| {
            if preset == Preset::Adwaita {
                let ink = crate::color::standalone(c, scheme.is_dark());
                let tint = with_alpha(ink, 0.15);
                let tinted = [p.surface, p.surface_sidebar, p.surface_raised, p.surface_overlay].map(|s| over(tint, s));
                legible(ink, &tinted, 4.5)
            } else {
                legible(c, &surfaces, 4.5)
            }
        };
        // libadwaita draws borders and separators as 15% of the text colour over what is under
        // them (`--border-color`). A source that gives an opaque line (matugen's outline_variant)
        // would read as a hard grey rule on the Linux look, so it takes libadwaita's instead.
        let separator =
            if preset == Preset::Adwaita && p.separator.a >= 0.99 { with_alpha(ink, 0.15) } else { p.separator };
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
                osd: if preset == Preset::Adwaita { nd_gpui::gpui::rgb(0x505053).into() } else { p.surface_overlay },
            },
            border: BorderTokens { subtle: separator, strong },
            separator,
            text: TextTokens {
                primary: text_primary,
                secondary: text_secondary,
                tertiary: p.text_tertiary,
                on_accent: p.on_accent,
                on_osd: if preset == Preset::Adwaita { nd_gpui::gpui::white() } else { text_primary },
                danger: role_text(p.danger),
                accent: role_text(p.accent),
                success: role_text(p.success),
                warning: role_text(p.warning),
            },
            accent: p.accent,
            accent_fill: fill_under(p.accent, p.on_accent, 4.5),
            accent_container: AccentContainer { bg: p.accent_container, fg: p.on_accent_container },
            role_fills: RoleFills {
                success: fill_under(p.success, p.on_accent, 4.5),
                warning: fill_under(p.warning, p.on_accent, 4.5),
                danger: fill_under(p.danger, p.on_accent, 4.5),
            },
            selection: p.selection,
            danger: p.danger,
            success: p.success,
            warning: p.warning,
            focus_ring: with_alpha(p.accent, 0.5),
            // libadwaita doubles a dialog's dimming on a dark window.
            scrim: with_alpha(
                nd_gpui::gpui::black(),
                match (preset, scheme.is_dark()) {
                    (Preset::Adwaita, true) => 0.5,
                    (_, true) => 0.4,
                    (_, false) => 0.15,
                },
            ),
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

    /// The fill for `role` and the ink that reads on it at 4.5:1.
    pub fn fill(&self, role: FillRole) -> (Hsla, Hsla) {
        match role {
            FillRole::Neutral => (self.surface.raised_hover, self.text.primary),
            FillRole::Accent => (self.accent_fill, self.text.on_accent),
            FillRole::Success => (self.role_fills.success, self.text.on_accent),
            FillRole::Warning => (self.role_fills.warning, self.text.on_accent),
            FillRole::Danger => (self.role_fills.danger, self.text.on_accent),
        }
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
    fn an_opaque_separator_is_softened_to_libadwaitas_border_on_the_linux_look() {
        use crate::palette::{Palette, Scheme};
        use crate::preset::{Metrics, Preset};
        use crate::tokens::Theme;
        let mut palette = Palette::baked(Scheme::Dark);
        palette.separator = parse_hex("#44474f").unwrap();
        let adwaita = Theme::derive(Preset::Adwaita, Scheme::Dark, palette, Metrics::preset(Preset::Adwaita), 1.);
        assert!((adwaita.separator.a - 0.15).abs() < 0.001, "15% of the text colour, as libadwaita's border");
        assert_eq!(adwaita.border.subtle, adwaita.separator);
        let mac = Theme::derive(Preset::MacOs, Scheme::Dark, palette, Metrics::preset(Preset::MacOs), 1.);
        assert_eq!(mac.separator, palette.separator, "other looks keep the source's line");
    }

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

    #[test]
    fn every_role_fill_carries_its_ink_and_role_text_reads_on_every_surface() {
        use crate::palette::{Palette, Scheme};
        use crate::preset::{Metrics, Preset};
        use crate::tokens::{FillRole, Theme};
        for scheme in [Scheme::Light, Scheme::Dark] {
            for preset in [Preset::MacOs, Preset::Adwaita] {
                let theme = Theme::derive(preset, scheme, Palette::baked(scheme), Metrics::preset(preset), 1.);
                for role in
                    [FillRole::Neutral, FillRole::Accent, FillRole::Success, FillRole::Warning, FillRole::Danger]
                {
                    let (fill, ink) = theme.fill(role);
                    let fill = crate::color::over(fill, theme.surface.base);
                    assert!(contrast(ink, fill) >= 4.5, "{scheme:?} {preset:?} {role:?}");
                }
                assert_ne!(
                    theme.fill(FillRole::Success).0,
                    theme.fill(FillRole::Accent).0,
                    "an SMS bubble is not blue"
                );
                for text in [theme.text.accent, theme.text.success, theme.text.warning] {
                    for surface in [theme.surface.base, theme.surface.raised, theme.surface.sidebar] {
                        assert!(contrast(text, surface) >= 4.5, "{scheme:?} {preset:?}");
                    }
                }
            }
        }
    }
}
