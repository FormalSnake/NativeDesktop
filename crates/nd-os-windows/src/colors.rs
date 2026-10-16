//! The system accent, read the way WinUI reads it: `UISettings` gives the accent and three
//! lighter and three darker shades, and each scheme takes the shade WinUI's resources use
//! (AccentFillColorDefault is AccentDark1 in light, AccentLight2 in dark).

use nd_gpui::gpui::{Hsla, Rgba};
use nd_theme::{PaletteRefinement, SchemePalettes};
use windows::UI::Color;
use windows::UI::ViewManagement::{UIColorType, UISettings};

/// The accent palettes of both schemes; empty where `UISettings` is unavailable.
pub fn os_palettes() -> SchemePalettes {
    let Ok(settings) = UISettings::new() else { return SchemePalettes::default() };
    let shade = |kind: UIColorType| settings.GetColorValue(kind).ok().map(hsla);
    let (Some(light2), Some(light3), Some(dark1), Some(dark2), Some(dark3)) = (
        shade(UIColorType::AccentLight2),
        shade(UIColorType::AccentLight3),
        shade(UIColorType::AccentDark1),
        shade(UIColorType::AccentDark2),
        shade(UIColorType::AccentDark3),
    ) else {
        return SchemePalettes::default();
    };
    let white: Hsla = nd_gpui::gpui::white();
    let black: Hsla = nd_gpui::gpui::black();
    SchemePalettes {
        light: PaletteRefinement {
            accent: Some(dark1),
            on_accent: Some(white),
            accent_container: Some(light3),
            on_accent_container: Some(dark3),
            selection: Some(light2.opacity(0.4)),
            ..PaletteRefinement::default()
        },
        dark: PaletteRefinement {
            accent: Some(light2),
            on_accent: Some(black),
            accent_container: Some(dark2),
            on_accent_container: Some(white),
            selection: Some(dark1.opacity(0.5)),
            ..PaletteRefinement::default()
        },
    }
}

fn hsla(c: Color) -> Hsla {
    Rgba { r: c.R as f32 / 255., g: c.G as f32 / 255., b: c.B as f32 / 255., a: c.A as f32 / 255. }.into()
}
