//! Presets: the metrics, type, motion and materials of one platform look. Colours are not part
//! of a preset; they come from the palette chain. Where the OS publishes a value (text style
//! sizes, control heights) the backend reads it into [`SystemMetrics`] and it replaces the
//! preset's own, so composed content follows the OS (ARCHITECTURE.md 3.10).

use std::time::Duration;

use std::ops::{Index, IndexMut};

use nd_gpui::gpui::{Font, FontWeight, Pixels, font, px};

use crate::fonts::SystemFonts;

/// A platform look. Each backend picks one at launch; apps override tokens, not the preset.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Preset {
    /// macOS: metrics read from AppKit, the rest from the HIG.
    #[default]
    MacOs,
    /// Windows: Fluent 2 (the GPUI skin that reproduces WinUI 3).
    Fluent,
    /// Linux: the GPUI look with libadwaita metrics and a matugen palette.
    Adwaita,
    /// The web target: plain GPUI.
    Plain,
}

impl Preset {
    /// The look drawn backends use: the one `ND_PRESET` names, else [`Preset::native`].
    pub fn current() -> Preset {
        Preset::forced().unwrap_or_else(Preset::native)
    }

    /// The preset `ND_PRESET` names (`macos`, `fluent`, `adwaita`, `plain`), read once. It lets a
    /// GPUI build draw another platform's look, the Fluent skin on Linux, without its OS
    /// materials or captions. The AppKit backend is native and ignores it.
    pub fn forced() -> Option<Preset> {
        static FORCED: std::sync::OnceLock<Option<Preset>> = std::sync::OnceLock::new();
        *FORCED.get_or_init(|| {
            let name = std::env::var("ND_PRESET").ok().filter(|n| !n.is_empty())?;
            let preset = Preset::parse(&name);
            if preset.is_none() {
                eprintln!("nd: ND_PRESET={name} is not one of macos, fluent, adwaita, plain");
            }
            preset
        })
    }

    pub fn parse(name: &str) -> Option<Preset> {
        match name.to_ascii_lowercase().as_str() {
            "macos" | "mac" => Some(Preset::MacOs),
            "fluent" | "windows" => Some(Preset::Fluent),
            "adwaita" | "linux" => Some(Preset::Adwaita),
            "plain" | "web" => Some(Preset::Plain),
            _ => None,
        }
    }

    /// The preset of the platform this binary was built for.
    pub fn native() -> Preset {
        if cfg!(target_os = "macos") {
            Preset::MacOs
        } else if cfg!(target_os = "windows") {
            Preset::Fluent
        } else if cfg!(target_family = "wasm") {
            Preset::Plain
        } else {
            Preset::Adwaita
        }
    }
}

/// A system material, mapped by each backend to the OS's own (never a replica, 3.10).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Material {
    /// Liquid Glass (`NSGlassEffectView`) on macOS 26 and later, the closest system material
    /// elsewhere.
    Glass,
    /// macOS vibrancy (`NSVisualEffectView`).
    Vibrancy,
    Mica,
    MicaAlt,
    Acrylic,
    /// A transparent surface the compositor blurs by its own rules (Hyprland).
    Transparent,
    KdeBlur,
    #[default]
    Opaque,
}

/// How a `glass()` element is drawn where the OS has more than one glass.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum GlassVariant {
    #[default]
    Regular,
    Clear,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spacing {
    pub xs: Pixels,
    pub sm: Pixels,
    pub md: Pixels,
    pub lg: Pixels,
    pub xl: Pixels,
    /// The platform's margin between window edge and content.
    pub content_margin: Pixels,
    /// The widest a readable column grows.
    pub content_width: Pixels,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Radii {
    pub sm: Pixels,
    pub md: Pixels,
    pub lg: Pixels,
    pub xl: Pixels,
}

impl Radii {
    pub fn scaled(self, scale: f32) -> Radii {
        Radii { sm: self.sm * scale, md: self.md * scale, lg: self.lg * scale, xl: self.xl * scale }
    }
}

/// Heights of push buttons, fields and pickers per control size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ControlHeights {
    pub small: Pixels,
    pub regular: Pixels,
    pub large: Pixels,
    /// Shortest row of a grouped list or settings form, a regular control plus its padding.
    pub row: Pixels,
}

