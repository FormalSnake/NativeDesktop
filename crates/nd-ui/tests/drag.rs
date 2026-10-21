//! Drag and drop, tier 1: a drag from a `draggable` div finds the innermost target taking its
//! kind in whichever window is under the pointer, reports entry, moves and exit to targets and
//! the outcome to the source; Escape and a release outside end it; widget sources drive the
//! same session; `.on_resize` reports laid-out sizes.

use std::cell::RefCell;
use std::rc::Rc;

use nd_gpui::gpui::{SharedString, px};
use nd_ui::test_app::TestApp;
use nd_ui::*;

type Log = Rc<RefCell<Vec<String>>>;

struct Side {
    log: Log,
    name: &'static str,
    source: bool,
}

impl Render for Side {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let (over, drop, end, inner, size) =
            (self.log.clone(), self.log.clone(), self.log.clone(), self.log.clone(), self.log.clone());
        let name = self.name;
        v_flex()
            .size_full()
            .child(div().id("spacer").h(px(40.)).w_full().when(self.source, |d| {
                d.child(
                    div()
                        .id("chip")
                        .w(px(40.))
                        .h(px(40.))
                        .draggable(DragItem::new("tab", "t1"))
                        .on_drag_end(move |e: &DragEnded, _, _| end.borrow_mut().push(format!("end {:?}", e.outcome))),
                )
            }))
            .child(
                div()
                    .id(SharedString::from(format!("{name}-list")))
                    .w_full()
                    .h(px(200.))
                    .accepts_drops(["tab"])
                    .on_drag_over(move |e: &DragOver, _, _| {
                        over.borrow_mut().push(format!("{name} {:?} {},{}", e.phase, e.local.x, e.local.y))
                    })
                    .on_drop(move |e: &Dropped, _, _| {
                        drop.borrow_mut().push(format!("{name} drop {} {},{}", e.item.id, e.local.x, e.local.y))
                    })
                    .child(
                        div()
                            .id(SharedString::from(format!("{name}-tiles")))
                            .w(px(100.))
                            .h(px(50.))
                            .accepts_drops(["tile"])
                            .on_drop(move |_: &Dropped, _, _| inner.borrow_mut().push(format!("{name} tiles drop"))),
                    ),
            )
            .child(
                div()
                    .id(SharedString::from(format!("{name}-meter")))
                    .w_full()
                    .h(px(10.))
                    .on_resize(move |e: &Resized, _, _| size.borrow_mut().push(format!("{name} size {}", e.width))),
            )
    }
}

fn node(cx: &TestApp, test_id: &str) -> NodeId {
    let tree = cx.semantic_tree();
    tree.nodes().find(|(_, n)| n.test_id.as_deref() == Some(test_id)).map(|(_, n)| n.id).expect(test_id)
}

const SIZE: Size = Size { width: 400., height: 300. };

/// Two windows side by side, 100 points apart: `a` holds the chip.
fn open() -> (TestApp, Log, WindowId, WindowId) {
    let log: Log = Rc::default();
    let mut cx = TestApp::new();
    let (la, lb) = (log.clone(), log.clone());
    let a = cx.open_window(SIZE, move |_, cx| cx.new(|_| Side { log: la, name: "a", source: true })).id();
    let b = cx.open_window(SIZE, move |_, cx| cx.new(|_| Side { log: lb, name: "b", source: false })).id();
    cx.backend_mut().set_window_frame(a, Rect::new(0., 0., 400., 300.));
    cx.backend_mut().set_window_frame(b, Rect::new(500., 0., 400., 300.));
    log.borrow_mut().clear();
    (cx, log, a, b)
}

fn drag(cx: &mut TestApp, chip: NodeId, phase: GesturePhase, x: f32, y: f32) {
    let p = Point { x, y };
    cx.emit(chip, DragEvent { phase, start: Point { x: 20., y: 20. }, position: p, translation: p, velocity: p });
}

fn take(log: &Log) -> Vec<String> {
    std::mem::take(&mut *log.borrow_mut())
}

#[test]
fn a_drag_crosses_windows_to_the_target_under_the_pointer() {
    let (mut cx, log, _, b) = open();
    let chip = node(&cx, "chip");
    drag(&mut cx, chip, GesturePhase::Began, 20., 20.);
    assert!(take(&log).is_empty(), "nothing under the chip takes tabs");
    drag(&mut cx, chip, GesturePhase::Changed, 200., 140.);
    assert_eq!(take(&log), ["a Entered 200,100"]);
    drag(&mut cx, chip, GesturePhase::Changed, 210., 150.);
    assert_eq!(take(&log), ["a Moved 210,110"]);
    // Over the inner target, which takes tiles only: the list keeps the drag.
    drag(&mut cx, chip, GesturePhase::Changed, 30., 60.);
    assert_eq!(take(&log), ["a Moved 30,20"]);
    // 550 in a's coordinates is 50 into b.
    drag(&mut cx, chip, GesturePhase::Changed, 550., 60.);
    assert_eq!(take(&log), ["a Exited 0,0", "b Entered 50,20"]);
    drag(&mut cx, chip, GesturePhase::Ended, 560., 60.);
    assert_eq!(take(&log), ["b Moved 60,20", "b drop t1 60,20", format!("end Dropped {{ window: {b:?} }}").as_str()]);
    assert!(!cx.is_dragging());
}

