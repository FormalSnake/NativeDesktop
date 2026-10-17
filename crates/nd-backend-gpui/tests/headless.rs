//! Tier 2: `nd-ui` apps on the GPUI backend inside GPUI's headless app, with the platform's real
//! text system and its headless renderer (Metal on macOS, wgpu on Linux). The reconciler drives
//! the seam exactly as `nd::Application::run` does; checks read GPUI's laid-out bounds, its input
//! path and rendered pixels back.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use nd_backend_gpui::{GpuiHost, OsHooks};
use nd_gpui::gpui::{self, HeadlessAppContext};
use nd_ui::prelude::*;
use nd_ui::semantics::{MouseButton, PointerPhase, SemanticAction, SyntheticInput};
use nd_ui::{KeyBinding, Keystroke, Role, SemanticNode, TitlebarInsets, WindowId};

gpui::actions!(test, [Bump]);

struct Counter {
    count: u32,
    bumps: Rc<Cell<u32>>,
}

impl Render for Counter {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let bumps = self.bumps.clone();
        v_flex()
            .size_full()
            .p(px(10.))
            .gap_2()
            .key_context("Counter")
            .focusable()
            .on_action(move |_: &Bump, _, _| bumps.set(bumps.get() + 1))
            .child(text(self.count.to_string()).id("count").tabular_nums())
            .child(button("add", "Add one").on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                this.count += 1;
                cx.notify();
            })))
            .child(div().id("box").w(px(120.)).h(px(40.)).rounded(px(8.)).bg(rgb(0x3366ff)))
    }
}

struct TitleRow;

impl Render for TitleRow {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .child(
                drag_region()
                    .id("title")
                    .titlebar_insets()
                    .flex()
                    .flex_row()
                    .px(px(8.))
                    .child(div().id("leading").w(px(20.)).h(px(10.)))
                    .child(div().flex_1())
                    .child(div().id("trailing").w(px(20.)).h(px(10.))),
            )
            .child(window_controls())
    }
}

/// Caption buttons on the trailing side, as `nd-os-windows` reports them.
struct Captions;

impl OsHooks for Captions {
    fn titlebar_insets(&self, _: WindowId, _: &gpui::Window) -> Option<TitlebarInsets> {
        Some(TitlebarInsets { leading: 0., trailing: 138., height: 32. })
    }
}

fn headless() -> HeadlessAppContext {
    let text = nd_gpui::gpui_platform::current_platform(true).text_system();
    HeadlessAppContext::with_platform(text, Arc::new(()), nd_gpui::gpui_platform::current_headless_renderer)
}

fn find<'a>(node: &'a SemanticNode, f: &impl Fn(&SemanticNode) -> bool) -> Option<&'a SemanticNode> {
    if f(node) {
        return Some(node);
    }
    node.children.iter().find_map(|c| find(c, f))
}

fn node(host: &GpuiHost, window: WindowId, f: impl Fn(&SemanticNode) -> bool) -> SemanticNode {
    host.read(|app| {
        let tree = app.semantic_window(window).expect("window");
        find(&tree.root, &f).cloned().expect("node")
    })
}

fn by_test_id(host: &GpuiHost, window: WindowId, id: &str) -> SemanticNode {
    node(host, window, |n| n.test_id.as_deref() == Some(id))
}

/// Pumps and draws until the app settles, the way GPUI's frame loop would.
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

struct Opened {
    cx: HeadlessAppContext,
    host: GpuiHost,
    window: WindowId,
    bumps: Rc<Cell<u32>>,
}

fn open_counter() -> Opened {
    let mut cx = headless();
    let bumps = Rc::new(Cell::new(0));
    let b = bumps.clone();
    let mut window = None;
    let host = cx.update(|cx| {
        GpuiHost::builder().quit_when_empty(false).start(cx, |app| {
            app.bind_keys([KeyBinding::new("ctrl-k", Bump, Some("Counter"))]);
            let handle = app
                .open_window(WindowOptions::main("main").title("Counter").size(size(px(320.), px(240.))), |_, cx| {
                    cx.new(|_| Counter { count: 0, bumps: b })
                });
            window = Some(handle.id());
        })
    });
    settle(&mut cx, &host);
    Opened { cx, host, window: window.expect("window"), bumps }
}