/// Point sizes of the text styles.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TypeScale {
    pub caption: Pixels,
    pub footnote: Pixels,
    pub body: Pixels,
    pub headline: Pixels,
    pub title: Pixels,
    pub large_title: Pixels,
}

/// The platform's text styles. On macOS each is `NSFont.preferredFont(forTextStyle:)`, so
/// labels follow the system's sizes; GPUI skins draw them at the theme's [`TextStyles`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TextStyle {
    LargeTitle,
    Title1,
    Title2,
    Title3,
    Headline,
    Subheadline,
    #[default]
    Body,
    Callout,
    Footnote,
    Caption1,
    Caption2,
}

impl TextStyle {
    pub const ALL: [TextStyle; 11] = [
        TextStyle::LargeTitle,
        TextStyle::Title1,
        TextStyle::Title2,
        TextStyle::Title3,
        TextStyle::Headline,
        TextStyle::Subheadline,
        TextStyle::Body,
        TextStyle::Callout,
        TextStyle::Footnote,
        TextStyle::Caption1,
        TextStyle::Caption2,
    ];
}

/// One text style: what text set in it is drawn at.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TypeStyle {
    pub size: Pixels,
    pub weight: FontWeight,
    pub line_height: Pixels,
}

/// Size, weight and line height of every [`TextStyle`], indexed by the style.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextStyles([TypeStyle; 11]);

impl TextStyles {
    /// From `(size, weight, line height)` rows in [`TextStyle::ALL`] order.
    pub fn from_rows(rows: [(f32, f32, f32); 11]) -> TextStyles {
        TextStyles(rows.map(|(size, weight, line)| TypeStyle {
            size: px(size),
            weight: FontWeight(weight),
            line_height: px(line),
        }))
    }
}

impl Index<TextStyle> for TextStyles {
    type Output = TypeStyle;

    fn index(&self, style: TextStyle) -> &TypeStyle {
        &self.0[style as usize]
    }
}

impl IndexMut<TextStyle> for TextStyles {
    fn index_mut(&mut self, style: TextStyle) -> &mut TypeStyle {
        &mut self.0[style as usize]
    }
}

/// Fonts by role, named by the family the OS reports ([`crate::fonts::resolve`]); a preset's
/// own are what stands when the OS names none.
#[derive(Clone, Debug, PartialEq)]
pub struct Fonts {
    pub body: Font,
    /// The system font's rounded design on macOS; the body font elsewhere.
    pub rounded: Font,
    pub mono: Font,
}

/// A critically damped-ish spring: `response` is the period in seconds, `damping` the ratio.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spring {
    pub response: f32,
    pub damping: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Motion {
    pub fast: Duration,
    pub normal: Duration,
    pub slow: Duration,
    pub spring: Spring,
}

/// Which material each structural surface uses.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Materials {
    pub sidebar: Material,
    pub toolbar: Material,
    pub overlay: Material,
    /// What `glass()` maps to.
    pub glass: Material,
}

/// Values the OS publishes, read by the backend. Unset fields keep the preset's.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SystemMetrics {
    pub caption: Option<f32>,
    pub footnote: Option<f32>,
    pub body: Option<f32>,
    pub headline: Option<f32>,
    pub title: Option<f32>,
    pub large_title: Option<f32>,
    pub control_small: Option<f32>,
    pub control_regular: Option<f32>,
    pub control_large: Option<f32>,
    /// The user's "reduce motion" setting.
    pub reduce_motion: Option<bool>,
}

/// Everything in a theme that is not a colour.
#[derive(Clone, Debug, PartialEq)]
pub struct Metrics {
    pub spacing: Spacing,
    pub radius: Radii,
    pub control: ControlHeights,
    pub type_scale: TypeScale,
    pub text_styles: TextStyles,
    pub fonts: Fonts,
    pub motion: Motion,
    pub materials: Materials,
}

