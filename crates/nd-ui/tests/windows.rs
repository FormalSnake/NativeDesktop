//! Window tabs and moving views between windows (ARCHITECTURE.md 5.5): close requests the app
//! can hold, new tab requests, and a view that moves to another window keeping its native nodes.

use std::cell::Cell;
use std::rc::Rc;

use nd_gpui::gpui::{px, size};
use nd_ui::backend::NodeKind;
use nd_ui::test_app::TestApp;
use nd_ui::test_backend::{Recorded, TestBackend};
use nd_ui::transaction::Op;
use nd_ui::*;

const WINDOW: Size = Size { width: 400., height: 300. };

/// A tab's content: a counter with a nested view, so a move carries a subtree of views.
struct Page {
    count: u32,
    detail: Entity<Detail>,
}

impl Render for Page {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        v_flex().child(text(format!("page {}", self.count))).child(self.detail.clone())
    }
}

struct Detail;

impl Render for Detail {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().child(text("detail"))
    }
}

/// A window's root: shows the page when it holds it.
struct Shelf {
    page: Option<Entity<Page>>,
}

impl Render for Shelf {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        v_flex().child(text("shelf")).children(self.page.clone())
    }
}

fn find_text(cx: &TestApp, s: &str) -> Option<NodeId> {
    cx.backend().find(|_, p| p.downcast_ref::<TextProps>().is_some_and(|t| t.text.as_ref() == s)).first().copied()
}

fn ancestor_root(cx: &TestApp, mut node: NodeId) -> NodeId {
    while let Some(parent) = cx.backend().parent(node) {
        node = parent;
    }
    node
}

fn root_of(cx: &TestApp, window: WindowId) -> NodeId {
    let shelf = cx.view_node(cx.window_root(window).expect("open")).expect("window mounted");
    ancestor_root(cx, shelf)
}

/// Two windows; the page starts in `first`.
fn two_shelves(cx: &mut TestApp) -> (WindowHandle<Shelf>, WindowHandle<Shelf>, Entity<Page>) {
    let a = cx.open_window(WINDOW, |_, cx| {
        let detail = cx.new(|_| Detail);
        let page = cx.new(|_| Page { count: 0, detail });
        cx.new(|_| Shelf { page: Some(page) })
    });
    let page = a.root().read(cx).page.clone().expect("page");
    let b = cx.open_window(WINDOW, |_, cx| cx.new(|_| Shelf { page: None }));
    (a, b, page)
}

fn give(cx: &mut TestApp, from: &WindowHandle<Shelf>, to: &WindowHandle<Shelf>) {
    let page = from.root().update(cx, |s, cx| {
        cx.notify();
        s.page.take()
    });
    to.root().update(cx, |s, cx| {
        s.page = page;
        cx.notify();
    });
    cx.run_until_idle();
}

/// The ops one window's transactions carried, in order.
fn window_ops(log: &[Recorded], window: WindowId) -> Vec<Op> {
    let mut out = Vec::new();
    let mut inside = false;
    for r in log {
        match r {
            Recorded::Begin(w) => inside = *w == window,
            Recorded::Commit(_) => inside = false,
            Recorded::Op(op) if inside => out.push(op.clone()),
            _ => {}
        }
    }
    out
}

#[test]
fn a_view_moving_to_another_window_keeps_its_native_nodes() {
    // The source window draws first (its old slot is parked until the frame ends) and, moving
    // back, the target draws first (the view is lifted while still attached in its source).
    let mut cx = TestApp::new();
    let (a, b, page) = two_shelves(&mut cx);
    let node = cx.view_node(page.entity_id()).expect("mounted");
    let detail = find_text(&cx, "detail").expect("detail");
    let live = cx.backend().live_count();
    assert_eq!(ancestor_root(&cx, node), root_of(&cx, a.id()));

    for (from, to) in [(&a, &b), (&b, &a)] {
        cx.backend_mut().take_log();
        give(&mut cx, from, to);
        assert_eq!(cx.view_node(page.entity_id()), Some(node), "the page keeps its native node");
        assert_eq!(find_text(&cx, "detail"), Some(detail), "and the nodes under it");
        assert_eq!(ancestor_root(&cx, node), root_of(&cx, to.id()), "it hangs off the other window");
        assert_eq!(cx.backend().live_count(), live, "nothing created or released");
        let log = cx.backend_mut().take_log();
        let created_or_released = log.iter().any(|r| {
            matches!(r, Recorded::Op(Op::Create { node: n, .. } | Op::Release { node: n }) if *n == node || *n == detail)
        });
        assert!(!created_or_released, "no create or release for the moved nodes");
        let ops = window_ops(&log, to.id());
        let insert = ops.iter().position(|op| matches!(op, Op::Insert { node: n, .. } if *n == node));
        assert!(insert.is_some(), "the target window's transaction attaches it: {ops:?}");
        if let Some(remove) = ops.iter().position(|op| matches!(op, Op::Remove { node: n } if *n == node)) {
            assert!(remove < insert.unwrap(), "a detach the target carries comes before its attach");
        }
        let source_detaches =
            window_ops(&log, from.id()).iter().filter(|op| matches!(op, Op::Remove { node: n } if *n == node)).count();
        let target_detaches = ops.iter().filter(|op| matches!(op, Op::Remove { node: n } if *n == node)).count();
        assert_eq!(source_detaches + target_detaches, 1, "detached exactly once");
        assert_eq!(cx.render_count(page.entity_id()), 1, "moving does not re-render it");
    }

    // It still renders where it is now, inside window a's tree.
    page.update(&mut cx, |p, cx| {
        p.count = 7;
        cx.notify();
    });
    cx.run_until_idle();
    let label = find_text(&cx, "page 7").expect("re-rendered");
    assert_eq!(ancestor_root(&cx, label), root_of(&cx, a.id()));
}