fn count_text(host: &GpuiHost, window: WindowId) -> String {
    by_test_id(host, window, "count").name
}

mod gate_m2 {
    use super::*;

    pub fn gpui_lays_out_the_retained_tree() {
        let Opened { host, window, .. } = open_counter();
        assert!(host.window_handle(window).is_some(), "a GPUI window per app window");
        let root = host.read(|app| app.semantic_window(window).expect("window").root);
        assert_eq!(root.role, Role::Window);
        let button = node(&host, window, |n| n.role == Role::Button);
        assert_eq!(button.name, "Add one");
        let b = button.bounds.expect("button laid out by GPUI");
        assert!(b.size.width > 40. && b.size.height > 10., "{b:?}");
        let boxed = by_test_id(&host, window, "box").bounds.expect("box");
        assert_eq!((boxed.size.width, boxed.size.height), (120., 40.));
        // Padding 10, gap 8: the box sits below the text and the button.
        assert_eq!(boxed.origin.x, 10.);
        assert!(boxed.origin.y > b.origin.y + b.size.height);
        assert_eq!(count_text(&host, window), "0");
    }

    pub fn a_semantic_click_reaches_the_handler() {
        let Opened { mut cx, host, window, .. } = open_counter();
        let button = node(&host, window, |n| n.role == Role::Button).id;
        cx.update(|cx| host.update(cx, |app| app.perform(button, &SemanticAction::Click))).expect("click");
        settle(&mut cx, &host);
        assert_eq!(count_text(&host, window), "1");
    }

    pub fn a_pointer_click_goes_through_the_window_input_path() {
        let Opened { mut cx, host, window, .. } = open_counter();
        let b = node(&host, window, |n| n.role == Role::Button).bounds.expect("bounds");
        let at = nd_ui::Point { x: b.origin.x + b.size.width / 2., y: b.origin.y + b.size.height / 2. };
        cx.update(|cx| {
            host.update(cx, |app| {
                for phase in [PointerPhase::Down, PointerPhase::Up] {
                    let input = SyntheticInput::Pointer { phase, at, button: MouseButton::Left, click_count: 1 };
                    app.synthesize(window, &input).expect("posted");
                }
            })
        });
        settle(&mut cx, &host);
        assert_eq!(count_text(&host, window), "1");
        // Missing the button clicks nothing.
        cx.update(|cx| {
            host.update(cx, |app| {
                let at = nd_ui::Point { x: 300., y: 230. };
                for phase in [PointerPhase::Down, PointerPhase::Up] {
                    let input = SyntheticInput::Pointer { phase, at, button: MouseButton::Left, click_count: 1 };
                    app.synthesize(window, &input).expect("posted");
                }
            })
        });
        settle(&mut cx, &host);
        assert_eq!(count_text(&host, window), "1");
    }

    pub fn keys_reach_nd_dispatch_before_gpui() {
        let Opened { mut cx, host, window, bumps } = open_counter();
        let root = node(&host, window, |n| n.role == Role::Group).id;
        cx.update(|cx| host.update(cx, |app| app.focus(window, root)));
        let key = Keystroke::parse("ctrl-k").expect("keystroke");
        cx.update(|cx| host.update(cx, |app| app.synthesize(window, &SyntheticInput::Key(key)))).expect("posted");
        settle(&mut cx, &host);
        assert_eq!(bumps.get(), 1, "the nd binding fired through GPUI's keystroke path");
    }

    pub fn windows_open_and_close_with_the_app() {
        let Opened { mut cx, host, window, .. } = open_counter();
        let second = cx.update(|cx| {
            host.update(cx, |app| {
                app.open_window(WindowOptions::main("second").size(size(px(200.), px(100.))), |_, cx| {
                    cx.new(|_| TitleRow)
                })
                .id()
            })
        });
        settle(&mut cx, &host);
        let (a, b) = (host.window_handle(window).expect("first"), host.window_handle(second).expect("second"));
        assert_ne!(a.window_id(), b.window_id());
        assert_eq!(cx.update(|cx| cx.windows().len()), 2);
        assert!(by_test_id(&host, second, "trailing").bounds.is_some(), "the second window draws its own tree");
        cx.update(|cx| host.update(cx, |app| app.close_window(second)));
        settle(&mut cx, &host);
        assert!(host.window_handle(second).is_none());
        assert_eq!(cx.update(|cx| cx.windows().len()), 1);
        assert_eq!(count_text(&host, window), "0", "the other window is untouched");
    }

