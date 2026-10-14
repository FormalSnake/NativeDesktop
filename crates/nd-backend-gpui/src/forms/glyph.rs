//! Every icon the controls draw, as GPUI paths from `nd-icons`' Lucide shapes, plus the arcs of
//! spinners and circular progress.

use std::f32::consts::PI;

use nd_components::Icon;
use nd_icons::Seg;
use nd_gpui::gpui::{self, Bounds, Hsla, IntoElement, PathBuilder, Pixels, Point, Styled, canvas, point};

/// An icon drawn as GPUI paths: Lucide's strokes (round caps and joins), tinted by the caller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Glyph(Icon);

#[allow(non_upper_case_globals)]
impl Glyph {
    pub const Check: Glyph = Glyph(Icon::Check);
    pub const Dash: Glyph = Glyph(Icon::Minus);
    pub const Minus: Glyph = Glyph(Icon::Minus);
    pub const Plus: Glyph = Glyph(Icon::Plus);
    pub const ChevronDown: Glyph = Glyph(Icon::ChevronDown);
    pub const ChevronUp: Glyph = Glyph(Icon::ChevronUp);
    pub const ChevronLeft: Glyph = Glyph(Icon::ChevronLeft);
    pub const ChevronRight: Glyph = Glyph(Icon::ChevronRight);
    pub const Search: Glyph = Glyph(Icon::Search);
    pub const Clear: Glyph = Glyph(Icon::X);
    pub const LocateFixed: Glyph = Glyph(Icon::LocateFixed);
    pub const Person: Glyph = Glyph(Icon::User);

    pub fn of(icon: Icon) -> Self {
        Glyph(icon)
    }

    /// The glyph for a Lucide icon name (what a button's `icon` carries), when nd or the app
    /// has an icon by that name.
    pub fn named(name: &str) -> Option<Self> {
        Icon::from_name(name).map(Glyph)
    }
}

fn arc_points(center: (f32, f32), radius: f32, start: f32, sweep: f32, segments: usize) -> Vec<(f32, f32)> {
    (0..=segments)
        .map(|i| {
            let a = start + sweep * i as f32 / segments as f32;
            (center.0 + radius * a.cos(), center.1 + radius * a.sin())
        })
        .collect()
}

fn stroke(bounds: Bounds<Pixels>, lines: &[Vec<(f32, f32)>], width: Pixels, color: Hsla, window: &mut gpui::Window) {
    let at = |(x, y): (f32, f32)| -> Point<Pixels> {
        point(bounds.origin.x + bounds.size.width * x, bounds.origin.y + bounds.size.height * y)
    };
    for line in lines {
        let mut path = PathBuilder::stroke(width);
        let mut points = line.iter();
        let Some(first) = points.next() else { continue };
        path.move_to(at(*first));
        for p in points {
            path.line_to(at(*p));
        }
        if let Ok(path) = path.build() {
            window.paint_path(path, color);
        }
    }
}

/// The tessellated paths of `glyph` in `bounds`, its strokes `width` thick however big the box is.
fn paths(glyph: Glyph, bounds: Bounds<Pixels>, width: Pixels) -> Vec<gpui::Path<Pixels>> {
    let Some(shape) = glyph.0.shape() else { return Vec::new() };
    let at = |x: f32, y: f32| -> Point<Pixels> {
        point(
            bounds.origin.x + bounds.size.width * (x / shape.size),
            bounds.origin.y + bounds.size.height * (y / shape.size),
        )
    };
    shape
        .paths
        .iter()
        .filter_map(|piece| {
            let mut path = if piece.filled { PathBuilder::fill() } else { PathBuilder::stroke(width) };
            for seg in piece.segs {
                match *seg {
                    Seg::Move(x, y) => path.move_to(at(x, y)),
                    Seg::Line(x, y) => path.line_to(at(x, y)),
                    Seg::Cubic(a, b, c, d, x, y) => path.cubic_bezier_to(at(x, y), at(a, b), at(c, d)),
                    Seg::Close => path.close(),
                }
            }
            path.build().ok()
        })
        .collect()
}

/// `glyph` drawn in `bounds`.
pub(crate) fn paint_glyph(glyph: Glyph, bounds: Bounds<Pixels>, color: Hsla, width: Pixels, window: &mut gpui::Window) {
    for path in paths(glyph, bounds, width) {
        window.paint_path(path, color);
    }
}

/// `glyph` in a `size` square.
pub(crate) fn glyph(glyph: Glyph, size: Pixels, color: Hsla, width: Pixels) -> impl IntoElement {
    canvas(|_, _, _| (), move |bounds, _, window, _| paint_glyph(glyph, bounds, color, width, window))
        .size(size)
        .flex_none()
}

/// An arc from `start` (radians, clockwise from three o'clock) through `sweep`, inset by half
/// the stroke so it stays inside a `size` square.
pub(crate) fn arc(size: Pixels, start: f32, sweep: f32, color: Hsla, width: Pixels) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            if sweep.abs() < 0.01 {
                return;
            }
            let inset = (width / bounds.size.width) / 2.;
            let segments = ((sweep.abs() / (2. * PI)) * 48.).ceil().max(4.) as usize;
            let points = arc_points((0.5, 0.5), 0.5 - inset, start, sweep, segments);
            stroke(bounds, &[points], width, color, window);
        },
    )
    .size(size)
    .flex_none()
}

#[cfg(test)]
mod tests {
    use nd_components::Icon;
    use nd_gpui::gpui::{Bounds, point, px, size};

    use super::{Glyph, paths};

    #[test]
    fn every_icon_tessellates() {
        let bounds = Bounds::new(point(px(0.), px(0.)), size(px(16.), px(16.)));
        for icon in Icon::ALL {
            let shape = icon.shape().unwrap_or_else(|| panic!("{} has no shape", icon.lucide()));
            let drawn = paths(Glyph::of(*icon), bounds, px(1.5));
            assert_eq!(drawn.len(), shape.paths.len(), "{} lost a path", icon.lucide());
        }
    }

    #[test]
    fn a_name_resolves_to_a_glyph() {
        assert!(Glyph::named("trash").is_some());
        assert!(Glyph::named("no-such-icon").is_none());
    }
}
