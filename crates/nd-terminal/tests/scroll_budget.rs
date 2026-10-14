//! The 100k-line scroll budget: a session holding 100k lines of coloured output, scrolled back
//! through its scrollback a few rows per frame (a trackpad) and many rows per frame (a fling),
//! one frame drawn per step. Timed per frame on the main thread: the scroll, libghostty's
//! snapshot, the presenter's build and, on AppKit, the Metal pass waited for. On the GPUI
//! presenter each step is a wheel event through the window's own input path and one headless
//! draw. Optimized builds fail when the p95 of the main thread's CPU time per frame is over the
//! 60 Hz period; debug builds report. Wall time is reported beside it: on a machine busy with
//! other builds the scheduler, not the frame, decides its tail. `ND_LINES`, `ND_FRAMES` override
//! the size of the run.

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

/// CPU time the calling thread has used, in milliseconds.
#[cfg(unix)]
fn thread_cpu_ms() -> f64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut ts) };
    ts.tv_sec as f64 * 1000. + ts.tv_nsec as f64 / 1e6
}

/// CPU time the calling thread has used, in milliseconds (kernel plus user, 100 ns units).
#[cfg(windows)]
fn thread_cpu_ms() -> f64 {
    use windows::Win32::Foundation::FILETIME;
    use windows::Win32::System::Threading::{GetCurrentThread, GetThreadTimes};
    let (mut created, mut exited, mut kernel, mut user) =
        (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
    let _ = unsafe { GetThreadTimes(GetCurrentThread(), &mut created, &mut exited, &mut kernel, &mut user) };
    let ticks = |t: FILETIME| (u64::from(t.dwHighDateTime) << 32 | u64::from(t.dwLowDateTime)) as f64;
    (ticks(kernel) + ticks(user)) / 10_000.
}

/// Times one frame: wall and main-thread CPU milliseconds.
fn timed(f: impl FnOnce()) -> (f64, f64) {
    let (wall, cpu) = (Instant::now(), thread_cpu_ms());
    f();
    (wall.elapsed().as_secs_f64() * 1000., thread_cpu_ms() - cpu)
}

fn stats(values: impl Iterator<Item = f64>) -> (f64, f64, f64, f64) {
    let mut sorted: Vec<f64> = values.collect();
    sorted.sort_by(f64::total_cmp);
    (quantile(&sorted, 0.5), quantile(&sorted, 0.95), quantile(&sorted, 0.99), sorted.last().copied().unwrap_or(0.))
}

/// Prints the run and returns the CPU p95.
fn report(presenter: &str, step: isize, work: &[(f64, f64)]) -> f64 {
    let (w50, w95, w99, wmax) = stats(work.iter().map(|w| w.0));
    let (c50, c95, c99, cmax) = stats(work.iter().map(|w| w.1));
    let over = work.iter().filter(|w| w.1 > BUDGET_MS).count();
    println!(
        "nd-budget: terminal_scroll presenter={presenter} rows_per_frame={step} frames={} cpu_p50_ms={c50:.2} cpu_p95_ms={c95:.2} cpu_p99_ms={c99:.2} cpu_max_ms={cmax:.2} cpu_over_60hz={over} wall_p50_ms={w50:.2} wall_p95_ms={w95:.2} wall_p99_ms={w99:.2} wall_max_ms={wmax:.2}",
        work.len(),
    );
    c95
}

fn settings() -> (usize, usize) {
    let lines = std::env::var("ND_LINES").ok().and_then(|v| v.parse().ok()).unwrap_or(100_000);
    // Debug builds only report, so they run a short pass.
    let default_frames = if cfg!(debug_assertions) { 60 } else { 600 };
    let frames = std::env::var("ND_FRAMES").ok().and_then(|v| v.parse().ok()).unwrap_or(default_frames);
    (lines, frames)
}

fn check(p95: f64) {
    if !cfg!(debug_assertions) {
        assert!(p95 <= BUDGET_MS, "p95 frame work {p95:.2} ms of CPU is over the 60 Hz period");
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
                work.push(timed(|| {
                    session.scroll_rows(-step);
                    surface.draw(&session);
                }));
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
                let event = ScrollWheelEvent {
                    position: gpui::point(px(400.), px(300.)),
                    delta: ScrollDelta::Lines(gpui::point(0., step as f32)),
                    modifiers: Modifiers::default(),
                    touch_phase: TouchPhase::Moved,
                };
                work.push(timed(|| {
                    cx.update(|cx| {
                        let _ = handle.update(cx, |_, window, cx| {
                            window.dispatch_event(PlatformInput::ScrollWheel(event), cx);
                        });
                        host.pump(cx);
                        host.draw(cx);
                    });
                    cx.run_until_parked();
                }));
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