    pub fn titlebar_insets_come_from_the_os_hooks() {
        let mut cx = headless();
        let mut window = None;
        let host = cx.update(|cx| {
            GpuiHost::builder().quit_when_empty(false).hooks(Captions).start(cx, |app| {
                let handle = app.open_window(WindowOptions::main("main").size(size(px(400.), px(200.))), |_, cx| {
                    cx.new(|_| TitleRow)
                });
                window = Some(handle.id());
            })
        });
        settle(&mut cx, &host);
        let window = window.expect("window");
        let title = by_test_id(&host, window, "title").bounds.expect("title");
        let trailing = by_test_id(&host, window, "trailing").bounds.expect("trailing");
        assert_eq!(title.size.height, 32.);
        assert_eq!(trailing.origin.x, 400. - 8. - 138. - 20.);
        assert_eq!(by_test_id(&host, window, "leading").bounds.expect("leading").origin.x, 8.);
        let insets = host.read(|app| app.window(window).expect("window").titlebar_insets());
        assert_eq!(insets.trailing, 138.);
        // The test platform decorates on the server side: the controls draw nothing.
        let controls = node(&host, window, |n| n.role == Role::WindowControls);
        assert!(controls.bounds.is_none_or(|b| b.size.width == 0.), "{:?}", controls.bounds);
    }

    pub fn capture_renders_the_window() {
        // GPUI renders headless only on macOS (Metal) and Linux (wgpu); Windows captures arrive
        // with the Fluent goldens (M3).
        if nd_gpui::gpui_platform::current_headless_renderer().ok().flatten().is_none() {
            println!("skipped: GPUI has no headless renderer on this OS");
            return;
        }
        // The headless app has to outlive the capture.
        let Opened { cx: _cx, host, window, .. } = open_counter();
        let capture = host.job(|app| app.capture(window)).expect("capture");
        assert!(capture.width >= 320 && capture.height >= 240, "{}x{}", capture.width, capture.height);
        let image = image::load_from_memory(&capture.png).expect("png").to_rgba8();
        let scale = capture.width as f32 / 320.;
        let boxed = by_test_id(&host, window, "box").bounds.expect("box");
        let x = ((boxed.origin.x + boxed.size.width / 2.) * scale) as u32;
        let y = ((boxed.origin.y + boxed.size.height / 2.) * scale) as u32;
        let p = image.get_pixel(x, y).0;
        assert!(p[2] > 200 && p[0] < 100, "the blue box is painted where GPUI laid it out: {p:?}");
        let corner = image.get_pixel(2, (capture.height - 3).min(image.height() - 1)).0;
        assert!(corner[0] > 200 && corner[1] > 200, "light background: {corner:?}");
    }

    pub fn the_backend_releases_what_the_reconciler_drops() {
        let Opened { mut cx, host, window, .. } = open_counter();
        let live = host.backend().live_count();
        assert!(live >= 5, "root, column, text, button, box: {live}");
        cx.update(|cx| host.update(cx, |app| app.close_window(window)));
        settle(&mut cx, &host);
        assert_eq!(host.backend().live_count(), 0);
    }
}

mod gate_m4 {
    use std::cell::RefCell;
    use std::time::Duration;

    use nd_gpui::gpui::{PlatformInput, ScrollDelta, ScrollWheelEvent, TouchPhase, point};

    use super::*;

    struct Feed {
        log: Rc<RefCell<Vec<String>>>,
    }

