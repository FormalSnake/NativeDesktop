//! Row actions on the GPUI presenter (M5): a two-finger swipe left over a list row goes through
//! nd-gesture's arbiter to the row (not the list, which scrolls vertically), the row settles open
//! with its trailing actions drawn underneath, and tapping one runs the app's action.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use nd_backend_gpui::GpuiHost;
use nd_gpui::gpui::{
    HeadlessAppContext, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, PlatformInput, ScrollDelta,
    ScrollWheelEvent, TouchPhase, point,
};
use nd_ui::WindowId;
use nd_ui::prelude::*;
use nd_virtual::{ListState, RowActions, list, row_action};

struct Rows {
    list: ListState<u32>,
    deleted: Rc<Cell<Option<u32>>>,
}

impl Render for Rows {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let deleted = self.deleted.clone();
        list("rows", &self.list, |row: u32, _, _| div().w_full().h(px(44.)).child(format!("Row {row}")))
            .row_actions(move |row: u32, _| {
                let deleted = deleted.clone();
                RowActions::new()
                    .trailing(row_action("Delete").destructive().on_invoke(move |_, _| deleted.set(Some(row))))
            })
            .size_full()
    }
}

fn send(cx: &mut HeadlessAppContext, host: &GpuiHost, window: WindowId, input: PlatformInput) {
    let handle = host.window_handle(window).expect("gpui window");
    cx.update(|cx| {
        let _ = handle.update(cx, |_, window, cx| {
            window.dispatch_event(input, cx);
        });
    });
}

fn frames(cx: &mut HeadlessAppContext, host: &GpuiHost, ms: u64) {
    for _ in 0..ms / 16 {
        cx.run_until_parked();
        cx.update(|cx| {
            host.pump(cx);
            host.draw(cx);
        });
        std::thread::sleep(Duration::from_millis(16));
    }
}

fn run() {
    let text = nd_gpui::gpui_platform::current_platform(true).text_system();
    let mut cx =
        HeadlessAppContext::with_platform(text, Arc::new(()), nd_gpui::gpui_platform::current_headless_renderer);
    let deleted = Rc::new(Cell::new(None));
    let d = deleted.clone();
    let mut window = None;
    let host = cx.update(|cx| {
        GpuiHost::builder().quit_when_empty(false).start(cx, |app| {
            let handle = app.open_window(WindowOptions::main("rows").size(size(px(320.), px(300.))), |_, cx| {
                cx.new(|cx| {
                    let mut list = ListState::new(cx);
                    list.set_items((0..50).collect::<Vec<u32>>(), cx);
                    Rows { list, deleted: d }
                })
            });
            window = Some(handle.id());
        })
    });
    let window = window.expect("window");
    frames(&mut cx, &host, 100);
    // Row 2 sits at y 88..132; swipe left over it.
    let at = point(px(160.), px(110.));
    let wheel = |dx: f32, phase: TouchPhase| {
        PlatformInput::ScrollWheel(ScrollWheelEvent {
            position: at,
            delta: ScrollDelta::Pixels(point(px(dx), px(0.))),
            modifiers: Modifiers::default(),
            touch_phase: phase,
        })
    };
    send(&mut cx, &host, window, wheel(0., TouchPhase::Started));
    for _ in 0..8 {
        send(&mut cx, &host, window, wheel(-14., TouchPhase::Moved));
        std::thread::sleep(Duration::from_millis(8));
    }
    send(&mut cx, &host, window, wheel(0., TouchPhase::Ended));
    frames(&mut cx, &host, 700);
    // The trailing action is drawn at the row's trailing edge: tap it.
    let tap = point(px(320. - 30.), px(110.));
    let down = MouseDownEvent {
        button: MouseButton::Left,
        position: tap,
        modifiers: Modifiers::default(),
        click_count: 1,
        first_mouse: false,
    };
    let up = MouseUpEvent { button: MouseButton::Left, position: tap, modifiers: Modifiers::default(), click_count: 1 };
    send(&mut cx, &host, window, PlatformInput::MouseDown(down));
    send(&mut cx, &host, window, PlatformInput::MouseUp(up));
    frames(&mut cx, &host, 100);
    assert_eq!(deleted.get(), Some(2), "the swiped row's Delete action ran");
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let name = "gate_m5::gpui_row_actions";
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