#[test]
fn escape_cancels_and_a_release_off_every_target_reports_where() {
    let (mut cx, log, a, _) = open();
    let chip = node(&cx, "chip");
    drag(&mut cx, chip, GesturePhase::Began, 20., 20.);
    drag(&mut cx, chip, GesturePhase::Changed, 200., 140.);
    take(&log);
    let escape = Keystroke::parse("escape").unwrap();
    assert_eq!(cx.dispatch_keystroke(a, &escape), nd_ui::dispatch::KeyDispatch::Handled);
    cx.run_until_idle();
    assert_eq!(take(&log), ["a Exited 0,0", "end Cancelled"]);
    // The rest of the gesture belongs to no drag.
    drag(&mut cx, chip, GesturePhase::Ended, 200., 140.);
    assert!(take(&log).is_empty());

    drag(&mut cx, chip, GesturePhase::Began, 20., 20.);
    drag(&mut cx, chip, GesturePhase::Ended, 300., 290.);
    assert_eq!(take(&log), ["end Cancelled"], "inside a window, nothing there takes it");

    drag(&mut cx, chip, GesturePhase::Began, 20., 20.);
    drag(&mut cx, chip, GesturePhase::Ended, 450., 500.);
    assert_eq!(take(&log), ["end Outside { screen: Point { x: 450.0, y: 500.0 } }"]);
}

#[test]
fn a_drag_outside_its_window_is_previewed_until_it_comes_back() {
    let (mut cx, _log, _, _) = open();
    let chip = node(&cx, "chip");
    drag(&mut cx, chip, GesturePhase::Began, 20., 20.);
    drag(&mut cx, chip, GesturePhase::Changed, 200., 140.);
    assert!(cx.backend().previews().is_empty(), "inside its window the source draws itself");
    drag(&mut cx, chip, GesturePhase::Changed, 450., 140.);
    drag(&mut cx, chip, GesturePhase::Changed, 600., 140.);
    drag(&mut cx, chip, GesturePhase::Changed, 100., 140.);
    let shown: Vec<_> = cx.backend().previews().iter().map(|(_, p)| p.map(|p| p.x)).collect();
    assert_eq!(shown, [Some(450.), Some(600.), None]);
}

#[test]
fn a_widget_source_drives_the_same_session() {
    let (mut cx, log, _, b) = open();
    let list = node(&cx, "a-list");
    let item = DragItem::new("tab", "row-7");
    let source = |phase, at| DragSource { item: item.clone(), phase, at, native: false };
    cx.emit(list, source(GesturePhase::Began, DragPoint::Window(Point { x: 10., y: 60. })));
    cx.emit(list, source(GesturePhase::Changed, DragPoint::Window(Point { x: 10., y: 61. })));
    assert_eq!(take(&log), ["a Entered 10,21"], "a source list is a target of its own rows");
    cx.emit(list, source(GesturePhase::Ended, DragPoint::Screen(Point { x: 520., y: 100. })));
    assert_eq!(take(&log), ["a Exited 0,0", "b Entered 20,60", "b drop row-7 20,60"]);
    let _ = b;
}

#[test]
fn a_backend_owned_drag_hit_tests_the_window_it_reports() {
    let (mut cx, log, a, b) = open();
    let chip = node(&cx, "chip");
    cx.begin_drag(a, chip, DragItem::new("tab", "t1"), true);
    cx.drag_moved(b, Point { x: 30., y: 70. });
    cx.drain_events();
    assert_eq!(take(&log), ["b Entered 30,30"]);
    cx.drag_exited();
    cx.drain_events();
    assert_eq!(take(&log), ["b Exited 0,0"]);
    cx.drag_finished(Point { x: 450., y: 20. });
    cx.drain_events();
    assert_eq!(take(&log), ["end Outside { screen: Point { x: 450.0, y: 20.0 } }"]);
    assert!(cx.backend().previews().is_empty(), "the backend draws its own drags");
}

#[test]
fn resizes_are_reported_once_per_size() {
    let log: Log = Rc::default();
    let l = log.clone();
    let mut cx = TestApp::new();
    let w = cx.open_window(SIZE, move |_, cx| cx.new(|_| Side { log: l, name: "c", source: false })).id();
    let sizes = |log: &Log| take(log).into_iter().filter(|l| l.contains("size")).collect::<Vec<_>>();
    assert_eq!(sizes(&log), ["c size 400"], "the first layout reports");
    cx.refresh_window(w);
    cx.run_until_idle();
    assert!(sizes(&log).is_empty(), "the same size is not reported again");
    cx.resize_window(w, Size { width: 500., height: 300. });
    cx.run_until_idle();
    assert_eq!(sizes(&log), ["c size 500"]);
}