    impl Render for Feed {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            let (pinch, drag) = (self.log.clone(), self.log.clone());
            v_flex()
                .size_full()
                .on_pinch(move |e: &PinchEvent, _, _| {
                    pinch.borrow_mut().push(format!("pinch {:?} {:.2}", e.phase, e.scale))
                })
                .child(
                    div()
                        .id("handle")
                        .h(px(20.))
                        .w(px(100.))
                        .on_drag(move |e: &DragEvent, _, _| drag.borrow_mut().push(format!("drag {:?}", e.phase))),
                )
                .child(v_flex().id("list").h(px(200.)).overflow_y_scroll().children(
                    (0..40).map(|i| div().id(format!("row-{i}")).h(px(30.)).flex_none().child(format!("Row {i}"))),
                ))
        }
    }

    fn open_feed() -> (HeadlessAppContext, GpuiHost, WindowId, Rc<RefCell<Vec<String>>>) {
        let mut cx = headless();
        let log = Rc::new(RefCell::new(Vec::new()));
        let l = log.clone();
        let mut window = None;
        let host = cx.update(|cx| {
            GpuiHost::builder().quit_when_empty(false).start(cx, |app| {
                let options = WindowOptions::main("feed").title("Feed").size(size(px(320.), px(260.)));
                window = Some(app.open_window(options, |_, cx| cx.new(|_| Feed { log: l })).id());
            })
        });
        settle(&mut cx, &host);
        (cx, host, window.expect("window"), log)
    }

    fn send(cx: &mut HeadlessAppContext, host: &GpuiHost, window: WindowId, input: PlatformInput) {
        let handle = host.window_handle(window).expect("gpui window");
        cx.update(|cx| {
            let _ = handle.update(cx, |_, window, cx| {
                window.dispatch_event(input, cx);
            });
        });
    }

    fn scroll(cx: &mut HeadlessAppContext, host: &GpuiHost, window: WindowId, dy: f32, phase: TouchPhase) {
        let event = ScrollWheelEvent {
            position: point(px(100.), px(100.)),
            delta: ScrollDelta::Pixels(point(px(0.), px(dy))),
            modifiers: Default::default(),
            touch_phase: phase,
        };
        send(cx, host, window, PlatformInput::ScrollWheel(event));
        std::thread::sleep(Duration::from_millis(8));
    }

    /// Draws frames in real time for `ms`, as the display link would while something animates.
    fn frames(cx: &mut HeadlessAppContext, host: &GpuiHost, ms: u64) {
        for _ in 0..ms / 16 {
            std::thread::sleep(Duration::from_millis(16));
            cx.update(|cx| {
                host.pump(cx);
                host.draw(cx);
            });
            cx.run_until_parked();
        }
    }

    fn row_top(host: &GpuiHost, window: WindowId) -> f32 {
        by_test_id(host, window, "row-0").bounds.expect("row bounds").origin.y
    }

    pub fn a_fling_carries_on_and_a_pull_springs_back() {
        let (mut cx, host, window, _) = open_feed();
        let top = by_test_id(&host, window, "list").bounds.expect("list").origin.y;
        assert_eq!(row_top(&host, window), top);

        // Fingers up 200 points quickly, then lift: momentum carries the list further.
        scroll(&mut cx, &host, window, 0., TouchPhase::Started);
        for _ in 0..10 {
            scroll(&mut cx, &host, window, -20., TouchPhase::Moved);
        }
        // Lifted straight away: a pause first would read as a stop, with no fling.
        scroll(&mut cx, &host, window, 0., TouchPhase::Ended);
        let at_lift = top - 200.;
        frames(&mut cx, &host, 2500);
        let rested = row_top(&host, window);
        assert!(rested < at_lift - 100., "the fling travelled on ({at_lift} -> {rested})");
        assert!(rested >= top - (40. * 30. - 200.) - 0.5, "and stopped at the end at most ({rested}, top {top})");

        // Back to the top and past it. macOS stretches the list, then springs it back to the
        // edge; Linux and Windows have no rubber band and stop at the edge.
        scroll(&mut cx, &host, window, 0., TouchPhase::Started);
        for _ in 0..80 {
            scroll(&mut cx, &host, window, 25., TouchPhase::Moved);
        }
        frames(&mut cx, &host, 16);
        let stretched = row_top(&host, window);
        if cfg!(target_os = "macos") {
            assert!(stretched > top, "pulled past the top ({stretched} > {top})");
            assert!(stretched < top + 200., "but resisted ({stretched})");
        } else {
            assert!((stretched - top).abs() < 0.5, "held at the top with no overshoot ({stretched}, top {top})");
        }
        std::thread::sleep(Duration::from_millis(80));
        scroll(&mut cx, &host, window, 0., TouchPhase::Ended);
        frames(&mut cx, &host, 1200);
        assert!((row_top(&host, window) - top).abs() < 0.5, "rests at the top");
    }

    pub fn pinch_and_drag_reach_their_handlers() {
        let (mut cx, host, window, log) = open_feed();
        for (phase, delta) in [(TouchPhase::Started, 0.), (TouchPhase::Moved, 0.1), (TouchPhase::Ended, 0.)] {
            let event =
                gpui::PinchEvent { position: point(px(150.), px(150.)), delta, modifiers: Default::default(), phase };
            send(&mut cx, &host, window, PlatformInput::Pinch(event));
        }
        let handle = by_test_id(&host, window, "handle").bounds.expect("handle");
        let from = nd_ui::Point { x: handle.origin.x + 10., y: handle.origin.y + 10. };
        let to = nd_ui::Point { x: from.x + 60., y: from.y };
        cx.update(|cx| host.update(cx, |app| app.synthesize(window, &SyntheticInput::Drag { from, to, steps: 6 })))
            .expect("posted");
        settle(&mut cx, &host);
        let log = log.borrow().clone();
        assert_eq!(&log[..3], ["pinch Began 1.00", "pinch Changed 1.10", "pinch Ended 1.00"], "{log:?}");
        assert_eq!(log.first_drag(), Some("drag Began"), "{log:?}");
        assert_eq!(log.last().map(String::as_str), Some("drag Ended"), "{log:?}");
    }

    trait FirstDrag {
        fn first_drag(&self) -> Option<&str>;
    }

    impl FirstDrag for Vec<String> {
        fn first_drag(&self) -> Option<&str> {
            self.iter().find(|l| l.starts_with("drag")).map(String::as_str)
        }
    }
}

