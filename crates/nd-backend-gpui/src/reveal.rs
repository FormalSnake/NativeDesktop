//! `Pane::edge_reveal()` drawn: while the sidebar is collapsed, the pointer touching the
//! window's leading edge slides the sidebar in as a floating panel over the content, and it
//! slides away again 250 ms after the pointer leaves it (200 ms in, 150 ms out; a fade with
//! reduce motion). The content pane keeps its size, so a page under the panel does not relayout.
//! The same timing as AppKit's (`nd-backend-appkit/src/reveal.rs`).

use std::time::{Duration, Instant};

use nd_ui::Curve;

/// The panel's gap to the window edges.
pub(crate) const INSET: f32 = 8.;
pub(crate) const RADIUS: f32 = 16.;
const IN: Duration = Duration::from_millis(200);
const OUT: Duration = Duration::from_millis(150);
const LEAVE: Duration = Duration::from_millis(250);
const CURVE: Curve = Curve::Bezier(0.23, 1., 0.32, 1.);

fn slowdown() -> f32 {
    std::env::var("ND_ANIMATION_SLOWDOWN").ok().and_then(|v| v.parse::<f32>().ok()).filter(|v| *v >= 1.).unwrap_or(1.)
}

/// One sidebar's reveal: how far in the panel is (0 hidden, 1 shown) and where it is heading.
#[derive(Clone, Debug)]
pub(crate) struct Reveal {
    shown: bool,
    from: f32,
    start: Instant,
    duration: Duration,
    /// When the pointer left the panel, while the hide waits out its grace.
    left: Option<Instant>,
    /// Where the panel was last painted, in the window.
    pub area: Option<nd_core::Rect>,
}

impl Default for Reveal {
    fn default() -> Self {
        Self { shown: false, from: 0., start: Instant::now(), duration: Duration::ZERO, left: None, area: None }
    }
}

impl Reveal {
    pub fn shown(&self) -> bool {
        self.shown
    }

    /// How far in the panel is at `now`, 0 to 1. A slide that turns round starts from here.
    pub fn progress(&self, now: Instant) -> f32 {
        let to = if self.shown { 1. } else { 0. };
        let t = if self.duration.is_zero() {
            1.
        } else {
            now.duration_since(self.start).as_secs_f32() / self.duration.as_secs_f32()
        };
        self.from + (to - self.from) * CURVE.at(t)
    }

    /// Whether a slide or a pending hide still needs frames.
    pub fn moving(&self, now: Instant) -> bool {
        self.left.is_some() || now.duration_since(self.start) < self.duration
    }

    /// Slides in or out; returns whether that changed anything.
    pub fn set(&mut self, shown: bool, now: Instant) -> bool {
        self.left = None;
        if self.shown == shown {
            return false;
        }
        self.from = self.progress(now);
        self.shown = shown;
        self.start = now;
        self.duration = if shown { IN } else { OUT }.mul_f32(slowdown());
        true
    }

    /// The pointer left the panel (`Some`) or came back (`None`).
    pub fn pointer_left(&mut self, at: Option<Instant>) {
        self.left = at.filter(|_| self.shown);
    }

    /// Hides once the pointer has been gone [`LEAVE`]; returns whether it hid. A pointer that is
    /// over the panel (`pointer` in the window) keeps it, whatever the hover events said.
    pub fn settle(&mut self, now: Instant, pointer: nd_core::Point) -> bool {
        let r = self.area;
        if r.is_some_and(|r| {
            pointer.x >= r.origin.x
                && pointer.y >= r.origin.y
                && pointer.x < r.origin.x + r.size.width
                && pointer.y < r.origin.y + r.size.height
        }) {
            self.left = None;
        }
        match self.left {
            Some(left) if now.duration_since(left) >= LEAVE.mul_f32(slowdown()) => self.set(false, now),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slides_in_and_out_on_their_own_timings_and_turn_round_from_where_they_are() {
        let t0 = Instant::now();
        let mut r = Reveal::default();
        assert_eq!(r.progress(t0), 0.);
        assert!(r.set(true, t0));
        assert!(r.progress(t0 + Duration::from_millis(50)) > 0.3, "eases out: most of the way early");
        assert_eq!(r.progress(t0 + IN), 1.);
        let mid = t0 + Duration::from_millis(60);
        let at = r.progress(mid);
        assert!(r.set(false, mid));
        assert!((r.progress(mid) - at).abs() < 1e-4, "turns round where it got to");
        assert_eq!(r.progress(mid + OUT), 0.);
    }

    #[test]
    fn a_pointer_that_leaves_hides_it_after_the_grace_unless_it_comes_back() {
        let t0 = Instant::now();
        let mut r = Reveal::default();
        r.set(true, t0);
        r.area = Some(nd_core::Rect::new(8., 8., 240., 400.));
        let (away, over) = (nd_core::Point { x: 600., y: 100. }, nd_core::Point { x: 100., y: 100. });
        r.pointer_left(Some(t0));
        assert!(!r.settle(t0 + Duration::from_millis(200), away));
        r.pointer_left(None);
        assert!(!r.settle(t0 + Duration::from_millis(300), away), "came back in time");
        r.pointer_left(Some(t0));
        assert!(!r.settle(t0 + LEAVE, over), "a pointer over the panel keeps it");
        r.pointer_left(Some(t0));
        assert!(r.settle(t0 + LEAVE, away));
        assert!(!r.shown());
    }
}
