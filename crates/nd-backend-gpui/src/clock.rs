//! A window's display clock on GPUI (ARCHITECTURE.md 5.3). GPUI drives each window's frames from
//! the display it is on (a per-display `CVDisplayLink` on macOS, `wl_surface.frame` on Wayland,
//! the window's CRTC on X11 with the fork's patch), so a frame the window draws is already paced
//! at that display's rate; this clock stamps it. `interval` follows the window's display:
//! GPUI's `refresh_interval`, updated when the window moves to another display.
//!
//! It also keeps the start times of recent frames, which is what `pacing` reports: the rate a
//! window actually drew at while it had work, next to its display's rate.

use nd_core::time::{Duration, Instant};
use std::collections::VecDeque;

use nd_core::FrameTime;

/// Frame starts kept for the pacing report: four seconds at 60 Hz, one at 240 Hz.
const RECENT: usize = 240;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pacing {
    /// The refresh rate of the window's display.
    pub display_hz: f64,
    /// The rate the window drew at during its latest run of consecutive frames; 0 when it has
    /// not drawn two frames in a row.
    pub measured_hz: f64,
    /// Frames in that run.
    pub frames: usize,
}

pub struct DisplayClock {
    origin: Instant,
    last: FrameTime,
    interval: f64,
    recent: VecDeque<f64>,
}

impl DisplayClock {
    /// `origin` is the app's start, shared by every window so their times compare.
    pub fn new(origin: Instant) -> Self {
        Self { origin, last: FrameTime::default(), interval: 1. / 60., recent: VecDeque::new() }
    }

    pub fn seconds(&self, at: Instant) -> f64 {
        at.saturating_duration_since(self.origin).as_secs_f64()
    }

    /// The display interval in seconds; 60 Hz until the platform reports one.
    pub fn interval(&self) -> f64 {
        self.interval
    }

    pub fn last(&self) -> FrameTime {
        self.last
    }

    /// Follows the window's display. Variable refresh displays report their maximum rate.
    pub fn set_interval(&mut self, interval: Option<Duration>) {
        if let Some(i) = interval.map(|d| d.as_secs_f64()).filter(|i| *i > 0.) {
            self.interval = i;
        }
    }

    /// The frame starting `now`: presented one display interval later as far as the backend
    /// can tell.
    pub fn tick(&mut self, now: Instant) -> FrameTime {
        let t = self.seconds(now);
        self.accept(FrameTime { frame: 0, timestamp: t, target: t + self.interval }, t)
    }

    /// Takes a frame time an OS crate measured. Frame numbers count on from the clock's own and
    /// targets never go backwards.
    pub fn accept(&mut self, time: FrameTime, started: f64) -> FrameTime {
        let time = FrameTime {
            frame: self.last.frame + 1,
            timestamp: time.timestamp,
            target: time.target.max(self.last.target),
        };
        self.last = time;
        if self.recent.len() == RECENT {
            self.recent.pop_front();
        }
        self.recent.push_back(started);
        time
    }

    /// The latest run of frames that followed each other within two display intervals.
    pub fn pacing(&self) -> Pacing {
        let display_hz = 1. / self.interval;
        let starts: Vec<f64> = self.recent.iter().copied().collect();
        let mut run: Vec<f64> = Vec::new();
        for pair in starts.windows(2).rev() {
            let gap = pair[1] - pair[0];
            if gap > self.interval * 2. {
                break;
            }
            run.push(gap);
        }
        if run.is_empty() {
            return Pacing { display_hz, measured_hz: 0., frames: 0 };
        }
        run.sort_by(f64::total_cmp);
        let median = run[run.len() / 2];
        Pacing { display_hz, measured_hz: if median > 0. { 1. / median } else { 0. }, frames: run.len() + 1 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(clock: &mut DisplayClock, start: Instant, hz: f64, frames: u32) -> Instant {
        let mut at = start;
        for _ in 0..frames {
            clock.tick(at);
            at += Duration::from_secs_f64(1. / hz);
        }
        at
    }

    #[test]
    fn targets_follow_the_display_and_never_go_backwards() {
        let origin = Instant::now();
        let mut clock = DisplayClock::new(origin);
        clock.set_interval(Some(Duration::from_secs_f64(1. / 240.)));
        let a = clock.tick(origin + Duration::from_millis(10));
        assert!((a.interval() - 1. / 240.).abs() < 1e-9);
        // Moved to the 60 Hz display: the next frame's target is one 60 Hz interval out.
        clock.set_interval(Some(Duration::from_secs_f64(1. / 60.)));
        let b = clock.tick(origin + Duration::from_millis(11));
        assert_eq!(b.frame, a.frame + 1);
        assert!(b.target >= a.target);
        assert!((b.interval() - 1. / 60.).abs() < 1e-9);
        // A measured time from an OS crate that lands earlier is clamped.
        let c = clock.accept(FrameTime { frame: 99, timestamp: 0., target: 0. }, 0.012);
        assert_eq!(c.frame, b.frame + 1);
        assert_eq!(c.target, b.target);
    }

    #[test]
    fn pacing_reports_the_rate_of_the_latest_run() {
        let origin = Instant::now();
        let mut clock = DisplayClock::new(origin);
        clock.set_interval(Some(Duration::from_secs_f64(1. / 240.)));
        let at = run(&mut clock, origin, 240., 120);
        let p = clock.pacing();
        assert!((p.measured_hz - 240.).abs() < 1., "{p:?}");
        assert_eq!(p.display_hz.round(), 240.);
        // Idle for a second, then a run on the 144 Hz display: only the new run counts.
        clock.set_interval(Some(Duration::from_secs_f64(1. / 144.)));
        run(&mut clock, at + Duration::from_secs(1), 144., 60);
        let p = clock.pacing();
        assert!((p.measured_hz - 144.).abs() < 1., "{p:?}");
        assert_eq!(p.frames, 60);
    }

    #[test]
    fn a_single_frame_has_no_rate() {
        let origin = Instant::now();
        let mut clock = DisplayClock::new(origin);
        clock.tick(origin);
        assert_eq!(clock.pacing().measured_hz, 0.);
    }
}
