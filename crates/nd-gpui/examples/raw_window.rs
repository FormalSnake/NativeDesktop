//! A bare GPUI window with no nd code: the baseline every nd number on e1504g is compared with
//! (ARCHITECTURE.md 9.5, docs/plan/baselines.md). `--features fast` runs it on gpui-fast.
//!
//! Prints one `nd-raw: key=value ...` line per measurement on stdout:
//! - `first_frame unix_ms=<ms> since_main_ms=<ms>` once GPUI has drawn the first frame. The
//!   baseline script subtracts its own launch timestamp from `unix_ms` for cold start.
//! - with `ND_RAW_ANIMATE=<seconds>`: redraws every frame for that long, then prints
//!   `frames n=.. refresh_ms=.. p50_ms=.. p95_ms=.. p99_ms=.. max_ms=.. missed=..` (frame to
//!   frame intervals; `missed` counts intervals over 1.5 refresh periods) and quits.
//! - with `ND_RAW_EXIT_AFTER_FIRST_FRAME=1`: quits right after `first_frame`.
//!
//! Otherwise it stays open and idle, which is how idle CPU and RSS are sampled.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use nd_gpui::{gpui, gpui_platform};

use gpui::{
    App, AppContext, Bounds, Context, IntoElement, ParentElement, Render, Styled, Window, WindowBounds, WindowOptions,
    div, px, relative, rgb, size,
};

struct Raw {
    started: Instant,
    animate: Option<Duration>,
    exit_after_first_frame: bool,
    first_frame_seen: bool,
    frames: Vec<Instant>,
}

impl Raw {
    fn report_frames(&self, window: &Window, cx: &App) {
        let mut intervals: Vec<f64> = self.frames.windows(2).map(|w| (w[1] - w[0]).as_secs_f64() * 1000.).collect();
        intervals.sort_by(|a, b| a.total_cmp(b));
        let refresh = window.refresh_interval(cx).map_or(1000. / 60., |d| d.as_secs_f64() * 1000.);
        let pick = |q: f64| intervals.get(((intervals.len() as f64 - 1.) * q).round() as usize).copied().unwrap_or(0.);
        let missed = intervals.iter().filter(|i| **i > refresh * 1.5).count();
        println!(
            "nd-raw: frames n={} refresh_ms={refresh:.2} p50_ms={:.2} p95_ms={:.2} p99_ms={:.2} max_ms={:.2} missed={missed}",
            intervals.len(),
            pick(0.5),
            pick(0.95),
            pick(0.99),
            intervals.last().copied().unwrap_or(0.),
        );
    }
}

impl Render for Raw {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.first_frame_seen {
            self.first_frame_seen = true;
            let started = self.started;
            let exit = self.exit_after_first_frame;
            // Runs once the frame this render belongs to has been drawn.
            window.on_next_frame(move |_, cx| {
                let unix_ms = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis();
                println!("nd-raw: first_frame unix_ms={unix_ms} since_main_ms={}", started.elapsed().as_millis());
                if exit {
                    cx.quit();
                }
            });
        }
        let mut offset = 0.;
        if let Some(limit) = self.animate {
            let now = Instant::now();
            self.frames.push(now);
            if now - self.frames[0] >= limit {
                self.report_frames(window, cx);
                cx.quit();
            } else {
                window.request_animation_frame();
            }
            offset = (self.frames.len() % 240) as f32;
        }
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_3()
            .p(px(24.))
            .bg(rgb(0x1e1e1e))
            .text_color(rgb(0xf0f0f0))
            .child("Raw gpui-pre window")
            .child(div().text_sm().child("No nd code: the e1504g baseline."))
            .child(
                div()
                    .w(relative(1.))
                    .h(px(40.))
                    .child(div().ml(px(offset)).size(px(40.)).bg(rgb(0x3d7eff)).rounded(px(8.))),
            )
    }
}

fn main() {
    let started = Instant::now();
    let animate = std::env::var("ND_RAW_ANIMATE").ok().and_then(|s| s.parse::<f64>().ok()).map(Duration::from_secs_f64);
    let exit_after_first_frame = std::env::var_os("ND_RAW_EXIT_AFTER_FIRST_FRAME").is_some_and(|v| v == "1");
    gpui_platform::application().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(640.), px(400.)), cx);
        cx.open_window(
            WindowOptions { window_bounds: Some(WindowBounds::Windowed(bounds)), ..Default::default() },
            |_, cx| {
                cx.new(|_| Raw {
                    started,
                    animate,
                    exit_after_first_frame,
                    first_frame_seen: false,
                    frames: Vec::new(),
                })
            },
        )
        .expect("open window");
        cx.activate(true);
    });
}
