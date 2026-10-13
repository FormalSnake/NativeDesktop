//! Views, the reconciler, layout and the native event queue on `TestBackend` (3.7, 3.13, 3.14).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use nd_gpui::gpui::px;
use nd_ui::backend::{NodeKind, Props, WidgetKind};
use nd_ui::events::Synced;
use nd_ui::test_app::TestApp;
use nd_ui::test_backend::{Recorded, TestBackend};
use nd_ui::transaction::Op;
use nd_ui::*;

const WINDOW: Size = Size { width: 800., height: 600. };

fn text_props(props: &Props) -> Option<&TextProps> {
    props.downcast_ref::<TextProps>()
}

fn find_text(cx: &TestApp, s: &str) -> Option<NodeId> {
    cx.backend().find(|_, p| text_props(p).is_some_and(|t| t.text.as_ref() == s)).first().copied()
}

fn ops(cx: &TestApp) -> Vec<Op> {
    cx.backend().ops().cloned().collect()
}

/// Text measures 8 points per character by 20 high.
fn measuring_backend() -> TestBackend {
    let mut backend = TestBackend::new();
    backend.on_measure(|_, props, _| match text_props(props) {
        Some(t) => Size { width: 8. * t.text.len() as f32, height: 20. },
        None => Size { width: 100., height: 24. },
    });
    backend
}

/// A stateful native field, keyed by id like every stateful widget.
struct Field;

#[derive(Clone, Debug, PartialEq)]
struct FieldProps;

struct Form {
    banner: bool,
}

impl Render for Form {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .when(self.banner, |d| d.child(text("banner")))
            .child(NativeElement::new::<Field>("name", FieldProps))
            .child(div().child(text("footer")))
    }
}

#[test]
fn when_toggle_keeps_sibling_native_nodes() {
    let mut cx = TestApp::new();
    let window = cx.open_window(WINDOW, |_, cx| cx.new(|_| Form { banner: false }));
    let field = cx.backend().find(|k, _| k == NodeKind::Widget(WidgetKind::of::<Field>()))[0];
    let footer = find_text(&cx, "footer").unwrap();
    let form = window.root().clone();
    cx.backend_mut().take_log();

    form.update(&mut cx, |form, cx| {
        form.banner = true;
        cx.notify();
    });
    cx.run_until_idle();
    let banner = find_text(&cx, "banner").unwrap();
    assert_eq!(cx.backend().created(), vec![banner], "only the banner is created");
    let column = cx.backend().parent(field).unwrap();
    assert_eq!(cx.backend().children(column)[0], banner);
    assert!(cx.backend().is_live(field) && cx.backend().is_live(footer));
    cx.backend_mut().take_log();

    form.update(&mut cx, |form, cx| {
        form.banner = false;
        cx.notify();
    });
    cx.run_until_idle();
    let structural: Vec<Op> = ops(&cx)
        .into_iter()
        .filter(|op| matches!(op, Op::Create { .. } | Op::Remove { .. } | Op::Release { .. }))
        .collect();
    assert!(
        matches!(structural.as_slice(), [Op::Remove { node: a }, Op::Release { node: b }] if *a == banner && *b == banner),
        "{structural:?}"
    );
    assert_eq!(cx.backend().children(column).len(), 2);
}

struct Keyed {
    order: Vec<&'static str>,
}

impl Render for Keyed {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        v_flex().children(self.order.iter().map(|id| div().id(*id).child(text(*id))))
    }
}

#[test]
fn keyed_children_reorder_with_moves_only() {
    let mut cx = TestApp::new();
    let window = cx.open_window(WINDOW, |_, cx| cx.new(|_| Keyed { order: vec!["a", "b", "c", "d"] }));
    let row = |cx: &TestApp, s| cx.backend().parent(find_text(cx, s).unwrap()).unwrap();
    let before: Vec<NodeId> = ["a", "b", "c", "d"].iter().map(|s| row(&cx, *s)).collect();
    let list = cx.backend().parent(before[0]).unwrap();
    cx.backend_mut().take_log();

    window.root().update(&mut cx, |k, cx| {
        k.order = vec!["d", "b", "a", "c"];
        cx.notify();
    });
    cx.run_until_idle();
    assert!(ops(&cx).iter().all(|op| matches!(op, Op::Move { .. })), "{:?}", ops(&cx));
    let expected: Vec<NodeId> = [3, 1, 0, 2].iter().map(|i| before[*i]).collect();
    assert_eq!(cx.backend().children(list), expected.as_slice());
}