#[test]
fn a_view_no_window_takes_is_released_when_the_frame_ends() {
    let mut cx = TestApp::new();
    let (a, _b, page) = two_shelves(&mut cx);
    let live = cx.backend().live_count();
    a.root().update(&mut cx, |s, cx| {
        s.page = None;
        cx.notify();
    });
    cx.run_until_idle();
    assert_eq!(cx.view_node(page.entity_id()), None);
    assert!(find_text(&cx, "detail").is_none(), "the nodes under it go too");
    // The page's column, its label, the detail's div and label.
    assert_eq!(cx.backend().live_count(), live - 4);
}

#[test]
fn a_view_that_moves_twice_in_one_frame_is_a_render_bug() {
    let mut cx = TestApp::new();
    let (a, b, page) = two_shelves(&mut cx);
    // Both windows place the page in the same frame.
    b.root().update(&mut cx, |s, cx| {
        s.page = Some(page.clone());
        cx.notify();
    });
    a.root().update(&mut cx, |_, cx| cx.notify());
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| cx.run_until_idle()));
    assert!(result.is_err(), "a view placed in two windows panics");
}

#[test]
fn closing_without_a_handler_closes_and_a_handler_can_hold_the_close() {
    let mut cx = TestApp::new();
    let plain = cx.open_window(WINDOW, |_, cx| cx.new(|_| Shelf { page: None }));
    assert!(cx.request_close(plain.id()));
    assert!(cx.window(plain.id()).is_none());

    let asked = Rc::new(Cell::new(0));
    let held = cx.open_window(WINDOW, {
        let asked = asked.clone();
        move |window, cx| {
            window.on_should_close(move |_, _| {
                asked.set(asked.get() + 1);
                false
            });
            cx.new(|_| Shelf { page: None })
        }
    });
    assert!(!cx.request_close(held.id()), "the handler held the close");
    assert!(cx.window(held.id()).is_some());
    assert_eq!(asked.get(), 1);
    // The app drops the tab's view, then closes the window itself.
    cx.close_window(held.id());
    assert!(cx.window(held.id()).is_none());
}

#[test]
fn a_new_tab_request_reaches_the_window_handler() {
    let mut cx = TestApp::new();
    let opened = Rc::new(Cell::new(0));
    let group = |cx: &mut TestApp, opened: Option<Rc<Cell<u32>>>| {
        let options = WindowOptions::main("tab").tab_group("browser").size(size(px(400.), px(300.)));
        let handle = cx.open_window_with(options, move |window, cx| {
            if let Some(opened) = opened {
                window.on_new_tab(move |_, _| opened.set(opened.get() + 1));
            }
            cx.new(|_| Shelf { page: None })
        });
        handle.id()
    };
    let with = group(&mut cx, Some(opened.clone()));
    let without = group(&mut cx, None);
    assert!(cx.request_new_tab(with));
    assert_eq!(opened.get(), 1);
    assert!(!cx.request_new_tab(without), "no handler, nothing to do");
    assert!(cx.window(with).unwrap().handles_new_tab());
    let tree = cx.semantic_tree();
    assert!(tree.windows.iter().all(|w| w.tab_group.as_deref() == Some("browser")));
}

#[test]
fn a_hidden_container_keeps_its_box_and_hides_its_subtree() {
    struct Stack;
    impl Render for Stack {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            // Two pages in one grid cell: the cell is as big as the bigger page.
            div()
                .grid()
                .grid_cols(1)
                .grid_rows(1)
                .child(div().id("small").row_start(1).col_start(1).h(px(40.)).w(px(100.)))
                .child(
                    div()
                        .id("big")
                        .role(Role::TabPanel, "Big")
                        .row_start(1)
                        .col_start(1)
                        .h(px(90.))
                        .w(px(60.))
                        .invisible()
                        .child(text("inside")),
                )
        }
    }
    let mut cx = TestApp::with_backend(TestBackend::new());
    let window = cx.open_window(WINDOW, |_, cx| cx.new(|_| Stack));
    let stack = cx.view_node(window.root().entity_id()).unwrap();
    let frame = cx.backend().frame(stack).expect("laid out");
    assert_eq!((frame.size.width, frame.size.height), (WINDOW.width, 90.), "the cell takes the bigger page");
    let tree = cx.semantic_window(window.id()).unwrap().root;
    let big = tree.iter().find(|n| n.test_id.as_deref() == Some("big")).expect("big page");
    assert_eq!(big.role, Role::TabPanel);
    assert_eq!(big.name, "Big");
    assert!(!big.visible, "a hidden page is not visible");
    assert!(big.children.iter().all(|c| !c.visible), "nor is anything in it");
    let small = tree.iter().find(|n| n.test_id.as_deref() == Some("small")).unwrap();
    assert!(small.visible);
    let _ = NodeKind::View;
}
