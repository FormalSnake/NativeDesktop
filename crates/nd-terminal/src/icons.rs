//! Nerd Font icons from the bundled Symbols Nerd Font Mono, rasterized here (swash) rather than
//! by the platform: GPUI refuses to load a face without an `m` glyph, and drawing them the same
//! way on every presenter keeps the goldens comparable. Each icon is fitted into its box by
//! [`crate::glyph::fit`] before it is rasterized, so it is never clipped.

use std::cell::RefCell;

use swash::FontRef;
use swash::scale::{Render, ScaleContext, Source};
use swash::zeno::Format;

use crate::glyph::{self, Ink, SYMBOLS_FONT};
use crate::sprite::Mask;

thread_local! {
    static SCALER: RefCell<ScaleContext> = RefCell::new(ScaleContext::new());
}

fn font() -> FontRef<'static> {
    FontRef::from_index(SYMBOLS_FONT, 0).expect("the bundled symbols font parses")
}

/// True when the bundled face has a glyph for `c`.
pub fn covers(c: char) -> bool {
    font().charmap().map(c as u32) != 0
}

/// The icon for `c` as a coverage mask the size of its box (`width` by `height` device pixels,
/// baseline `baseline` from the top), drawn at `ppem` pixels per em and shrunk to fit.
pub fn mask(c: char, ppem: f32, width: u32, height: u32, baseline: f32) -> Option<Mask> {
    let font = font();
    let glyph = font.charmap().map(c as u32);
    if glyph == 0 || ppem <= 0. {
        return None;
    }
    SCALER.with(|ctx| {
        let mut ctx = ctx.borrow_mut();
        let bounds = ctx.builder(font).size(ppem).hint(false).build().scale_outline(glyph)?.bounds();
        // Outlines are y up from the pen; the fit works y down.
        let ink = Ink { x: bounds.min.x, y: -bounds.max.y, width: bounds.width(), height: bounds.height() };
        let fit = glyph::fit(ink, width as f32, height as f32, baseline, true);
        let mut scaler = ctx.builder(font).size(ppem * fit.scale).hint(false).build();
        let image = Render::new(&[Source::Outline]).format(Format::Alpha).render(&mut scaler, glyph)?;
        let mut out = Mask::new(width, height);
        let p = image.placement;
        // The image's top left, from the box's: the pen sits at the fit's point.
        let x0 = fit.x.round() as i32 + p.left;
        let y0 = fit.y.round() as i32 - p.top;
        for row in 0..p.height as i32 {
            for col in 0..p.width as i32 {
                let (x, y) = (x0 + col, y0 + row);
                if x < 0 || y < 0 || x >= width as i32 || y >= height as i32 {
                    continue;
                }
                let a = image.data[(row * p.width as i32 + col) as usize];
                let at = (y * width as i32 + x) as usize;
                out.alpha[at] = out.alpha[at].max(a);
            }
        }
        Some(out)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_icon_fills_its_box_without_clipping() {
        assert!(covers('\u{f115}'));
        assert!(!covers('a'));
        let m = mask('\u{f115}', 26., 16, 34, 26.).expect("folder icon");
        let lit: Vec<(u32, u32)> = (0..34)
            .flat_map(|y| (0..16).map(move |x| (x, y)))
            .filter(|(x, y)| m.alpha[(y * 16 + x) as usize] > 0)
            .collect();
        let (xs, ys): (Vec<u32>, Vec<u32>) = lit.iter().copied().unzip();
        let width = xs.iter().max().unwrap() - xs.iter().min().unwrap() + 1;
        // Scaled to the box's width, centred vertically with room above and below.
        assert!(width >= 14, "the icon uses the box's width, got {width}");
        assert!(*ys.iter().min().unwrap() > 2 && *ys.iter().max().unwrap() < 31, "{ys:?}");
    }
}