struct Counter {
    label: &'static str,
    count: u32,
}

impl Render for Counter {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        text(format!("{} {}", self.label, self.count))
    }
}

struct Pair {
    a: Entity<Counter>,
    b: Entity<Counter>,
}

impl Render for Pair {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        h_flex().child(self.a.clone()).child(self.b.clone())
    }
}

fn pair(cx: &mut TestApp) -> (Entity<Pair>, Entity<Counter>, Entity<Counter>) {
    let window = cx.open_window(WINDOW, |_, cx| {
        let a = cx.new(|_| Counter { label: "a", count: 0 });
        let b = cx.new(|_| Counter { label: "b", count: 0 });
        cx.new(|_| Pair { a, b })
    });
    let root = window.root().clone();
    let (a, b) = {
        let p = root.read(cx);
        (p.a.clone(), p.b.clone())
    };
    cx.backend_mut().take_log();
    (root, a, b)
}

fn bump(cx: &mut App, counter: &Entity<Counter>) {
    counter.update(cx, |c, cx| {
        c.count += 1;
        cx.notify();
    });
}

#[test]
fn notify_rerenders_only_that_view() {
    let mut cx = TestApp::new();
    let (root, a, b) = pair(&mut cx);
    let a_node = find_text(&cx, "a 0").unwrap();

    bump(&mut cx, &a);
    cx.run_until_idle();
    assert_eq!(
        (cx.render_count(root.entity_id()), cx.render_count(a.entity_id()), cx.render_count(b.entity_id())),
        (1, 2, 1)
    );
    let ops = ops(&cx);
    let updates: Vec<NodeId> = ops
        .iter()
        .filter_map(|op| match op {
            Op::Update { node, .. } => Some(*node),
            _ => None,
        })
        .collect();
    assert_eq!(updates, vec![a_node]);
    assert!(ops.iter().all(|op| matches!(op, Op::Update { .. } | Op::SetFrame { .. })), "{ops:?}");
    assert_eq!(cx.view_node(a.entity_id()), Some(a_node));
}

#[test]
fn notifies_in_one_frame_share_one_transaction() {
    let mut cx = TestApp::new();
    let (_, a, b) = pair(&mut cx);
    bump(&mut cx, &a);
    bump(&mut cx, &b);
    bump(&mut cx, &a);
    cx.run_until_idle();
    assert_eq!(cx.backend().commits(), 1);
    assert_eq!(cx.render_count(a.entity_id()), 2);
    assert!(find_text(&cx, "a 2").is_some() && find_text(&cx, "b 1").is_some());
}

struct Toggle {
    child: Entity<Counter>,
    shown: bool,
}

impl Render for Toggle {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().when(self.shown, |d| d.child(div().child(self.child.clone())))
    }
}

#[test]
fn hiding_a_view_releases_its_nodes_and_keeps_its_state() {
    let mut cx = TestApp::new();
    let window = cx.open_window(WINDOW, |_, cx| {
        let child = cx.new(|_| Counter { label: "c", count: 7 });
        cx.new(|_| Toggle { child, shown: true })
    });
    let root = window.root().clone();
    let live = cx.backend().live_count();
    let child = root.read(&cx).child.clone();
    root.update(&mut cx, |t, cx| {
        t.shown = false;
        cx.notify();
    });
    cx.run_until_idle();
    assert_eq!(cx.backend().live_count(), live - 2);
    assert_eq!(cx.view_node(child.entity_id()), None);

    root.update(&mut cx, |t, cx| {
        t.shown = true;
        cx.notify();
    });
    cx.run_until_idle();
    assert!(find_text(&cx, "c 7").is_some());
    assert_eq!(cx.backend().live_count(), live);
}

#[test]
fn observers_run_after_notify_and_drive_renders() {
    struct Model(u32);
    struct Watcher {
        seen: u32,
    }
    impl Render for Watcher {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            text(format!("seen {}", self.seen))
        }
    }

    let mut cx = TestApp::new();
    let model = cx.new(|_| Model(0));
    let m = model.clone();
    cx.open_window(WINDOW, move |_, cx| {
        cx.new(|cx| {
            cx.observe(&m, |this: &mut Watcher, m, cx| {
                this.seen = m.read(cx).0;
                cx.notify();
            })
            .detach();
            Watcher { seen: 0 }
        })
    });
    model.update(&mut cx, |m, cx| {
        m.0 = 5;
        cx.notify();
    });
    cx.run_until_idle();
    assert!(find_text(&cx, "seen 5").is_some());
}