fn main() {
    // libtest-style listing and filtering for nextest, one test per process.
    let args: Vec<String> = std::env::args().skip(1).collect();
    let tests: Vec<(&str, fn())> = vec![
        ("gate_m2::gpui_lays_out_the_retained_tree", gate_m2::gpui_lays_out_the_retained_tree),
        ("gate_m2::a_semantic_click_reaches_the_handler", gate_m2::a_semantic_click_reaches_the_handler),
        (
            "gate_m2::a_pointer_click_goes_through_the_window_input_path",
            gate_m2::a_pointer_click_goes_through_the_window_input_path,
        ),
        ("gate_m2::keys_reach_nd_dispatch_before_gpui", gate_m2::keys_reach_nd_dispatch_before_gpui),
        ("gate_m2::windows_open_and_close_with_the_app", gate_m2::windows_open_and_close_with_the_app),
        ("gate_m2::titlebar_insets_come_from_the_os_hooks", gate_m2::titlebar_insets_come_from_the_os_hooks),
        ("gate_m2::capture_renders_the_window", gate_m2::capture_renders_the_window),
        (
            "gate_m2::the_backend_releases_what_the_reconciler_drops",
            gate_m2::the_backend_releases_what_the_reconciler_drops,
        ),
        ("gate_m4::a_fling_carries_on_and_a_pull_springs_back", gate_m4::a_fling_carries_on_and_a_pull_springs_back),
        ("gate_m4::pinch_and_drag_reach_their_handlers", gate_m4::pinch_and_drag_reach_their_handlers),
    ];
    harness(&args, &tests);
}

/// The libtest command line, enough for `cargo test` and nextest. GPUI's headless app holds
/// thread-local state, so every test runs on the main thread of its own process when nextest
/// drives it, and one after another under `cargo test`.
fn harness(args: &[String], tests: &[(&str, fn())]) {
    let has = |flag: &str| args.iter().any(|a| a == flag);
    if has("--list") {
        if !has("--ignored") {
            for (name, _) in tests {
                println!("{name}: test");
            }
        }
        return;
    }
    if has("--ignored") {
        return;
    }
    let exact = has("--exact");
    let mut skip = false;
    let filters: Vec<&String> = args
        .iter()
        .filter(|a| {
            if std::mem::take(&mut skip) {
                return false;
            }
            if matches!(a.as_str(), "--format" | "--test-threads" | "--color" | "--skip" | "-Z") {
                skip = true;
            }
            !a.starts_with('-')
        })
        .collect();
    let mut failed = 0;
    for (name, test) in tests {
        let selected =
            filters.is_empty() || filters.iter().any(|f| if exact { *f == name } else { name.contains(f.as_str()) });
        if !selected {
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
