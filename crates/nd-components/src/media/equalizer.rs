//! The now-playing mark: bars rising and falling while a track plays, standing still while it is
//! paused, as Music draws it in place of a song's number and YouTube Music over its artwork.
//! Every backend draws the same bars from [`EQUALIZER`] and [`level`]; AppKit hands the motion to
//! Core Animation, GPUI paints it frame by frame. Reduce Motion stills them.

use std::panic::Location;

use nd_ui::prelude::*;
use nd_ui::semantics::{Semantics, WidgetSemantics};
use nd_ui::{Key, Role, StyleRefinement};

use crate::forms::leaf;

/// The bars' shape, as fractions of the square they stand in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EqualizerMetrics {
    pub bars: usize,
    /// Each bar's width and the gap after it, as fractions of the side.
    pub bar: f32,
    pub gap: f32,
    /// The lowest and highest a bar reaches, as fractions of the side.
    pub low: f32,
    pub high: f32,
    /// Seconds for one bar to rise and fall, per bar, so they never move in step.
    pub periods: [f32; 4],
    /// Where in its period each bar starts.
    pub phases: [f32; 4],
    /// The heights the bars hold while paused or under Reduce Motion.
    pub still: [f32; 4],
}

pub const EQUALIZER: EqualizerMetrics = EqualizerMetrics {
    bars: 4,
    bar: 0.14,
    gap: 0.08,
    low: 0.2,
    high: 0.9,
    periods: [0.9, 0.7, 1.1, 0.8],
    phases: [0.1, 0.6, 0.35, 0.8],
    still: [0.45, 0.8, 0.6, 0.35],
};

/// Bar `i`'s height, as a fraction of the side, `t` seconds into playing.
pub fn level(i: usize, t: f32) -> f32 {
    let m = EQUALIZER;
    let turn = (t / m.periods[i] + m.phases[i]).fract();
    let wave = 0.5 - 0.5 * (turn * std::f32::consts::TAU).cos();
    m.low + (m.high - m.low) * wave
}

/// The bars' frames in a `side` point square, bottom aligned and centred, at `heights`.
pub fn bar_frames(side: f32, heights: impl Fn(usize) -> f32) -> Vec<(f32, f32, f32, f32)> {
    let m = EQUALIZER;
    let width = (side * m.bar).max(1.);
    let gap = (side * m.gap).max(1.);
    let total = width * m.bars as f32 + gap * (m.bars - 1) as f32;
    let left = ((side - total) / 2.).round();
    (0..m.bars)
        .map(|i| {
            let h = (side * heights(i)).max(width);
            (left + i as f32 * (width + gap), side - h, width, h)
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq)]
pub struct EqualizerProps {
    /// Moving; `false` holds the bars still, as a paused track.
    pub playing: bool,
    /// The square it is drawn in, in points.
    pub size: f32,
    pub enabled: bool,
    pub tooltip: Option<SharedString>,
    /// `text_color` colours the bars; without one they take the accent.
    pub style: StyleRefinement,
}

/// Widget kind of `equalizer`.
pub struct EqualizerKind;

impl WidgetSemantics for EqualizerProps {
    fn semantics(&self) -> Semantics {
        Semantics::new(Role::Image, if self.playing { "Now playing" } else { "Paused" }).enabled(self.enabled)
    }
}

pub struct Equalizer(NativeElement<EqualizerProps>);

/// The now-playing bars in a `size` point square, moving while `playing`.
#[track_caller]
pub fn equalizer(size: f32, playing: bool) -> Equalizer {
    let props = EqualizerProps { playing, size, enabled: true, tooltip: None, style: StyleRefinement::default() };
    let el =
        NativeElement::described::<EqualizerKind>(Key::Site(Location::caller()), props).size(px(size)).flex_none();
    Equalizer(el)
}

impl Equalizer {
    pub fn id(mut self, id: impl Into<SharedString>) -> Self {
        *self.0.key_mut() = Key::Id(id.into());
        self
    }
}

leaf!(Equalizer, EqualizerProps);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bars_stay_in_range_and_out_of_step() {
        for step in 0..200 {
            let t = step as f32 * 0.05;
            for i in 0..EQUALIZER.bars {
                let l = level(i, t);
                assert!((EQUALIZER.low - 1e-4..=EQUALIZER.high + 1e-4).contains(&l));
            }
        }
        assert_ne!(level(0, 0.), level(1, 0.));
    }

    #[test]
    fn bars_sit_on_the_floor_inside_the_square() {
        let frames = bar_frames(16., |i| EQUALIZER.still[i]);
        assert_eq!(frames.len(), 4);
        for (x, y, w, h) in frames {
            assert!(x >= 0. && x + w <= 16.);
            assert!((y + h - 16.).abs() < 1e-4);
        }
    }
}
