//! The e1504g budget knobs (ARCHITECTURE.md 9.5, docs/plan/baselines.md), read from the
//! environment and silent unless set. Each prints `nd-budget: ...` lines on stdout:
//!
//! - `ND_EXIT_AFTER_FIRST_FRAME=1`: `first_frame unix_ms=<ms> since_start_ms=<ms>` once GPUI has
//!   drawn the first frame, then quits. The launcher subtracts its own timestamp from `unix_ms`.
//! - `ND_ANIMATE=<seconds>`: re-renders every window's root view every display frame for that
//!   long (the reconciler, the element build and GPUI's layout and paint all run), then prints
//!   `frames n=.. refresh_ms=.. p50_ms=.. p95_ms=.. p99_ms=.. max_ms=.. missed=..` and quits.
//! - `ND_BUDGET_CLICKS=<n>`: clicks the first button `n` times, 150 ms apart, through the window's
//!   own input path, and prints `input_to_frame n=.. p50_ms=.. p95_ms=.. max_ms=..`: from the
//!   click's dispatch to the start of the display frame after the one that drew its result, an
//!   upper bound on input to present until `wp_presentation` timestamps land (`nd-os-linux`).

use nd_core::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use nd_core::WindowId;

fn env_f64(name: &str) -> Option<f64> {
    std::env::var(name).ok()?.parse().ok()
}

fn quantile(sorted: &[f64], q: f64) -> f64 {
    sorted.get(((sorted.len() as f64 - 1.) * q).round() as usize).copied().unwrap_or(0.)
}

#[derive(Default)]
pub(crate) struct Budget {
    exit_after_first_frame: bool,
    animate: Option<Duration>,
    animate_from: Option<Instant>,
    frames: Vec<Instant>,
    /// Clicks still to send.
    pub clicks: u32,
    click_mode: bool,
    pending_input: Option<Instant>,
    latencies: Vec<f64>,
}

impl Budget {
    pub fn from_env() -> Self {
        Self {
            exit_after_first_frame: std::env::var_os("ND_EXIT_AFTER_FIRST_FRAME").is_some_and(|v| v == "1"),
            animate: env_f64("ND_ANIMATE").map(Duration::from_secs_f64),
            clicks: env_f64("ND_BUDGET_CLICKS").map_or(0, |n| n as u32),
            click_mode: std::env::var_os("ND_BUDGET_CLICKS").is_some(),
            ..Self::default()
        }
    }

    /// True while `ND_ANIMATE` keeps every frame busy.
    pub fn animating(&mut self, now: Instant) -> bool {
        let Some(limit) = self.animate else { return false };
        let from = *self.animate_from.get_or_insert(now);
        now - from < limit
    }

    /// An input reached the window; the next frame drawn after it closes the measurement.
    pub fn input(&mut self, at: Instant) {
        if self.click_mode {
            self.pending_input.get_or_insert(at);
        }
    }

    /// Whether the frame being drawn needs its present observed.
    pub fn wants_present(&self) -> bool {
        self.animate.is_some() || self.pending_input.is_some()
    }

    pub fn first_frame(&mut self, start: Instant) {
        if self.exit_after_first_frame {
            let unix_ms = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis();
            println!("nd-budget: first_frame unix_ms={unix_ms} since_start_ms={}", start.elapsed().as_millis());
        }
    }

    /// The display frame after one that started at `started` has begun. True when the knob in
    /// effect is done and the app should quit.
    pub fn presented(&mut self, _window: WindowId, started: Instant, refresh: Option<Duration>) -> bool {
        if self.exit_after_first_frame {
            return true;
        }
        let now = Instant::now();
        if let Some(t0) = self.pending_input
            && started >= t0
        {
            self.latencies.push((now - t0).as_secs_f64() * 1000.);
            self.pending_input = None;
            if self.clicks == 0 {
                self.report_latency();
                return true;
            }
        }
        let Some(limit) = self.animate else { return false };
        self.frames.push(now);
        if now - self.frames[0] < limit {
            return false;
        }
        let mut intervals: Vec<f64> = self.frames.windows(2).map(|w| (w[1] - w[0]).as_secs_f64() * 1000.).collect();
        intervals.sort_by(f64::total_cmp);
        let refresh = refresh.map_or(1000. / 60., |d| d.as_secs_f64() * 1000.);
        let missed = intervals.iter().filter(|i| **i > refresh * 1.5).count();
        println!(
            "nd-budget: frames n={} refresh_ms={refresh:.2} p50_ms={:.2} p95_ms={:.2} p99_ms={:.2} max_ms={:.2} missed={missed}",
            intervals.len(),
            quantile(&intervals, 0.5),
            quantile(&intervals, 0.95),
            quantile(&intervals, 0.99),
            intervals.last().copied().unwrap_or(0.),
        );
        true
    }

    fn report_latency(&mut self) {
        let mut l = std::mem::take(&mut self.latencies);
        l.sort_by(f64::total_cmp);
        println!(
            "nd-budget: input_to_frame n={} p50_ms={:.2} p95_ms={:.2} max_ms={:.2}",
            l.len(),
            quantile(&l, 0.5),
            quantile(&l, 0.95),
            l.last().copied().unwrap_or(0.)
        );
    }
}
