//! Presets: the metrics, type, motion and materials of one platform look. Colours are not part
//! of a preset; they come from the palette chain. Where the OS publishes a value (text style
//! sizes, control heights) the backend reads it into [`SystemMetrics`] and it replaces the
//! preset's own, so composed content follows the OS (ARCHITECTURE.md 3.10).

use std::time::Duration;

use nd_gpui::gpui::{Font, Pixels, font, px};

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
    /// The preset of the platform this binary was built for.
    pub fn current() -> Preset {
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

#[derive(Clone, Debug, PartialEq)]
pub struct Fonts {
    pub body: Font,
    /// SF Pro Rounded on macOS; the body font elsewhere.
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

/// The font family nd resolves to the platform's UI font with a design: `""` is the default
/// design, `"rounded"` and `"monospaced"` the others. Backends map these names; elsewhere they
/// fall back to the body font.
pub const SYSTEM_FONT: &str = ".SystemUIFont";
pub const SYSTEM_FONT_ROUNDED: &str = ".SystemUIFont.rounded";
pub const SYSTEM_FONT_MONO: &str = ".SystemUIFont.monospaced";

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
        let controls =
            |small, regular, large| ControlHeights { small: px(small), regular: px(regular), large: px(large) };
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
                control: controls(22., 28., 34.),
                type_scale: scale(10., 10., 13., 13., 22., 26.),
                fonts: Fonts {
                    body: font(SYSTEM_FONT),
                    rounded: font(SYSTEM_FONT_ROUNDED),
                    mono: font(SYSTEM_FONT_MONO),
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
                control: controls(24., 32., 40.),
                type_scale: scale(12., 12., 14., 14., 20., 28.),
                fonts: Fonts {
                    body: font("Segoe UI Variable Text"),
                    rounded: font("Segoe UI Variable Text"),
                    mono: font("Cascadia Mono"),
                },
                motion: Motion {
                    fast: ms(83),
                    normal: ms(167),
                    slow: ms(333),
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
                control: controls(24., 34., 42.),
                type_scale: scale(11., 12., 14., 14., 20., 26.),
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
                    glass: Material::Transparent,
                },
            },
            Preset::Plain => Metrics {
                spacing: spacing(4., 8., 12., 16., 24., 16., 680.),
                radius: radii(4., 6., 8., 12.),
                control: controls(24., 32., 40.),
                type_scale: scale(12., 12., 14., 14., 20., 28.),
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
}