/// A widget whose native side emits from inside create, as AppKit delegates do from setters.
struct Eager;

#[derive(Clone, Debug, PartialEq)]
struct EagerProps;

#[derive(Debug)]
struct Loaded;

struct Host {
    loaded: Rc<Cell<u32>>,
    inside_transaction: Rc<Cell<bool>>,
    renders_seen: u32,
}

impl Render for Host {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (loaded, inside) = (self.loaded.clone(), self.inside_transaction.clone());
        v_flex().child(text(format!("loaded {}", self.renders_seen))).child(
            NativeElement::new::<Eager>("eager", EagerProps).on(cx.listener(move |this, _: &Loaded, _window, cx| {
                inside.set(inside.get() || cx.backend_as::<TestBackend>().unwrap().in_transaction());
                loaded.set(loaded.get() + 1);
                this.renders_seen += 1;
                cx.notify();
            })),
        )
    }
}

#[test]
fn native_callbacks_queue_and_run_after_commit() {
    let mut backend = TestBackend::new();
    backend.on_apply(|node, kind, _, events| {
        if kind == NodeKind::Widget(WidgetKind::of::<Eager>()) {
            events.emitter(node).emit(Loaded);
        }
    });
    let loaded = Rc::new(Cell::new(0));
    let inside = Rc::new(Cell::new(false));
    let mut cx = TestApp::with_backend(backend);
    let (l, i) = (loaded.clone(), inside.clone());
    cx.open_window(WINDOW, move |_, cx| cx.new(|_| Host { loaded: l, inside_transaction: i, renders_seen: 0 }));
    // The handler updated the view that owns the widget; that would panic on a lease if it ran
    // inside the render or the apply.
    assert_eq!(loaded.get(), 1);
    assert!(!inside.get());
    assert!(find_text(&cx, "loaded 1").is_some());
    let log = cx.backend().log();
    assert!(matches!(log.last(), Some(Recorded::Commit(_))));
}

struct Slider;

#[derive(Clone, Debug, PartialEq)]
struct SliderProps {
    value: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct SliderChanged(f32);

struct Volume {
    value: f32,
    events: u32,
}

impl Render for Volume {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        NativeElement::new::<Slider>("volume", SliderProps { value: self.value }).on(cx.listener(
            |this, e: &SliderChanged, _window, cx| {
                this.value = e.0;
                this.events += 1;
                cx.notify();
            },
        ))
    }
}

/// A native slider snapping to quarters, whose setter fires its own change callback
/// synchronously, as `MKMapView.setRegion` fires `regionDidChange`.
fn slider_backend(sets: Rc<Cell<u32>>, state: Rc<RefCell<HashMap<NodeId, Synced<f32>>>>) -> TestBackend {
    let mut backend = TestBackend::new();
    backend.on_apply(move |node, _, props, events| {
        let Some(props) = props.downcast_ref::<SliderProps>() else { return };
        let emit = events.emitter(node);
        let mut state = state.borrow_mut();
        let synced = state.entry(node).or_default();
        let normalised = synced.apply(&props.value, |v| {
            sets.set(sets.get() + 1);
            let snapped = (v * 4.).round() / 4.;
            emit.programmatic(|| emit.emit(SliderChanged(snapped)));
            snapped
        });
        if let Some(v) = normalised {
            emit.emit(SliderChanged(v));
        }
    });
    backend
}

#[test]
fn controlled_values_do_not_echo() {
    let sets = Rc::new(Cell::new(0));
    let state = Rc::new(RefCell::new(HashMap::new()));
    let mut cx = TestApp::with_backend(slider_backend(sets.clone(), state.clone()));
    let window = cx.open_window(WINDOW, |_, cx| cx.new(|_| Volume { value: 0., events: 0 }));
    let volume = window.root().clone();
    assert_eq!(sets.get(), 1);

    // The app sets a value the control normalises: one set, one event, and feeding the
    // normalised value back sets nothing.
    volume.update(&mut cx, |v, cx| {
        v.value = 0.33;
        cx.notify();
    });
    cx.run_until_idle();
    assert_eq!(sets.get(), 2);
    assert_eq!(volume.read(&cx).events, 1);
    assert_eq!(volume.read(&cx).value, 0.25);

    // User input: reported, delivered once, and the app's echo sets nothing.
    let node = cx.view_node(volume.entity_id()).unwrap();
    state.borrow_mut().get_mut(&node).unwrap().report(0.5);
    cx.emit(node, SliderChanged(0.5));
    assert_eq!(sets.get(), 2);
    assert_eq!(volume.read(&cx).events, 2);
    let props = cx.backend().props(node).unwrap().downcast_ref::<SliderProps>().unwrap().clone();
    assert_eq!(props, SliderProps { value: 0.5 });
}

