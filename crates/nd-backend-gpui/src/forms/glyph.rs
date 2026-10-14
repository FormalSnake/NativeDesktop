//! The few glyphs the controls draw themselves (check marks, arrows, the search magnifier, the
//! media transport) and the arcs of spinners and circular progress, as GPUI paths: stroked
//! lines, and filled shapes where a stroke reads wrong at icon size (play, pause, the speaker).
//! Field edge icons fall back to a ring for any icon without a glyph here, until `nd-icons`
//! reaches the GPUI backend.

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
    Play,
    Pause,
    SkipBack,
    SkipForward,
    Volume,
    Volume1,
    Volume2,
    VolumeX,
    Paperclip,
    Smile,
    ArrowUp,
    Music,
    LocateFixed,
    Image,
    Person,
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
            Icon::Play => Glyph::Play,
            Icon::Pause => Glyph::Pause,
            Icon::SkipBack => Glyph::SkipBack,
            Icon::SkipForward => Glyph::SkipForward,
            Icon::Volume => Glyph::Volume,
            Icon::Volume1 => Glyph::Volume1,
            Icon::Volume2 => Glyph::Volume2,
            Icon::VolumeX => Glyph::VolumeX,
            Icon::Paperclip => Glyph::Paperclip,
            Icon::Smile => Glyph::Smile,
            Icon::ArrowUp => Glyph::ArrowUp,
            Icon::Music => Glyph::Music,
            Icon::LocateFixed => Glyph::LocateFixed,
            Icon::Image => Glyph::Image,
            Icon::User => Glyph::Person,
            _ => Glyph::Ring,
        }
    }

    /// The glyph for a Lucide icon name (what a button's `icon` carries), when there is one.
    pub fn named(name: &str) -> Option<Self> {
        Some(match name {
            "check" | "circle-check" => Glyph::Check,
            "minus" => Glyph::Minus,
            "plus" => Glyph::Plus,
            "chevron-down" => Glyph::ChevronDown,
            "chevron-up" => Glyph::ChevronUp,
            "chevron-left" => Glyph::ChevronLeft,
            "chevron-right" => Glyph::ChevronRight,
            "search" => Glyph::Search,
            "x" | "circle-x" => Glyph::Clear,
            "play" => Glyph::Play,
            "pause" => Glyph::Pause,
            "skip-back" => Glyph::SkipBack,
            "skip-forward" => Glyph::SkipForward,
            "volume" => Glyph::Volume,
            "volume-1" => Glyph::Volume1,
            "volume-2" => Glyph::Volume2,
            "volume-x" => Glyph::VolumeX,
            "paperclip" => Glyph::Paperclip,
            "smile" => Glyph::Smile,
            "arrow-up" => Glyph::ArrowUp,
            "music" => Glyph::Music,
            "locate-fixed" => Glyph::LocateFixed,
            "image" => Glyph::Image,
            "user" => Glyph::Person,
            _ => return None,
        })
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
            Glyph::Play | Glyph::Pause | Glyph::SkipBack | Glyph::SkipForward | Glyph::Volume | Glyph::Person => {
                Vec::new()
            }
            Glyph::Volume1 => vec![arc_points((0.48, 0.5), 0.16, -0.9, 1.8, 12)],
            Glyph::Volume2 => {
                vec![arc_points((0.48, 0.5), 0.16, -0.9, 1.8, 12), arc_points((0.48, 0.5), 0.32, -0.9, 1.8, 16)]
            }
            Glyph::VolumeX => vec![vec![(0.62, 0.38), (0.86, 0.62)], vec![(0.86, 0.38), (0.62, 0.62)]],
            Glyph::Paperclip => {
                let mut clip = vec![(0.64, 0.3), (0.64, 0.66)];
                clip.extend(arc_points((0.5, 0.66), 0.14, 0., PI, 12));
                clip.extend([(0.36, 0.66), (0.36, 0.28)]);
                clip.extend(arc_points((0.46, 0.28), 0.1, PI, PI, 10));
                clip.extend([(0.56, 0.28), (0.56, 0.62)]);
                vec![clip]
            }
            Glyph::Smile => {
                vec![arc_points((0.5, 0.5), 0.36, 0., 2. * PI, 32), arc_points((0.5, 0.5), 0.19, 0.5, PI - 1., 12)]
            }
            Glyph::ArrowUp => vec![vec![(0.5, 0.8), (0.5, 0.22)], vec![(0.26, 0.45), (0.5, 0.21), (0.74, 0.45)]],
            Glyph::Music => vec![vec![(0.62, 0.7), (0.62, 0.2), (0.82, 0.28)]],
            Glyph::LocateFixed => vec![
                arc_points((0.5, 0.5), 0.28, 0., 2. * PI, 28),
                vec![(0.5, 0.06), (0.5, 0.22)],
                vec![(0.5, 0.78), (0.5, 0.94)],
                vec![(0.06, 0.5), (0.22, 0.5)],
                vec![(0.78, 0.5), (0.94, 0.5)],
            ],
            Glyph::Image => vec![
                vec![(0.15, 0.2), (0.85, 0.2), (0.85, 0.8), (0.15, 0.8), (0.15, 0.2)],
                vec![(0.15, 0.72), (0.4, 0.48), (0.6, 0.66), (0.7, 0.57), (0.85, 0.7)],
            ],
        }
    }

    /// Filled polygons in a unit square.
    fn fills(self) -> Vec<Vec<(f32, f32)>> {
        let speaker = vec![(0.12, 0.38), (0.28, 0.38), (0.48, 0.2), (0.48, 0.8), (0.28, 0.62), (0.12, 0.62)];
        let rect = |x0: f32, y0: f32, x1: f32, y1: f32| vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1)];
        match self {
            Glyph::Play => vec![vec![(0.3, 0.2), (0.8, 0.5), (0.3, 0.8)]],
            Glyph::Pause => vec![rect(0.27, 0.2, 0.42, 0.8), rect(0.58, 0.2, 0.73, 0.8)],
            Glyph::SkipForward => vec![vec![(0.22, 0.22), (0.64, 0.5), (0.22, 0.78)], rect(0.68, 0.22, 0.78, 0.78)],
            Glyph::SkipBack => vec![vec![(0.78, 0.22), (0.36, 0.5), (0.78, 0.78)], rect(0.22, 0.22, 0.32, 0.78)],
            Glyph::Volume | Glyph::Volume1 | Glyph::Volume2 | Glyph::VolumeX => vec![speaker],
            Glyph::Smile => {
                vec![arc_points((0.38, 0.4), 0.05, 0., 2. * PI, 10), arc_points((0.62, 0.4), 0.05, 0., 2. * PI, 10)]
            }
            Glyph::Music => vec![arc_points((0.5, 0.7), 0.12, 0., 2. * PI, 16)],
            Glyph::LocateFixed => vec![arc_points((0.5, 0.5), 0.08, 0., 2. * PI, 12)],
            Glyph::Image => vec![arc_points((0.66, 0.36), 0.06, 0., 2. * PI, 10)],
            Glyph::Person => {
                let mut shoulders = arc_points((0.5, 0.86), 0.28, PI, PI, 16);
                shoulders.retain(|p| p.1 <= 0.86);
                vec![arc_points((0.5, 0.38), 0.15, 0., 2. * PI, 20), shoulders]
            }
            _ => Vec::new(),
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

fn fill(bounds: Bounds<Pixels>, shapes: &[Vec<(f32, f32)>], color: Hsla, window: &mut gpui::Window) {
    let at = |(x, y): (f32, f32)| -> Point<Pixels> {
        point(bounds.origin.x + bounds.size.width * x, bounds.origin.y + bounds.size.height * y)
    };
    for shape in shapes {
        let mut path = PathBuilder::fill();
        let mut points = shape.iter();
        let Some(first) = points.next() else { continue };
        path.move_to(at(*first));
        for p in points {
            path.line_to(at(*p));
        }
        path.close();
        if let Ok(path) = path.build() {
            window.paint_path(path, color);
        }
    }
}

/// `glyph` drawn in `bounds`.
pub(crate) fn paint_glyph(glyph: Glyph, bounds: Bounds<Pixels>, color: Hsla, width: Pixels, window: &mut gpui::Window) {
    stroke(bounds, &glyph.strokes(), width, color, window);
    fill(bounds, &glyph.fills(), color, window);
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