impl Metrics {
    pub fn preset(preset: Preset) -> Metrics {
        let spacing = |xs, sm, md, lg, xl, margin, width| Spacing {
            xs: px(xs),
            sm: px(sm),
            md: px(md),
            lg: px(lg),
            xl: px(xl),
            content_margin: px(margin),
            content_width: px(width),
        };
        let radii = |sm, md, lg, xl| Radii { sm: px(sm), md: px(md), lg: px(lg), xl: px(xl) };
        let controls = |small, regular, large, row| ControlHeights {
            small: px(small),
            regular: px(regular),
            large: px(large),
            row: px(row),
        };
        let scale = |caption, footnote, body, headline, title, large_title| TypeScale {
            caption: px(caption),
            footnote: px(footnote),
            body: px(body),
            headline: px(headline),
            title: px(title),
            large_title: px(large_title),
        };
        let ms = Duration::from_millis;
        match preset {
            Preset::MacOs => Metrics {
                spacing: spacing(4., 8., 12., 20., 32., 20., 680.),
                radius: radii(4., 6., 10., 16.),
                control: controls(22., 28., 34., 40.),
                type_scale: scale(10., 10., 13., 13., 22., 26.),
                // NSFont's text styles: line heights are NSLayoutManager's default for each.
                text_styles: TextStyles::from_rows([
                    (26., 400., 32.),
                    (22., 400., 26.),
                    (17., 400., 22.),
                    (15., 400., 20.),
                    (13., 700., 16.),
                    (11., 400., 14.),
                    (13., 400., 16.),
                    (12., 400., 15.),
                    (10., 400., 13.),
                    (10., 400., 13.),
                    (10., 510., 13.),
                ]),
                fonts: Fonts {
                    body: font(".AppleSystemUIFont"),
                    rounded: font(".AppleSystemUIFontRounded"),
                    mono: font(".AppleSystemUIFontMonospaced"),
                },
                motion: Motion {
                    fast: ms(150),
                    normal: ms(250),
                    slow: ms(400),
                    spring: Spring { response: 0.35, damping: 0.86 },
                },
                materials: Materials {
                    sidebar: Material::Vibrancy,
                    toolbar: Material::Glass,
                    overlay: Material::Glass,
                    glass: Material::Glass,
                },
            },
            Preset::Fluent => Metrics {
                spacing: spacing(4., 8., 12., 16., 24., 24., 720.),
                radius: radii(2., 4., 8., 12.),
                control: controls(24., 32., 40., 48.),
                type_scale: scale(12., 12., 14., 14., 20., 28.),
                // WinUI's type ramp: Title Large, Title, Subtitle, Body Large, Body Strong for
                // the headline, Caption for the small styles.
                text_styles: TextStyles::from_rows([
                    (40., 600., 52.),
                    (28., 600., 36.),
                    (20., 600., 28.),
                    (18., 400., 24.),
                    (14., 600., 20.),
                    (12., 400., 16.),
                    (14., 400., 20.),
                    (14., 400., 20.),
                    (12., 400., 16.),
                    (12., 400., 16.),
                    (12., 400., 16.),
                ]),
                fonts: Fonts {
                    body: font("Segoe UI Variable Text"),
                    rounded: font("Segoe UI Variable Text"),
                    mono: font("Cascadia Mono"),
                },
                // WinUI's ControlFasterAnimationDuration, ControlFastAnimationDuration and
                // ControlNormalAnimationDuration (learn.microsoft.com, Timing and easing).
                motion: Motion {
                    fast: ms(83),
                    normal: ms(167),
                    slow: ms(250),
                    spring: Spring { response: 0.3, damping: 0.9 },
                },
                materials: Materials {
                    sidebar: Material::Mica,
                    toolbar: Material::Mica,
                    overlay: Material::Acrylic,
                    glass: Material::Acrylic,
                },
            },
            Preset::Adwaita => Metrics {
                spacing: spacing(3., 6., 12., 18., 24., 12., 600.),
                radius: radii(4., 6., 12., 16.),
                control: controls(24., 34., 42., 50.),
                type_scale: scale(11., 12., 14., 14., 20., 26.),
                text_styles: adwaita_text_styles(),
                fonts: Fonts { body: font("Adwaita Sans"), rounded: font("Adwaita Sans"), mono: font("Adwaita Mono") },
                motion: Motion {
                    fast: ms(150),
                    normal: ms(250),
                    slow: ms(400),
                    spring: Spring { response: 0.35, damping: 0.9 },
                },
                materials: Materials {
                    sidebar: Material::Opaque,
                    toolbar: Material::Opaque,
                    overlay: Material::Opaque,
                    glass: Material::Transparent,
                },
            },
            Preset::Plain => Metrics {
                spacing: spacing(4., 8., 12., 16., 24., 16., 680.),
                radius: radii(4., 6., 8., 12.),
                control: controls(24., 32., 40., 48.),
                type_scale: scale(12., 12., 14., 14., 20., 28.),
                text_styles: adwaita_text_styles(),
                fonts: Fonts { body: font("Noto Sans"), rounded: font("Noto Sans"), mono: font("Noto Sans Mono") },
                motion: Motion {
                    fast: ms(150),
                    normal: ms(250),
                    slow: ms(400),
                    spring: Spring { response: 0.35, damping: 0.9 },
                },
                materials: Materials {
                    sidebar: Material::Opaque,
                    toolbar: Material::Opaque,
                    overlay: Material::Opaque,
                    glass: Material::Opaque,
                },
            },
        }
    }

