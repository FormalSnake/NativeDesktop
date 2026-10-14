//! The 100k-line scroll budget: a session holding 100k lines of coloured output, scrolled back
//! through its scrollback a few rows per frame (a trackpad) and many rows per frame (a fling),
//! one frame drawn per step. Timed per frame on the main thread: the scroll, libghostty's
//! snapshot, the presenter's build and, on AppKit, the Metal pass waited for. On the GPUI
//! presenter each step is a wheel event through the window's own input path and one headless
//! draw. Optimized builds fail when p95 is over the 60 Hz period; debug builds report.
//! `ND_LINES`, `ND_FRAMES` override the size of the run.

mod support;

use std::time::Instant;

use nd_terminal::{SessionOptions, TerminalSession};
use nd_theme::{Preset, ThemeRegistry};

const BUDGET_MS: f64 = 1000. / 60.;

fn quantile(sorted: &[f64], q: f64) -> f64 {
    sorted.get(((sorted.len() as f64 - 1.) * q).round() as usize).copied().unwrap_or(0.)
}

/// A session with `lines` lines of `ls -l`-like output in its scrollback.
fn filled(lines: usize) -> TerminalSession {
    let session = TerminalSession::detached(SessionOptions { cols: 120, rows: 40, scrollback: lines + 100 });
    let started = Instant::now();
    let mut chunk = String::with_capacity(1 << 20);
    for i in 0..lines {
        use std::fmt::Write;
        let _ = write!(
            chunk,
            "\x1b[2m{i:>7}\x1b[0m -rw-r--r--  1 kyan staff \x1b[33m{:>8}\x1b[0m Oct 14 12:00 \x1b[1;34mcrates/nd-terminal/src/file_{i}.rs\x1b[0m \x1b[32m✓\x1b[0m\r\n",
            (i * 7919) % 1_000_000
        );
        if chunk.len() > 1 << 20 {
            session.feed(chunk.as_bytes());
            chunk.clear();
        }
    }
    session.feed(chunk.as_bytes());
    let secs = started.elapsed().as_secs_f64();
    println!("nd-budget: terminal_parse lines={lines} secs={secs:.3} lines_per_sec={:.0}", lines as f64 / secs);
    assert!(session.scrollback_rows() >= lines - 40, "the scrollback kept every line");
    session
}

fn report(presenter: &str, step: isize, work: &[f64]) -> f64 {
    let mut sorted = work.to_vec();
    sorted.sort_by(f64::total_cmp);
    let over = work.iter().filter(|w| **w > BUDGET_MS).count();
    let p95 = quantile(&sorted, 0.95);
    println!(
        "nd-budget: terminal_scroll presenter={presenter} rows_per_frame={step} frames={} p50_ms={:.2} p95_ms={p95:.2} p99_ms={:.2} max_ms={:.2} over_60hz={over}",
        work.len(),
        quantile(&sorted, 0.5),
        quantile(&sorted, 0.99),
        sorted.last().copied().unwrap_or(0.),
    );
    p95
}

fn settings() -> (usize, usize) {
    let lines = std::env::var("ND_LINES").ok().and_then(|v| v.parse().ok()).unwrap_or(100_000);
    let frames = std::env::var("ND_FRAMES").ok().and_then(|v| v.parse().ok()).unwrap_or(600);
    (lines, frames)
}

fn check(p95: f64) {
    if !cfg!(debug_assertions) {
        assert!(p95 <= BUDGET_MS, "p95 frame work {p95:.2} ms is over the 60 Hz period");
    }
}

mod gate_m9_terminal {
    use super::*;

    #[cfg(target_os = "macos")]
    pub fn appkit_scroll_budget() {
        let (lines, frames) = settings();
        let session = filled(lines);
        let registry = ThemeRegistry::new(Preset::current());
        let mut surface =
            nd_terminal::appkit::Offscreen::new(registry.app_theme(), 13., 6., (900., 560.), 2.).expect("Metal");
        surface.fit(&session);
        surface.draw(&session);
        for step in [3isize, 40] {
            session.scroll_to_bottom();
            let mut work = Vec::with_capacity(frames);
            for _ in 0..frames {
                let started = Instant::now();
                session.scroll_rows(-step);
                surface.draw(&session);
                work.push(started.elapsed().as_secs_f64() * 1000.);
            }
            check(report("appkit", step, &work));
        }
    }

    #[cfg(any(not(target_os = "macos"), feature = "gpui"))]
    pub fn gpui_scroll_budget() {
        use std::sync::Arc;

        use nd_backend_gpui::GpuiHost;
        use nd_gpui::gpui::{
            self, HeadlessAppContext, Modifiers, PlatformInput, ScrollDelta, ScrollWheelEvent, TouchPhase,
        };
        use nd_terminal::terminal;
        use nd_ui::prelude::*;

        struct Shell {
            session: TerminalSession,
        }

        impl Render for Shell {
            fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
                div().size_full().child(terminal("term", &self.session).size_full().font_size(13.))
            }
        }

        let (lines, frames) = settings();
        let session = filled(lines);
        let text = nd_gpui::gpui_platform::current_platform(true).text_system();
        let mut cx =
            HeadlessAppContext::with_platform(text, Arc::new(()), nd_gpui::gpui_platform::current_headless_renderer);
        let mut window = None;
        let s = session.clone();
        let host = cx.update(|cx| {
            GpuiHost::builder().quit_when_empty(false).start(cx, |app| {
                let handle = app.open_window(WindowOptions::main("scroll").size(size(px(900.), px(560.))), |_, cx| {
                    cx.new(|_| Shell { session: s })
                });
                window = Some(handle.id());
            })
        });
        for _ in 0..6 {
            cx.run_until_parked();
            cx.update(|cx| {
                host.pump(cx);
                host.draw(cx);
            });
        }
        let handle = host.window_handle(window.expect("window")).expect("GPUI window");
        for step in [3isize, 40] {
            session.scroll_to_bottom();
            let mut work = Vec::with_capacity(frames);
            for _ in 0..frames {
                let started = Instant::now();
                let event = ScrollWheelEvent {
                    position: gpui::point(px(400.), px(300.)),
                    delta: ScrollDelta::Lines(gpui::point(0., step as f32)),
                    modifiers: Modifiers::default(),
                    touch_phase: TouchPhase::Moved,
                };
                cx.update(|cx| {
                    let _ = handle.update(cx, |_, window, cx| {
                        window.dispatch_event(PlatformInput::ScrollWheel(event), cx);
                    });
                    host.pump(cx);
                    host.draw(cx);
                });
                cx.run_until_parked();
                work.push(started.elapsed().as_secs_f64() * 1000.);
            }
            check(report("gpui", step, &work));
        }
    }
}

// One push per presenter this target links.
#[allow(clippy::vec_init_then_push)]
fn main() {
    let mut tests: Vec<(&str, fn())> = Vec::new();
    #[cfg(target_os = "macos")]
    tests.push(("gate_m9_terminal::appkit_scroll_budget", gate_m9_terminal::appkit_scroll_budget));
    #[cfg(any(not(target_os = "macos"), feature = "gpui"))]
    tests.push(("gate_m9_terminal::gpui_scroll_budget", gate_m9_terminal::gpui_scroll_budget));
    support::run(&tests);
}
