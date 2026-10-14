//! Tier 2: the single canvas the web draws into (ARCHITECTURE.md 12), run natively in GPUI's
//! headless app with `HostBuilder::single_canvas(true)`. Every app window shares one GPUI
//! window; a later window floats above the first and takes presses over it, so nothing leaks
//! through to the window below; the menu bar takes its row off the top.

use std::sync::Arc;

use nd_backend_gpui::GpuiHost;
use nd_gpui::gpui::{self, HeadlessAppContext};
use nd_ui::prelude::*;
use nd_ui::semantics::{MouseButton, PointerPhase, SyntheticInput};
use nd_ui::{SemanticNode, TapEvent, WindowId};

gpui::actions!(canvas_test, [Noop]);

/// A window that is one tap target counting its taps. Taps go through the gesture router of the
/// window GPUI delivered the press to, so a press that leaks shows up in the window below.
struct Pad {
    taps: std::rc::Rc<std::cell::Cell<u32>>,
}

impl Render for Pad {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let taps = self.taps.clone();
        v_flex()
            .id("pad")
            .size_full()
            .on_tap(move |_: &TapEvent, _, _| taps.set(taps.get() + 1))
            .child(text(format!("taps {}", self.taps.get())))
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

fn pad(host: &GpuiHost, window: WindowId) -> SemanticNode {
    host.read(|app| {
        let tree = app.semantic_window(window).expect("window").root;
        tree.iter().find(|n| n.test_id.as_deref() == Some("pad")).cloned().expect("pad")
    })
}

fn click_at(cx: &mut HeadlessAppContext, host: &GpuiHost, window: WindowId, node: &SemanticNode) {
    let b = node.bounds.expect("laid out");
    let at = nd_ui::Point { x: b.origin.x + b.size.width / 2., y: b.origin.y + b.size.height / 2. };
    cx.update(|cx| {
        host.update(cx, |app| {
            for phase in [PointerPhase::Down, PointerPhase::Up] {
                let input = SyntheticInput::Pointer { phase, at, button: MouseButton::Left, click_count: 1 };
                app.synthesize(window, &input).expect("posted");
            }
        })
    });
    settle(cx, host);
}

struct Two {
    cx: HeadlessAppContext,
    host: GpuiHost,
    base: WindowId,
    top: WindowId,
    base_taps: std::rc::Rc<std::cell::Cell<u32>>,
    top_taps: std::rc::Rc<std::cell::Cell<u32>>,
}

fn open_two() -> Two {
    let mut cx = headless();
    let (mut base, mut top) = (None, None);
    let (base_taps, top_taps) = (std::rc::Rc::default(), std::rc::Rc::<std::cell::Cell<u32>>::default());
    let (b, t) = (std::rc::Rc::clone(&base_taps), std::rc::Rc::clone(&top_taps));
    let host = cx.update(|cx| {
        GpuiHost::builder().single_canvas(true).quit_when_empty(false).start(cx, |app| {
            let options = WindowOptions::main("base").title("Base").size(size(px(640.), px(480.)));
            base = Some(app.open_window(options, |_, cx| cx.new(|_| Pad { taps: b })).id());
            let options = WindowOptions::main("top").title("Top").size(size(px(240.), px(160.)));
            top = Some(app.open_window(options, |_, cx| cx.new(|_| Pad { taps: t })).id());
        })
    });
    settle(&mut cx, &host);
    Two { cx, host, base: base.expect("base"), top: top.expect("top"), base_taps, top_taps }
}

mod gate_m11 {
    use super::*;

    pub fn app_windows_share_one_gpui_window() {
        let Two { host, base, top, .. } = open_two();
        let handle = host.window_handle(base).expect("canvas");
        assert_eq!(host.window_handle(top), Some(handle), "one canvas for both");
        let (b, t) = (pad(&host, base).bounds.expect("base"), pad(&host, top).bounds.expect("top"));
        assert!(t.size.width <= 240. && t.size.height <= 160., "the top window keeps its size: {t:?}");
        assert!(b.size.width > t.size.width, "the base window fills the canvas: {b:?}");
    }

    pub fn a_press_on_the_top_window_does_not_reach_the_one_below() {
        let Two { mut cx, host, base, top, base_taps, top_taps } = open_two();
        let top_pad = pad(&host, top);
        click_at(&mut cx, &host, top, &top_pad);
        assert_eq!(top_taps.get(), 1);
        assert_eq!(base_taps.get(), 0, "the press leaked into the base window");
        // Outside the top window the base window takes the press.
        let mut corner = pad(&host, base);
        if let Some(b) = corner.bounds.as_mut() {
            b.size = nd_ui::Size { width: 20., height: 20. };
        }
        click_at(&mut cx, &host, base, &corner);
        assert_eq!(base_taps.get(), 1);
        assert_eq!(top_taps.get(), 1);
    }

    pub fn closing_the_top_window_keeps_the_canvas() {
        let Two { mut cx, host, base, top, base_taps, .. } = open_two();
        cx.update(|cx| host.update(cx, |app| app.close_window(top)));
        settle(&mut cx, &host);
        assert!(host.window_handle(top).is_none());
        assert!(host.window_handle(base).is_some());
        let base_pad = pad(&host, base);
        click_at(&mut cx, &host, base, &base_pad);
        assert_eq!(base_taps.get(), 1);
    }

    pub fn the_menu_bar_takes_a_row_off_the_top() {
        let Two { mut cx, host, base, .. } = open_two();
        let before = pad(&host, base).bounds.expect("base");
        host.backend().set_menu_bar(vec![
            nd_components::Menu::new("View").items([nd_components::MenuItem::action("Nothing", Noop)]),
        ]);
        settle(&mut cx, &host);
        let after = pad(&host, base).bounds.expect("base");
        assert_eq!(after.origin.y - before.origin.y, 28., "{before:?} -> {after:?}");
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let tests: Vec<(&str, fn())> = vec![
        ("gate_m11::app_windows_share_one_gpui_window", gate_m11::app_windows_share_one_gpui_window),
        (
            "gate_m11::a_press_on_the_top_window_does_not_reach_the_one_below",
            gate_m11::a_press_on_the_top_window_does_not_reach_the_one_below,
        ),
        ("gate_m11::closing_the_top_window_keeps_the_canvas", gate_m11::closing_the_top_window_keeps_the_canvas),
        ("gate_m11::the_menu_bar_takes_a_row_off_the_top", gate_m11::the_menu_bar_takes_a_row_off_the_top),
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
