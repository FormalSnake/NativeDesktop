//! Element animation on the frame clock and hover, tier 1: presented values reach the backend
//! as props on each frame, a new target mid-flight continues from where the motion is, and
//! reduce motion leaves only fades.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use nd_gpui::gpui::{SharedString, px};
use nd_ui::test_app::TestApp;
use nd_ui::*;

const WINDOW: Size = Size { width: 400., height: 300. };
const FRAME: f64 = 1. / 60.;

struct Card {
    shown: bool,
    rows: usize,
    ended: Rc<RefCell<u32>>,
}

impl Render for Card {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let ended = self.ended.clone();
        v_flex()
            .child(
                div()
                    .id("card")
                    .w(px(if self.shown { 200. } else { 100. }))
                    .h(px(40.))
                    .opacity(if self.shown { 1. } else { 0. })
                    .offset(px(0.), px(if self.shown { 0. } else { 20. }))
                    .animate(Animation::linear(Duration::from_millis(200)))
                    .on_animation_end(move |_, _, _| *ended.borrow_mut() += 1),
            )
            .children((0..self.rows).map(|i| {
                div()
                    .id(SharedString::from(format!("row{i}")))
                    .h(px(10.))
                    .enter_from(|p| p.opacity(0.).offset(px(0.), px(8.)))
            }))
    }
}

fn props(cx: &TestApp, id: NodeId) -> DivProps {
    cx.backend().props(id).and_then(|p| p.downcast_ref::<DivProps>()).cloned().expect("div")
}

fn node(cx: &TestApp, test_id: &str) -> NodeId {
    let tree = cx.semantic_tree();
    tree.nodes().find(|(_, n)| n.test_id.as_deref() == Some(test_id)).map(|(_, n)| n.id).expect(test_id)
}

fn step(cx: &mut TestApp, seconds: f64) {
    cx.advance_clock(seconds);
    cx.frame();
}

fn open(cx: &mut TestApp, ended: Rc<RefCell<u32>>) -> WindowHandle<Card> {
    cx.open_window(WINDOW, move |_, cx| cx.new(|_| Card { shown: false, rows: 0, ended }))
}

#[test]
fn a_changed_target_animates_on_the_frame_clock_and_reports_its_end() {
    let ended = Rc::new(RefCell::new(0));
    let mut cx = TestApp::new();
    let handle = open(&mut cx, ended.clone());
    let card = node(&cx, "card");
    assert_eq!(props(&cx, card).style.opacity, Some(0.), "a first render shows its values at once");
    assert!(!cx.needs_frame(), "nothing moves, nothing draws");

    handle.root().update(&mut cx, |c, cx| {
        c.shown = true;
        cx.notify();
    });
    cx.frame();
    assert!(cx.needs_frame(), "a motion in flight asks for frames");
    step(&mut cx, 0.1);
    let mid = props(&cx, card);
    assert!((mid.style.opacity.unwrap() - 0.5).abs() < 0.02, "halfway at 100 ms: {:?}", mid.style.opacity);
    assert!((mid.transform.offset.1 - 10.).abs() < 0.5);
    let width = cx.backend().frame(card).unwrap().size.width;
    assert!((width - 150.).abs() < 2., "the size animates through layout: {width}");

    for _ in 0..12 {
        step(&mut cx, FRAME);
    }
    cx.run_until_idle();
    let done = props(&cx, card);
    assert_eq!(done.style.opacity, Some(1.));
    assert_eq!(done.transform.offset, (0., 0.));
    assert_eq!(cx.backend().frame(card).unwrap().size.width, 200.);
    assert!(!cx.needs_frame(), "a settled window is idle");
    assert_eq!(*ended.borrow(), 1);
}

#[test]
fn a_reversal_mid_flight_continues_from_where_it_is() {
    let mut cx = TestApp::new();
    let handle = open(&mut cx, Rc::default());
    let card = node(&cx, "card");
    handle.root().update(&mut cx, |c, cx| {
        c.shown = true;
        cx.notify();
    });
    cx.frame();
    step(&mut cx, 0.05);
    let at = props(&cx, card).style.opacity.unwrap();
    handle.root().update(&mut cx, |c, cx| {
        c.shown = false;
        cx.notify();
    });
    cx.frame();
    let after = props(&cx, card).style.opacity.unwrap();
    assert!((after - at).abs() < 0.02, "no jump back to either end: {at} then {after}");
    step(&mut cx, 0.1);
    assert!(props(&cx, card).style.opacity.unwrap() < after, "heading back down");
}

#[test]
fn mounted_rows_enter_from_their_pose() {
    let mut cx = TestApp::new();
    let handle = open(&mut cx, Rc::default());
    handle.root().update(&mut cx, |c, cx| {
        c.rows = 1;
        cx.notify();
    });
    cx.frame();
    let row = node(&cx, "row0");
    let first = props(&cx, row);
    assert_eq!(first.style.opacity, Some(0.), "a new row starts transparent");
    assert_eq!(first.transform.offset.1, 8.);
    for _ in 0..30 {
        step(&mut cx, FRAME);
    }
    let last = props(&cx, row);
    assert_eq!(last.style.opacity, Some(1.));
    assert_eq!(last.transform.offset.1, 0.);
    handle.root().update(&mut cx, |c, cx| {
        c.rows = 2;
        cx.notify();
    });
    cx.frame();
    assert_eq!(props(&cx, row).style.opacity, Some(1.), "a row already mounted does not enter again");
}

#[test]
fn reduce_motion_jumps_movement_and_keeps_the_fade() {
    let mut cx = TestApp::new();
    cx.set_reduce_motion(true);
    let handle = open(&mut cx, Rc::default());
    let card = node(&cx, "card");
    handle.root().update(&mut cx, |c, cx| {
        c.shown = true;
        cx.notify();
    });
    cx.frame();
    step(&mut cx, 0.1);
    let mid = props(&cx, card);
    assert_eq!(mid.transform.offset.1, 0., "the offset jumped");
    assert_eq!(cx.backend().frame(card).unwrap().size.width, 200., "the size jumped");
    assert!(mid.style.opacity.unwrap() < 1., "the fade still runs");
}

struct Hoverable {
    log: Rc<RefCell<Vec<bool>>>,
}

impl Render for Hoverable {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let log = self.log.clone();
        div()
            .id("row")
            .hover(|s| s.bg(nd_gpui::gpui::red()))
            .on_hover(move |hovered, _, _| log.borrow_mut().push(*hovered))
    }
}

#[test]
fn hover_style_reaches_the_backend_and_on_hover_hears_enter_and_exit() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let mut cx = TestApp::new();
    let l = log.clone();
    cx.open_window(WINDOW, move |_, cx| cx.new(|_| Hoverable { log: l }));
    let row = node(&cx, "row");
    let p = props(&cx, row);
    assert!(p.tracks_hover);
    assert!(p.hover.as_ref().and_then(|h| h.background.as_ref()).is_some(), "the backend applies it itself");
    cx.emit(row, HoverChanged { hovered: true });
    cx.emit(row, HoverChanged { hovered: false });
    cx.run_until_idle();
    assert_eq!(*log.borrow(), [true, false]);
}
