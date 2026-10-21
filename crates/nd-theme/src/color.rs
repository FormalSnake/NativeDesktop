//! Colour helpers the token derivation uses: sRGB mixing, alpha and hex parsing.

use nd_gpui::gpui::{Hsla, Rgba, black, white};

/// `#rrggbb` or `#rrggbbaa`, with or without the `#`. `None` for anything else.
pub fn parse_hex(text: &str) -> Option<Hsla> {
    let hex = text.trim().trim_start_matches('#');
    if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let channel = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok().map(|v| f32::from(v) / 255.);
    let (r, g, b, a) = match hex.len() {
        6 => (channel(0)?, channel(2)?, channel(4)?, 1.),
        8 => (channel(0)?, channel(2)?, channel(4)?, channel(6)?),
        _ => return None,
    };
    Some(Rgba { r, g, b, a }.into())
}

/// `#rrggbb`, or `#rrggbbaa` when not opaque.
pub fn to_hex(color: Hsla) -> String {
    let c = color.to_rgb();
    let byte = |v: f32| (v.clamp(0., 1.) * 255.).round() as u8;
    if byte(c.a) == 255 {
        format!("#{:02x}{:02x}{:02x}", byte(c.r), byte(c.g), byte(c.b))
    } else {
        format!("#{:02x}{:02x}{:02x}{:02x}", byte(c.r), byte(c.g), byte(c.b), byte(c.a))
    }
}

/// `a` moved towards `b` by `amount` (0 is `a`, 1 is `b`), in sRGB, alpha included.
pub fn mix(a: Hsla, b: Hsla, amount: f32) -> Hsla {
    let (a, b) = (a.to_rgb(), b.to_rgb());
    let t = amount.clamp(0., 1.);
    let lerp = |x: f32, y: f32| x + (y - x) * t;
    Rgba { r: lerp(a.r, b.r), g: lerp(a.g, b.g), b: lerp(a.b, b.b), a: lerp(a.a, b.a) }.into()
}

/// The same colour at `alpha`.
pub fn with_alpha(color: Hsla, alpha: f32) -> Hsla {
    Hsla { a: alpha.clamp(0., 1.), ..color }
}

/// `color` composited over an opaque `base`.
pub fn over(color: Hsla, base: Hsla) -> Hsla {
    let alpha = color.a;
    with_alpha(mix(base, with_alpha(color, 1.), alpha), 1.)
}

/// Relative luminance (WCAG), of the colour composited over itself as opaque.
pub fn luminance(color: Hsla) -> f32 {
    let c = color.to_rgb();
    let lin = |v: f32| if v <= 0.040_45 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
    0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b)
}

/// WCAG contrast ratio of two colours, each composited over itself as opaque.
pub fn contrast(a: Hsla, b: Hsla) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// `color` made just strong enough to reach `ratio` against every opaque colour in `under`:
/// a translucent colour gains alpha, an opaque one moves towards black or white. Colours that
/// already pass come back unchanged.
pub fn legible(color: Hsla, under: &[Hsla], ratio: f32) -> Hsla {
    let passes = |c: Hsla| under.iter().all(|b| contrast(over(c, *b), *b) >= ratio);
    if passes(color) {
        return color;
    }
    let towards = with_alpha(color, 1.);
    let ink = if under.iter().all(|b| is_dark(*b)) { white() } else { black() };
    for step in 1..=100 {
        let t = step as f32 / 100.;
        let candidate =
            if color.a < 1. { with_alpha(color, color.a + (1. - color.a) * t) } else { mix(towards, ink, t) };
        if passes(candidate) {
            return candidate;
        }
    }
    with_alpha(ink, 1.)
}

/// libadwaita's standalone colour of a background colour (`--accent-color` from
/// `--accent-bg-color`): its OKLab lightness capped at 0.5 on a light window, raised to at
/// least 0.85 on a dark one, hue and chroma kept.
pub fn standalone(color: Hsla, dark: bool) -> Hsla {
    let c = color.to_rgb();
    let lin = |v: f32| if v <= 0.040_45 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
    let (r, g, b) = (lin(c.r), lin(c.g), lin(c.b));
    let l = (0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b).cbrt();
    let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
    let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
    let lightness = 0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s;
    let a = 1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s;
    let bb = 0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s;
    let lightness = if dark { lightness.max(0.85) } else { lightness.min(0.5) };
    let l = (lightness + 0.396_337_78 * a + 0.215_803_76 * bb).powi(3);
    let m = (lightness - 0.105_561_346 * a - 0.063_854_17 * bb).powi(3);
    let s = (lightness - 0.089_484_18 * a - 1.291_485_5 * bb).powi(3);
    let gamma = |v: f32| {
        let v = v.clamp(0., 1.);
        if v <= 0.003_130_8 { 12.92 * v } else { 1.055 * v.powf(1. / 2.4) - 0.055 }
    };
    Rgba {
        r: gamma(4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s),
        g: gamma(-1.268_438 * l + 2.609_757_4 * m - 0.341_319_38 * s),
        b: gamma(-0.004_196_086_3 * l - 0.703_418_6 * m + 1.707_614_7 * s),
        a: c.a,
    }
    .into()
}

/// True for colours that read as dark (text on them should be light).
pub fn is_dark(color: Hsla) -> bool {
    luminance(color) < 0.18
}

/// Approximate equality, for tests and change detection after round trips through `f32`.
pub fn close(a: Hsla, b: Hsla) -> bool {
    let (a, b) = (a.to_rgb(), b.to_rgb());
    let d = |x: f32, y: f32| (x - y).abs() < 1.5 / 255.;
    d(a.r, b.r) && d(a.g, b.g) && d(a.b, b.b) && d(a.a, b.a)
}

#[cfg(test)]
mod tests {
    #[test]
    fn standalone_colours_match_libadwaitas() {
        use super::{parse_hex, standalone, to_hex};
        let hex = |s: &str| parse_hex(s).unwrap();
        assert_eq!(to_hex(standalone(hex("#e01b24"), false)), "#c30000");
        assert_eq!(to_hex(standalone(hex("#c01c28"), true)), "#ff938c");
        assert_eq!(to_hex(standalone(hex("#3584e4"), false)), "#0461be");
        assert_eq!(to_hex(standalone(hex("#3584e4"), true)), "#81d0ff");
    }

    use super::*;

    #[test]
    fn hex_round_trips() {
        for hex in ["#3d6bff", "#000000", "#ffffff", "#ff8a3d33"] {
            assert_eq!(to_hex(parse_hex(hex).unwrap()), hex);
        }
        assert!(parse_hex("#12345").is_none());
        assert!(parse_hex("rgb(1,2,3)").is_none());
    }

    #[test]
    fn mix_ends_are_inputs() {
        let a = parse_hex("#102030").unwrap();
        let b = parse_hex("#f0e0d0").unwrap();
        assert!(close(mix(a, b, 0.), a));
        assert!(close(mix(a, b, 1.), b));
    }
}
