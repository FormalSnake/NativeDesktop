//! The few glyphs the controls draw themselves (check marks, arrows, the search magnifier) and
//! the arcs of spinners and circular progress, as stroked GPUI paths. Field edge icons fall back
//! to a ring for any icon without a glyph here, until `nd-icons` reaches the GPUI backend.

use std::f32::consts::PI;

use nd_components::Icon;
use nd_gpui::gpui::{self, Bounds, Hsla, IntoElement, PathBuilder, Pixels, Point, Styled, canvas, point};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Glyph {
    Check,
    Dash,
    ChevronDown,
    ChevronUp,
    ChevronLeft,
    ChevronRight,
    Plus,
    Minus,
    Search,
    Clear,
    Ring,
}

impl Glyph {
    pub fn of(icon: Icon) -> Self {
        match icon {
            Icon::Check | Icon::CircleCheck => Glyph::Check,
            Icon::Minus => Glyph::Minus,
            Icon::Plus => Glyph::Plus,
            Icon::ChevronLeft => Glyph::ChevronLeft,
            Icon::ChevronRight => Glyph::ChevronRight,
            Icon::Search => Glyph::Search,
            Icon::X | Icon::CircleX => Glyph::Clear,
            _ => Glyph::Ring,
        }
    }

    /// Stroked polylines in a unit square.
    fn strokes(self) -> Vec<Vec<(f32, f32)>> {
        match self {
            Glyph::Check => vec![vec![(0.2, 0.52), (0.42, 0.74), (0.8, 0.28)]],
            Glyph::Dash | Glyph::Minus => vec![vec![(0.24, 0.5), (0.76, 0.5)]],
            Glyph::Plus => vec![vec![(0.24, 0.5), (0.76, 0.5)], vec![(0.5, 0.24), (0.5, 0.76)]],
            Glyph::ChevronDown => vec![vec![(0.25, 0.38), (0.5, 0.63), (0.75, 0.38)]],
            Glyph::ChevronUp => vec![vec![(0.25, 0.62), (0.5, 0.37), (0.75, 0.62)]],
            Glyph::ChevronLeft => vec![vec![(0.62, 0.25), (0.37, 0.5), (0.62, 0.75)]],
            Glyph::ChevronRight => vec![vec![(0.38, 0.25), (0.63, 0.5), (0.38, 0.75)]],
            Glyph::Clear => vec![vec![(0.28, 0.28), (0.72, 0.72)], vec![(0.72, 0.28), (0.28, 0.72)]],
            Glyph::Search => vec![arc_points((0.43, 0.43), 0.27, 0., 2. * PI, 24), vec![(0.63, 0.63), (0.86, 0.86)]],
            Glyph::Ring => vec![arc_points((0.5, 0.5), 0.32, 0., 2. * PI, 24)],
        }
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

/// `glyph` stroked in a `size` square.
pub(crate) fn glyph(glyph: Glyph, size: Pixels, color: Hsla, width: Pixels) -> impl IntoElement {
    canvas(|_, _, _| (), move |bounds, _, window, _| stroke(bounds, &glyph.strokes(), width, color, window))
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
