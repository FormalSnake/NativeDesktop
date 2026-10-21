//! The tile grid, tier 1: a dragged tile takes the slot under the pointer and a drop there
//! reports the move; Escape puts it back and reports nothing; a tile of another grid dropped in
//! reports the slot it lands in.

use std::cell::RefCell;
use std::rc::Rc;

use nd_components::{TileMoved, tile, tile_grid};
use nd_ui::prelude::*;
use nd_ui::test_app::TestApp;
use nd_ui::{DragEnded, DragEvent, Dropped, GesturePhase, Keystroke, NodeId, Point, Rect, Size, WindowId};

type Log = Rc<RefCell<Vec<String>>>;

struct Grid {
    log: Log,
}

impl Render for Grid {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let (moved, dropped, ended) = (self.log.clone(), self.log.clone(), self.log.clone());
        div().size_full().child(
            tile_grid("pinned")
                .tiles(["a", "b", "c", "d", "e", "f"].map(|id| tile(id, div().size_full())))
                .draggable_tiles("tab")
                .on_move(move |e: &TileMoved, _, _| moved.borrow_mut().push(format!("move {} {}", e.id, e.index)))
                .on_drop(move |e: &Dropped, _, _| {
                    let slot = e.slot.clone().expect("slot");
                    dropped.borrow_mut().push(format!("drop {} {}", e.item.id, slot.index))
                })
                .on_drag_end(move |e: &DragEnded, _, _| ended.borrow_mut().push(format!("end {:?}", e.outcome))),
        )
    }
}

/// Frames until the tiles' slides have run out: they animate on the frame clock.
fn settle(cx: &mut TestApp) {
    for _ in 0..6 {
        cx.drain_events();
        cx.advance_clock(1.);
        cx.frame();
    }
}

fn node(cx: &TestApp, test_id: &str) -> NodeId {
    let tree = cx.semantic_tree();
    tree.nodes().find(|(_, n)| n.test_id.as_deref() == Some(test_id)).map(|(_, n)| n.id).expect(test_id)
}

fn drag(cx: &mut TestApp, tile: NodeId, phase: GesturePhase, start: Point, to: Point) {
    let translation = Point { x: to.x - start.x, y: to.y - start.y };
    cx.events().emitter(tile).emit(DragEvent { phase, start, position: to, translation, velocity: Point::default() });
    settle(cx);
}

fn open() -> (TestApp, Log, WindowId) {
    let log: Log = Rc::default();
    let l = log.clone();
    let mut cx = TestApp::new();
    let w = cx.open_window(Size { width: 480., height: 400. }, move |_, cx| cx.new(|_| Grid { log: l })).id();
    cx.backend_mut().set_window_frame(w, Rect::new(0., 0., 480., 400.));
    // The grid learns its width from its first layout.
    settle(&mut cx);
    (cx, log, w)
}

/// Four columns of 114 pt with 8 pt gaps across 480 pt.
fn centre(slot: usize) -> Point {
    let (col, row) = ((slot % 4) as f32, (slot / 4) as f32);
    Point { x: col * 122. + 57., y: row * (114. * 0.74 + 8.) + 40. }
}

#[test]
fn a_tile_dropped_on_a_slot_reports_the_move() {
    let (mut cx, log, _) = open();
    let a = node(&cx, "a");
    let start = centre(0);
    drag(&mut cx, a, GesturePhase::Began, start, start);
    drag(&mut cx, a, GesturePhase::Changed, start, centre(2));
    drag(&mut cx, a, GesturePhase::Ended, start, centre(2));
    let log = log.borrow();
    assert_eq!(log.first().map(String::as_str), Some("move a 2"), "{log:?}");
}

#[test]
fn escape_puts_the_tile_back() {
    let (mut cx, log, w) = open();
    let b = node(&cx, "b");
    let start = centre(1);
    drag(&mut cx, b, GesturePhase::Began, start, start);
    drag(&mut cx, b, GesturePhase::Changed, start, centre(5));
    cx.dispatch_keystroke(w, &Keystroke::parse("escape").unwrap());
    settle(&mut cx);
    drag(&mut cx, b, GesturePhase::Ended, start, centre(5));
    assert_eq!(*log.borrow(), ["end Cancelled"]);
}
