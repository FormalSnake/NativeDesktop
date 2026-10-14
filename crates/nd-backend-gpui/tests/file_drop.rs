//! Tier 2: files dropped on an app window reach `on_file_drop` with that window's id, in GPUI's
//! headless app. The drop is the `FileDropEvent` sequence a platform window delivers, dispatched
//! into the second of two windows; on a single canvas, into the floating window on top.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use nd_backend_gpui::GpuiHost;
use nd_gpui::gpui::{ExternalPaths, FileDropEvent, HeadlessAppContext, PlatformInput, point};
use nd_ui::WindowId;
use nd_ui::prelude::*;

struct Empty;

impl Render for Empty {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full()
    }
}

fn headless() -> HeadlessAppContext {
    let text = nd_gpui::gpui_platform::current_platform(true).text_system();
    HeadlessAppContext::with_platform(text, Arc::new(()), nd_gpui::gpui_platform::current_headless_renderer)
}

fn settle(cx: &mut HeadlessAppContext, host: &GpuiHost) {
    for _ in 0..4 {
        cx.run_until_parked();
        cx.update(|cx| {
            host.pump(cx);
            host.draw(cx);
        });
    }
    cx.run_until_parked();
}

type Drops = Rc<RefCell<Vec<(WindowId, Vec<PathBuf>)>>>;

/// Opens a large and a small window and listens for drops.
fn open(single_canvas: bool) -> (HeadlessAppContext, GpuiHost, [WindowId; 2], Drops) {
    let mut cx = headless();
    let mut ids = Vec::new();
    let host = cx.update(|cx| {
        GpuiHost::builder().single_canvas(single_canvas).quit_when_empty(false).start(cx, |app| {
            for (title, w, h) in [("First", 640., 480.), ("Second", 240., 160.)] {
                let options = WindowOptions::main(title).title(title).size(size(px(w), px(h)));
                ids.push(app.open_window(options, |_, cx| cx.new(|_| Empty)).id());
            }
        })
    });
    let drops: Drops = Rc::default();
    let log = drops.clone();
    host.backend().on_file_drop(move |window, paths, _at| log.borrow_mut().push((window, paths)));
    settle(&mut cx, &host);
    (cx, host, [ids[0], ids[1]], drops)
}

fn drop_files(cx: &mut HeadlessAppContext, host: &GpuiHost, into: WindowId, at: (f32, f32), paths: &[PathBuf]) {
    let handle = host.window_handle(into).expect("window");
    let position = point(px(at.0), px(at.1));
    let events = [
        PlatformInput::FileDrop(FileDropEvent::Entered {
            position,
            paths: ExternalPaths(paths.iter().cloned().collect()),
        }),
        PlatformInput::FileDrop(FileDropEvent::Submit { position }),
    ];
    cx.update(|cx| {
        handle
            .update(cx, |_, window, cx| {
                for event in events {
                    window.dispatch_event(event, cx);
                }
            })
            .expect("window alive");
    });
    settle(cx, host);
}

fn separate_windows() {
    let (mut cx, host, [first, second], drops) = open(false);
    let paths = vec![PathBuf::from("/tmp/a.txt"), PathBuf::from("/tmp/b b.png")];
    drop_files(&mut cx, &host, second, (20., 20.), &paths);
    assert_eq!(*drops.borrow(), [(second, paths.clone())]);
    drop_files(&mut cx, &host, first, (100., 100.), &paths[..1]);
    assert_eq!(drops.borrow()[1], (first, paths[..1].to_vec()));
}

fn single_canvas() {
    let (mut cx, host, [first, second], drops) = open(true);
    let path = vec![PathBuf::from("/tmp/c.txt")];
    // The canvas centres the floating window, so its middle is the canvas's middle.
    let canvas = host.window_handle(first).expect("canvas");
    let middle = cx.update(|cx| {
        canvas
            .update(cx, |_, window, _| {
                let s = window.viewport_size();
                (s.width.as_f32() / 2., s.height.as_f32() / 2.)
            })
            .expect("canvas alive")
    });
    drop_files(&mut cx, &host, first, middle, &path);
    drop_files(&mut cx, &host, first, (4., 4.), &path);
    assert_eq!(*drops.borrow(), [(second, path.clone()), (first, path)], "the window under the drop takes it");
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let tests: Vec<(&str, fn())> = vec![
        ("gate_m7::gpui_file_drop_separate_windows", separate_windows),
        ("gate_m7::gpui_file_drop_single_canvas", single_canvas),
    ];
    let has = |flag: &str| args.iter().any(|a| a == flag);
    if has("--list") {
        if !has("--ignored") {
            for (name, _) in &tests {
                println!("{name}: test");
            }
        }
        return;
    }
    if has("--ignored") {
        return;
    }
    let filters: Vec<&String> = args.iter().filter(|a| !a.starts_with('-')).collect();
    let mut failed = 0;
    for (name, test) in &tests {
        if !filters.is_empty() && !filters.iter().any(|f| name.contains(f.as_str())) {
            continue;
        }
        match std::panic::catch_unwind(*test) {
            Ok(()) => println!("test {name} ... ok"),
            Err(_) => {
                println!("test {name} ... FAILED");
                failed += 1;
            }
        }
    }
    if failed > 0 {
        std::process::exit(101);
    }
}