    /// The preset with every value the OS published replacing its own.
    pub fn with_system(mut self, system: &SystemMetrics) -> Metrics {
        let set = |slot: &mut Pixels, value: Option<f32>| {
            if let Some(v) = value.filter(|v| *v > 0.) {
                *slot = px(v);
            }
        };
        let t = &mut self.type_scale;
        set(&mut t.caption, system.caption);
        set(&mut t.footnote, system.footnote);
        set(&mut t.body, system.body);
        set(&mut t.headline, system.headline);
        set(&mut t.title, system.title);
        set(&mut t.large_title, system.large_title);
        let c = &mut self.control;
        set(&mut c.small, system.control_small);
        set(&mut c.regular, system.control_regular);
        set(&mut c.large, system.control_large);
        if system.reduce_motion == Some(true) {
            self.motion.fast = Duration::ZERO;
            self.motion.normal = Duration::ZERO;
            self.motion.slow = Duration::ZERO;
        }
        self
    }

    /// The preset with the families and text styles the OS named replacing its own.
    pub fn with_fonts(mut self, system: &SystemFonts) -> Metrics {
        let set = |slot: &mut Font, family: Option<&nd_gpui::gpui::SharedString>| {
            if let Some(family) = family.filter(|f| !f.is_empty()) {
                slot.family = family.clone();
            }
        };
        set(&mut self.fonts.body, system.body.as_ref());
        set(&mut self.fonts.rounded, system.rounded.as_ref().or(system.body.as_ref()));
        set(&mut self.fonts.mono, system.mono.as_ref());
        if let Some(styles) = system.text_styles {
            self.text_styles = styles;
            let t = &mut self.type_scale;
            t.caption = styles[TextStyle::Caption1].size;
            t.footnote = styles[TextStyle::Footnote].size;
            t.body = styles[TextStyle::Body].size;
            t.headline = styles[TextStyle::Headline].size;
            t.title = styles[TextStyle::Title1].size;
            t.large_title = styles[TextStyle::LargeTitle].size;
        }
        self
    }
}

fn adwaita_text_styles() -> TextStyles {
    TextStyles::from_rows([
        (26., 800., 32.),
        (20., 800., 26.),
        (20., 700., 26.),
        (17., 700., 22.),
        (14., 700., 20.),
        (13., 400., 18.),
        (14., 400., 20.),
        (13., 400., 18.),
        (12., 400., 16.),
        (12., 400., 16.),
        (11., 400., 14.),
    ])
}

#[cfg(test)]
mod tests {
    use super::Preset;

    #[test]
    fn nd_preset_names_parse() {
        assert_eq!(Preset::parse("fluent"), Some(Preset::Fluent));
        assert_eq!(Preset::parse("Windows"), Some(Preset::Fluent));
        assert_eq!(Preset::parse("adwaita"), Some(Preset::Adwaita));
        assert_eq!(Preset::parse("macos"), Some(Preset::MacOs));
        assert_eq!(Preset::parse("plain"), Some(Preset::Plain));
        assert_eq!(Preset::parse("aqua"), None);
    }
}
