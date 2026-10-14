//! The playback clock media widgets read. A player reports its position now and then (once a
//! second, on a state change); between reports the widgets extrapolate from the last one, so
//! the seek bar and lyrics move every frame without the app re-rendering.

use nd_core::time::{Duration, Instant};

/// A position reported at an instant, and whether time is moving from there.
#[derive(Clone, Copy, Debug)]
pub struct PlaybackClock {
    pub position: Duration,
    pub at: Instant,
    pub playing: bool,
    /// Playback speed; 1 is normal.
    pub rate: f32,
}

impl PartialEq for PlaybackClock {
    /// A paused clock reads the same whenever it was reported, so rebuilding one each render
    /// sends no update.
    fn eq(&self, other: &Self) -> bool {
        self.position == other.position
            && self.playing == other.playing
            && self.rate == other.rate
            && (!self.playing || self.at == other.at)
    }
}

impl Default for PlaybackClock {
    fn default() -> Self {
        Self::paused(Duration::ZERO)
    }
}

impl PlaybackClock {
    /// Playing from `position`, reported now.
    pub fn playing(position: Duration) -> Self {
        Self { position, at: Instant::now(), playing: true, rate: 1. }
    }

    /// Stopped at `position`.
    pub fn paused(position: Duration) -> Self {
        Self { position, at: Instant::now(), playing: false, rate: 1. }
    }

    pub fn reported_at(mut self, at: Instant) -> Self {
        self.at = at;
        self
    }

    pub fn rate(mut self, rate: f32) -> Self {
        self.rate = rate.max(0.);
        self
    }

    /// The position at `now`, clamped to `duration` when it is known.
    pub fn position_at(&self, now: Instant, duration: Option<Duration>) -> Duration {
        let mut position = self.position;
        if self.playing {
            position += now.saturating_duration_since(self.at).mul_f32(self.rate);
        }
        match duration {
            Some(d) => position.min(d),
            None => position,
        }
    }

    pub fn now(&self, duration: Option<Duration>) -> Duration {
        self.position_at(Instant::now(), duration)
    }
}

/// `m:ss`, or `h:mm:ss` from an hour.
pub fn format_clock(d: Duration) -> String {
    let s = d.as_secs();
    let (h, m, s) = (s / 3600, (s / 60) % 60, s % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

/// Reads `1:23`, `1:02:03`, `83` or `83.5` seconds, or `50%` of `duration`.
pub fn parse_clock(text: &str, duration: Option<Duration>) -> Option<Duration> {
    let text = text.trim();
    if let Some(pct) = text.strip_suffix('%') {
        let f: f64 = pct.trim().parse().ok()?;
        return Some(duration?.mul_f64((f / 100.).clamp(0., 1.)));
    }
    if text.contains(':') {
        let mut total = 0f64;
        for part in text.split(':') {
            let v: f64 = part.trim().parse().ok()?;
            if v < 0. {
                return None;
            }
            total = total * 60. + v;
        }
        return Some(Duration::from_secs_f64(total));
    }
    let secs: f64 = text.parse().ok()?;
    (secs >= 0.).then(|| Duration::from_secs_f64(secs))
}
