//! The 100k-row scroll budget (ARCHITECTURE.md 9.5) without a display: the scroll-bench list in
//! GPUI's headless app, one wheel event through the window's input path and one draw per frame,
//! timed on the main thread. It measures the CPU side of each frame (input, the app frame, GPUI
//! layout, prepaint and paint), the part that scales with a weak CPU; presentation is measured
//! by `nd diag scroll` in a real window. Optimized builds fail over budget; debug builds report.

use std::sync::Arc;
use std::time::Instant;

use nd_backend_gpui::GpuiHost;
use nd_gpui::gpui::{self, HeadlessAppContext, Modifiers, PlatformInput, ScrollDelta, ScrollWheelEvent, TouchPhase};
use nd_ui::prelude::*;
use nd_virtual::{ListState, list};

struct Rows {
    list: ListState<u32>,
}

fn height(row: u32) -> f32 {
    28. + (row.wrapping_mul(2_654_435_761) >> 7) as f32 % 97.
}

impl Render for Rows {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        list("rows", &self.list, |row: u32, _, _| {
            h_flex()
                .px(px(12.))
                .py(px(4.))
                .gap_2()
                .child(div().w(px(6.)).h(px(height(row) - 8.)).bg(hsla((row % 360) as f32 / 360., 0.6, 0.5, 1.)))
                .child(text(format!("Row {row}")).tabular_nums())
        })
        .size_full()
    }
}

fn quantile(sorted: &[f64], q: f64) -> f64 {
    sorted.get(((sorted.len() as f64 - 1.) * q).round() as usize).copied().unwrap_or(0.)
}

fn run() {
    let rows: u32 = std::env::var("ND_ROWS").ok().and_then(|v| v.parse().ok()).unwrap_or(100_000);
    let frames: usize = std::env::var("ND_FRAMES").ok().and_then(|v| v.parse().ok()).unwrap_or(600);
    let step: f32 = std::env::var("ND_SCROLL_STEP").ok().and_then(|v| v.parse().ok()).unwrap_or(40.);
    let text = nd_gpui::gpui_platform::current_platform(true).text_system();
    let mut cx =
        HeadlessAppContext::with_platform(text, Arc::new(()), nd_gpui::gpui_platform::current_headless_renderer);
    let mut window = None;
    let host = cx.update(|cx| {
        GpuiHost::builder().quit_when_empty(false).start(cx, |app| {
            let handle = app.open_window(WindowOptions::main("scroll").size(size(px(480.), px(720.))), |_, cx| {
                cx.new(|cx| {
                    let mut list = ListState::new(cx).estimated_row_height(px(60.));
                    list.set_items_versioned((0..rows).map(|r| (r, 0)), cx);
                    Rows { list }
                })
            });
            window = Some(handle.id());
        })
    });
    let window = window.expect("window");
    for _ in 0..6 {
        cx.run_until_parked();
        cx.update(|cx| {
            host.pump(cx);
            host.draw(cx);
        });
    }
    let handle = host.window_handle(window).expect("gpui window");
    let mut work = Vec::with_capacity(frames);
    for _ in 0..frames {
        let started = Instant::now();
        let event = ScrollWheelEvent {
            position: gpui::point(px(200.), px(300.)),
            delta: ScrollDelta::Pixels(gpui::point(px(0.), px(-step))),
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
    let mut sorted = work.clone();
    sorted.sort_by(f64::total_cmp);
    let budget = 1000. / 60.;
    let over = work.iter().filter(|w| **w > budget).count();
    println!(
        "nd-budget: scroll_work rows={rows} frames={frames} step={step} p50_ms={:.2} p95_ms={:.2} p99_ms={:.2} max_ms={:.2} over_60hz={over}",
        quantile(&sorted, 0.5),
        quantile(&sorted, 0.95),
        quantile(&sorted, 0.99),
        sorted.last().copied().unwrap_or(0.),
    );
    if !cfg!(debug_assertions) {
        assert!(quantile(&sorted, 0.95) <= budget, "p95 frame work over the 60 Hz period");
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let name = "gate_m5::gpui_scroll_budget";
    if args.iter().any(|a| a == "--list") {
        if !args.iter().any(|a| a == "--ignored") {
            println!("{name}: test");
        }
        return;
    }
    if args.iter().any(|a| a == "--ignored") {
        return;
    }
    let filters: Vec<&String> = args.iter().filter(|a| !a.starts_with('-')).collect();
    if !filters.is_empty() && !filters.iter().any(|f| name.contains(f.as_str())) {
        return;
    }
    run();
    println!("test {name} ... ok");
}
