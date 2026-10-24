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
    assert_eq!(props(&cx, row).style.opacity.unwrap_or(1.), 1., "a row already mounted does not enter again");
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

struct Tile;

impl Render for Tile {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("tile")
            .group("tile")
            .size(px(100.))
            .child(div().id("play").size(px(20.)).reveal_on_hover("tile"))
            .child(div().id("other").size(px(20.)).reveal_on_hover("row"))
    }
}

fn hover(cx: &mut TestApp, node: NodeId, hovered: bool) {
    cx.events().emitter(node).emit(HoverChanged { hovered });
    cx.drain_events();
    cx.frame();
}

#[test]
fn a_hover_group_fades_its_reveals_in_and_out_on_the_platform_timing() {
    let mut cx = TestApp::new();
    cx.open_window(WINDOW, |_, cx| cx.new(|_| Tile));
    let (tile, play, other) = (node(&cx, "tile"), node(&cx, "play"), node(&cx, "other"));
    assert!(props(&cx, tile).tracks_hover, "a group hears the pointer");
    assert_eq!(props(&cx, play).style.opacity, Some(0.));
    assert!(!cx.needs_frame());

    hover(&mut cx, tile, true);
    let fast = cx.motion().fast.as_secs_f64();
    step(&mut cx, fast / 2.);
    let mid = props(&cx, play).style.opacity.unwrap();
    assert!(mid > 0.05 && mid < 0.95, "fading in: {mid}");
    assert_eq!(props(&cx, other).style.opacity, Some(0.), "another group's reveal stays hidden");
    step(&mut cx, fast);
    assert_eq!(props(&cx, play).style.opacity, Some(1.));
    assert!(!cx.needs_frame(), "idle once shown");

    hover(&mut cx, tile, false);
    step(&mut cx, fast / 2.);
    assert!(props(&cx, play).style.opacity.unwrap() < 1.);
    step(&mut cx, fast);
    assert_eq!(props(&cx, play).style.opacity, Some(0.));
}

#[test]
fn a_themed_animation_follows_the_apps_motion_tokens() {
    let mut cx = TestApp::new();
    cx.set_motion(MotionTokens { fast: Duration::from_millis(400), curve: Curve::Linear, ..MotionTokens::default() });
    cx.open_window(WINDOW, |_, cx| cx.new(|_| Tile));
    let (tile, play) = (node(&cx, "tile"), node(&cx, "play"));
    hover(&mut cx, tile, true);
    step(&mut cx, 0.1);
    let at = props(&cx, play).style.opacity.unwrap();
    assert!((at - 0.25).abs() < 0.03, "a quarter of 400 ms, linear: {at}");
}

struct Presented {
    open: bool,
    away: bool,
    landed: Rc<RefCell<u32>>,
}

impl Render for Presented {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let landed = self.landed.clone();
        div().size_full().child(div().id("thumb").absolute().left(px(10.)).top(px(250.)).size(px(40.))).when(
            self.open,
            |root| {
                root.child(
                    div().absolute().left(px(100.)).top(px(20.)).size(px(200.)).child(
                        div()
                            .id("cover")
                            .absolute()
                            .size(px(200.))
                            .hero_from("thumb", self.away)
                            .animate(Animation::linear(Duration::from_millis(200)))
                            .on_animation_end(move |_, _, _| *landed.borrow_mut() += 1),
                    ),
                )
            },
        )
    }
}

#[test]
fn a_hero_comes_in_from_its_source_frame_and_goes_back_to_it() {
    let mut cx = TestApp::new();
    let handle =
        cx.open_window(WINDOW, |_, cx| cx.new(|_| Presented { open: false, away: false, landed: Rc::default() }));
    handle.root().update(&mut cx, |p, cx| {
        p.open = true;
        cx.notify();
    });
    cx.frame();
    let cover = node(&cx, "cover");
    assert_eq!(props(&cx, cover).style.opacity, Some(0.), "hidden until its frame is known");
    assert!(cx.needs_frame());
    cx.frame();
    let start = props(&cx, cover);
    assert_eq!(start.style.opacity, Some(1.));
    assert!((start.transform.offset.0 - -90.).abs() < 0.5, "on the thumb: {:?}", start.transform.offset);
    assert!((start.transform.offset.1 - 230.).abs() < 0.5);
    assert!((cx.backend().frame(cover).unwrap().size.width - 40.).abs() < 0.5, "at the thumb's size");
    step(&mut cx, 0.25);
    cx.run_until_idle();
    assert_eq!(props(&cx, cover).transform.offset, (0., 0.));
    assert_eq!(cx.backend().frame(cover).unwrap().size.width, 200.);

    handle.root().update(&mut cx, |p, cx| {
        p.away = true;
        cx.notify();
    });
    cx.frame();
    cx.frame();
    step(&mut cx, 0.1);
    assert_eq!(props(&cx, cover).transform.offset, (0., 0.), "waits while what surrounds it fades");
    step(&mut cx, 0.3);
    let back = props(&cx, cover);
    assert!((back.transform.offset.0 - -90.).abs() < 0.5, "back on the thumb: {:?}", back.transform.offset);
    assert!((cx.backend().frame(cover).unwrap().size.width - 40.).abs() < 0.5);
}

#[test]
fn reduce_motion_shows_a_hero_in_place() {
    let mut cx = TestApp::new();
    cx.set_reduce_motion(true);
    let handle =
        cx.open_window(WINDOW, |_, cx| cx.new(|_| Presented { open: false, away: false, landed: Rc::default() }));
    handle.root().update(&mut cx, |p, cx| {
        p.open = true;
        cx.notify();
    });
    cx.frame();
    let cover = node(&cx, "cover");
    assert_eq!(props(&cx, cover).style.opacity, None, "not held back");
    assert_eq!(props(&cx, cover).transform.offset, (0., 0.));
}

#[test]
fn reduce_motion_lands_a_hero_going_back_at_once() {
    let mut cx = TestApp::new();
    cx.set_reduce_motion(true);
    let landed = Rc::new(RefCell::new(0));
    let handle = cx.open_window(WINDOW, {
        let landed = landed.clone();
        move |_, cx| cx.new(|_| Presented { open: true, away: false, landed })
    });
    cx.run_until_idle();
    let cover = node(&cx, "cover");
    *landed.borrow_mut() = 0;
    handle.root().update(&mut cx, |p, cx| {
        p.away = true;
        cx.notify();
    });
    cx.run_until_idle();
    assert_eq!(*landed.borrow(), 1, "a presentation waiting on it can close");
    assert_eq!(props(&cx, cover).transform.offset, (0., 0.), "no travel");
    handle.root().update(&mut cx, |_, cx| cx.notify());
    cx.run_until_idle();
    assert_eq!(*landed.borrow(), 1, "lands once, not on every render");
}

#[test]
fn reduce_motion_hides_a_hero_going_back_in_the_frame_it_goes() {
    let mut cx = TestApp::new();
    cx.set_reduce_motion(true);
    let handle =
        cx.open_window(WINDOW, |_, cx| cx.new(|_| Presented { open: true, away: false, landed: Rc::default() }));
    cx.run_until_idle();
    let cover = node(&cx, "cover");
    handle.root().update(&mut cx, |p, cx| {
        p.away = true;
        cx.notify();
    });
    cx.frame();
    assert_eq!(props(&cx, cover).style.opacity, Some(0.), "not left at its expanded frame");
    step(&mut cx, FRAME);
    cx.run_until_idle();
    assert_eq!(props(&cx, cover).style.opacity, Some(0.), "stays hidden");
}