struct Sidebar {
    rows: Vec<&'static str>,
}

impl Render for Sidebar {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .size_full()
            .child(
                v_flex()
                    .id("sidebar")
                    .w(px(220.))
                    .h_full()
                    .p_2()
                    .gap_1()
                    .children(self.rows.iter().map(|r| div().id(*r).child(text(*r)))),
            )
            .child(div().id("content").flex_1().h_full().child(text("Body")))
    }
}

#[test]
fn sidebar_and_list_layout() {
    let mut cx = TestApp::with_backend(measuring_backend());
    cx.open_window(WINDOW, |_, cx| cx.new(|_| Sidebar { rows: vec!["Inbox", "Drafts", "Archive"] }));
    let frame = |cx: &TestApp, node| cx.backend().frame(node).unwrap();
    let row = |cx: &TestApp, s| cx.backend().parent(find_text(cx, s).unwrap()).unwrap();
    let sidebar = cx.backend().parent(row(&cx, "Inbox")).unwrap();
    let content = cx.backend().parent(find_text(&cx, "Body").unwrap()).unwrap();
    let split = cx.backend().parent(sidebar).unwrap();

    assert_eq!(frame(&cx, split), Rect::new(0., 0., 800., 600.));
    assert_eq!(frame(&cx, sidebar), Rect::new(0., 0., 220., 600.));
    assert_eq!(frame(&cx, content), Rect::new(220., 0., 580., 600.));
    // Rows stack inside 8 points of padding with 4 points between, stretched to the column.
    assert_eq!(frame(&cx, row(&cx, "Inbox")), Rect::new(8., 8., 204., 20.));
    assert_eq!(frame(&cx, row(&cx, "Drafts")), Rect::new(8., 32., 204., 20.));
    assert_eq!(frame(&cx, row(&cx, "Archive")), Rect::new(8., 56., 204., 20.));
    // `content` is a block box, so its text fills the width and takes its measured height.
    assert_eq!(frame(&cx, find_text(&cx, "Body").unwrap()), Rect::new(0., 0., 580., 20.));

    // A resize moves frames and nothing else; untouched frames are not resent.
    cx.backend_mut().take_log();
    let id = cx.window_ids().next().unwrap();
    cx.resize_window(id, Size { width: 1000., height: 700. });
    cx.run_until_idle();
    assert!(ops(&cx).iter().all(|op| matches!(op, Op::SetFrame { .. })), "{:?}", ops(&cx));
    assert_eq!(frame(&cx, content), Rect::new(220., 0., 780., 700.));
    let moved: Vec<NodeId> = ops(&cx)
        .iter()
        .filter_map(|op| match op {
            Op::SetFrame { node, .. } => Some(*node),
            _ => None,
        })
        .collect();
    assert!(!moved.contains(&row(&cx, "Inbox")));
}

#[test]
fn closing_a_window_releases_every_node() {
    let mut cx = TestApp::new();
    let window = cx.open_window(WINDOW, |_, cx| cx.new(|_| Keyed { order: vec!["a", "b"] }));
    assert!(cx.backend().live_count() > 0);
    cx.close_window(window.id());
    assert_eq!(cx.backend().live_count(), 0);
}

/// One child view that hops between two panes. `left` decides which pane holds it; the panes
/// are siblings, so one of them is reconciled before the other whichever way it moves.
struct Panes {
    child: Entity<Counter>,
    left: bool,
    /// Wraps the right pane's copy in an extra `div`, so a move out of it removes a native node
    /// with the view still inside.
    wrapped: bool,
}

impl Render for Panes {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let child = self.child.clone();
        h_flex().child(div().id("left").when(self.left, |d| d.child(child.clone()))).child(div().id("right").when(
            !self.left,
            |d| {
                if self.wrapped { d.child(div().id("wrap").child(child.clone())) } else { d.child(child.clone()) }
            },
        ))
    }
}

fn panes(cx: &mut TestApp, left: bool, wrapped: bool) -> (Entity<Panes>, Entity<Counter>) {
    let window = cx.open_window(WINDOW, |_, cx| {
        let child = cx.new(|_| Counter { label: "c", count: 3 });
        cx.new(|_| Panes { child, left, wrapped })
    });
    let root = window.root().clone();
    let child = root.read(cx).child.clone();
    (root, child)
}

