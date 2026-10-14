//! Where a glyph sits in its cell, the same on every presenter. Text draws at its natural size
//! on the row's baseline; a glyph whose ink overflows its box (a fallback face drawn larger than
//! the grid's font) shrinks to fit and centres instead of being clipped. Nerd Font icons always
//! centre, and take two cells when the next one is blank, as Ghostty does, since they are drawn
//! about a cell and a half wide. The zig branch sliced them (`examples/terminal/nerd-*.png`).

use crate::frame::{Attrs, Row};

/// The bundled Symbols Nerd Font Mono (v3.5.1, MIT, `assets/SymbolsNerdFont-LICENSE`), the
/// face icons resolve to before the platform's fallback, so they draw without an installed
/// Nerd Font.
pub static SYMBOLS_FONT: &[u8] = include_bytes!("../assets/SymbolsNerdFontMono-Regular.ttf");
pub const SYMBOLS_FAMILY: &str = "Symbols Nerd Font Mono";

/// Ink inside a glyph's box may overhang the box by this much before the glyph is scaled:
/// italics and some CJK faces reach slightly past their advance by design.
pub const FIT_SLACK: f32 = 1.2;

/// True for the private use areas Nerd Fonts and Powerline extras live in, and the symbol
/// blocks icon-heavy prompts draw from.
pub fn is_symbol(c: char) -> bool {
    matches!(c as u32,
        0xE000..=0xF8FF | 0xF0000..=0xFFFFD | 0x100000..=0x10FFFD
        | 0x2300..=0x23FF | 0x2600..=0x27BF | 0x2B00..=0x2BFF)
}

/// Cells the glyph of `row.cells[col]` may use: two for a wide character or an icon followed
/// by a blank cell, else one.
pub fn box_cells(row: &Row, col: usize) -> u16 {
    let cell = &row.cells[col];
    if cell.attrs.contains(Attrs::WIDE) {
        return 2;
    }
    let symbol = row.grapheme(cell).chars().next().is_some_and(is_symbol);
    let next_blank = row.cells.get(col + 1).is_some_and(|n| n.is_blank() && !n.attrs.contains(Attrs::SPACER));
    if symbol && next_blank { 2 } else { 1 }
}

/// A glyph's ink rectangle relative to its pen position on the baseline, y down (`y` is
/// negative above the baseline).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Ink {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// The pen position inside the box (from the box's top left) and the scale to draw at.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fit {
    pub scale: f32,
    pub x: f32,
    pub y: f32,
}

/// Places a glyph with ink `ink` in a `box_width` by `box_height` box whose baseline is
/// `baseline` from the top.
pub fn fit(ink: Ink, box_width: f32, box_height: f32, baseline: f32, symbol: bool) -> Fit {
    let natural = Fit { scale: 1., x: 0., y: baseline };
    if ink.width <= 0. || ink.height <= 0. {
        return natural;
    }
    let overflows = ink.x < -0.5
        || ink.x + ink.width > box_width * FIT_SLACK
        || baseline + ink.y < -box_height * (FIT_SLACK - 1.)
        || baseline + ink.y + ink.height > box_height * FIT_SLACK;
    if !symbol && !overflows {
        return natural;
    }
    let scale = (box_width / ink.width).min(box_height / ink.height).min(1.);
    Fit {
        scale,
        x: box_width / 2. - (ink.x + ink.width / 2.) * scale,
        y: box_height / 2. - (ink.y + ink.height / 2.) * scale,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_that_fits_stays_on_the_baseline() {
        let f = fit(Ink { x: 1., y: -10., width: 6., height: 13. }, 8., 17., 13., false);
        assert_eq!(f, Fit { scale: 1., x: 0., y: 13. });
    }

    #[test]
    fn an_oversized_icon_shrinks_into_its_box_and_centres() {
        // An icon 15 wide on an 8 wide cell: scaled into 16 (two cells), centred.
        let ink = Ink { x: 0., y: -14., width: 15., height: 15. };
        let f = fit(ink, 16., 17., 13., true);
        assert_eq!(f.scale, 1.);
        assert_eq!((f.x, f.y), (0.5, 1. + 14.));
        let f = fit(ink, 8., 17., 13., true);
        assert!((f.scale - 8. / 15.).abs() < 1e-6);
        let left = f.x + ink.x * f.scale;
        let right = left + ink.width * f.scale;
        assert!(left >= -1e-4 && right <= 8. + 1e-4, "ink {left}..{right} stays in the cell");
    }

    #[test]
    fn symbol_ranges() {
        assert!(is_symbol('\u{e0a0}'));
        assert!(is_symbol('\u{f115}'));
        assert!(is_symbol('\u{f0001}'));
        assert!(!is_symbol('a'));
        assert!(!is_symbol('你'));
    }
}
