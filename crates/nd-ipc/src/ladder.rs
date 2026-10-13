//! Reconnect backoff with a stability window, ported from `@nativedesktop/rpc`'s
//! `ConnectionLadder`: delays double from 500 ms to 16 s, and a drop after a connection that
//! held for 30 s starts again at the bottom instead of inheriting an old outage's backoff.
//! Pure: callers pass the clock, so it tests without sleeping.

use std::time::{Duration, Instant};

pub const RECONNECT_BASE: Duration = Duration::from_millis(500);
pub const RECONNECT_MAX: Duration = Duration::from_secs(16);
pub const STABILITY_WINDOW: Duration = Duration::from_secs(30);

/// 500 ms, 1 s, 2 s, 4 s, 8 s, 16 s, 16 s, ... `attempt` below 1 counts as 1.
pub fn backoff_delay(attempt: u32, base: Duration, max: Duration) -> Duration {
    let doublings = attempt.max(1) - 1;
    base.checked_mul(1u32.checked_shl(doublings).unwrap_or(u32::MAX)).map_or(max, |d| d.min(max))
}

#[derive(Clone, Copy, Debug)]
pub struct LadderConfig {
    pub base: Duration,
    pub max: Duration,
    pub stability_window: Duration,
}

impl Default for LadderConfig {
    fn default() -> Self {
        Self { base: RECONNECT_BASE, max: RECONNECT_MAX, stability_window: STABILITY_WINDOW }
    }
}

#[derive(Debug, Default)]
pub struct Ladder {
    config: LadderConfig,
    attempt: u32,
    connected_at: Option<Instant>,
}

impl Ladder {
    pub fn new(config: LadderConfig) -> Self {
        Self { config, attempt: 0, connected_at: None }
    }

    pub fn attempt(&self) -> u32 {
        self.attempt
    }

    /// Advances one rung and returns the wait before that attempt.
    pub fn next_delay(&mut self) -> Duration {
        self.attempt = self.attempt.saturating_add(1);
        backoff_delay(self.attempt, self.config.base, self.config.max)
    }

    pub fn note_connected(&mut self, now: Instant) {
        self.connected_at = Some(now);
    }

    /// Resets the ladder when the connection that just dropped had been up for the stability
    /// window.
    pub fn note_disconnected(&mut self, now: Instant) {
        if let Some(at) = self.connected_at.take()
            && now.saturating_duration_since(at) >= self.config.stability_window
        {
            self.attempt = 0;
        }
    }

    /// Manual retry: back to the bottom unconditionally.
    pub fn reset(&mut self) {
        self.attempt = 0;
        self.connected_at = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn doubles_and_caps() {
        let got: Vec<_> = (1..=7).map(|n| backoff_delay(n, RECONNECT_BASE, RECONNECT_MAX)).collect();
        assert_eq!(got, [500, 1000, 2000, 4000, 8000, 16000, 16000].map(ms));
        assert_eq!(backoff_delay(0, RECONNECT_BASE, RECONNECT_MAX), ms(500));
        assert_eq!(backoff_delay(400, RECONNECT_BASE, RECONNECT_MAX), RECONNECT_MAX);
        assert_eq!(backoff_delay(3, ms(100), ms(400)), ms(400));
    }

    #[test]
    fn short_connection_keeps_the_rung_long_one_resets() {
        let mut ladder = Ladder::new(LadderConfig::default());
        let t0 = Instant::now();
        ladder.next_delay();
        ladder.next_delay();
        assert_eq!(ladder.attempt(), 2);

        ladder.note_connected(t0);
        ladder.note_disconnected(t0 + Duration::from_secs(5));
        assert_eq!(ladder.attempt(), 2);
        assert_eq!(ladder.next_delay(), ms(2000));

        ladder.note_connected(t0);
        ladder.note_disconnected(t0 + STABILITY_WINDOW);
        assert_eq!(ladder.attempt(), 0);
        assert_eq!(ladder.next_delay(), RECONNECT_BASE);
    }
}