fn set_left(cx: &mut TestApp, root: &Entity<Panes>, left: bool) {
    root.update(cx, |p, cx| {
        p.left = left;
        cx.notify();
    });
    cx.run_until_idle();
}

#[test]
fn a_view_moves_between_two_parents_in_one_render_and_keeps_its_node() {
    for (from_left, wrapped) in [(false, false), (true, false), (false, true)] {
        let mut cx = TestApp::new();
        let (root, child) = panes(&mut cx, from_left, wrapped);
        let node = cx.view_node(child.entity_id()).expect("mounted");
        let parent = cx.backend().parent(node);
        let live = cx.backend().live_count();
        cx.backend_mut().take_log();

        set_left(&mut cx, &root, !from_left);
        let case = format!("from_left={from_left} wrapped={wrapped}");
        assert_eq!(cx.view_node(child.entity_id()), Some(node), "{case}: the view keeps its native node");
        assert!(find_text(&cx, "c 3").is_some_and(|n| n == node), "{case}");
        assert_ne!(cx.backend().parent(node), parent, "{case}: it now hangs off the other pane");
        assert!(
            !ops(&cx).iter().any(|op| matches!(op, Op::Create { node: n, .. } | Op::Release { node: n } if *n == node))
        );
        assert_eq!(cx.render_count(child.entity_id()), 1, "{case}: moving does not re-render it");
        let wrapper = usize::from(wrapped);
        assert_eq!(cx.backend().live_count(), live - wrapper, "{case}: only the wrapper is released");

        // It still renders where it went, and moves back.
        bump(&mut cx, &child);
        cx.run_until_idle();
        assert!(find_text(&cx, "c 4").is_some_and(|n| n == node), "{case}");
        set_left(&mut cx, &root, from_left);
        assert_eq!(cx.view_node(child.entity_id()), Some(node), "{case}: and back");
    }
}

/// A view whose root element changes kind keeps the child views inside it: the new root is
/// mounted before the old one is unmounted, with the child view still under the old one.
struct Shell {
    child: Entity<Counter>,
    column: bool,
}

impl Render for Shell {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        if self.column { v_flex().child(self.child.clone()) } else { h_flex().child(self.child.clone()) }
    }
}

#[test]
fn a_new_root_element_adopts_the_child_views_of_the_old_one() {
    let mut cx = TestApp::new();
    let window = cx.open_window(WINDOW, |_, cx| {
        let child = cx.new(|_| Counter { label: "s", count: 0 });
        cx.new(|_| Shell { child, column: false })
    });
    let root = window.root().clone();
    let child = root.read(&cx).child.clone();
    let node = cx.view_node(child.entity_id()).unwrap();
    root.update(&mut cx, |s, cx| {
        s.column = true;
        cx.notify();
    });
    cx.run_until_idle();
    assert_eq!(cx.view_node(child.entity_id()), Some(node));
    assert_eq!(cx.render_count(child.entity_id()), 1);
}

#[test]
#[should_panic(expected = "rendered in two places")]
fn one_view_in_two_places_in_one_render_still_panics() {
    struct Twice(Entity<Counter>);
    impl Render for Twice {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            h_flex().child(div().child(self.0.clone())).child(div().child(self.0.clone()))
        }
    }
    let mut cx = TestApp::new();
    cx.open_window(WINDOW, |_, cx| {
        let child = cx.new(|_| Counter { label: "t", count: 0 });
        cx.new(|_| Twice(child))
    });
}

/// A view that notifies itself while it renders, around a child view. Its first frame mounts it;
/// the notify must wait for the next frame, not render it again into the frame that just placed
/// the child (which panicked with "rendered in two places").
struct Ticking {
    child: Entity<Leaf>,
    frames: u32,
}

struct Leaf;

impl Render for Leaf {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        text("leaf")
    }
}

impl Render for Ticking {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.frames += 1;
        if self.frames < 3 {
            cx.notify();
        }
        div().child(self.child.clone())
    }
}

#[test]
fn a_view_notifying_itself_on_mount_renders_once_per_frame() {
    let mut cx = TestApp::new();
    let handle = cx.open_window(WINDOW, |_, cx| {
        let child = cx.new(|_| Leaf);
        cx.new(|_| Ticking { child, frames: 0 })
    });
    let view = handle.root().clone();
    assert_eq!(view.read(&cx).frames, 3);
    assert!(find_text(&cx, "leaf").is_some());
}
