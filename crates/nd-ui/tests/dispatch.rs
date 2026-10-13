//! nd-owned focus and dispatch on `TestBackend` (3.12).

use nd_gpui::gpui;
use nd_ui::dispatch::{DispatchTree, Focusable, KeyDispatch};
use nd_ui::test_backend::TestBackend;
use nd_ui::transaction::{MeasureCache, Op, Transaction};
use nd_ui::{KeyBinding, KeyContext, Keystroke, NodeId, WindowId};

gpui::actions!(spike, [Ping, Save, Chord]);

#[derive(Default)]
struct App {
    log: Vec<&'static str>,
}

const ROOT: NodeId = NodeId(1);
const SIDEBAR: NodeId = NodeId(2);
const LIST: NodeId = NodeId(3);
const EDITOR: NodeId = NodeId(4);
const TOOLBAR_SEARCH: NodeId = NodeId(5);

fn key(source: &str) -> Keystroke {
    Keystroke::parse(source).expect("valid keystroke")
}

/// Workspace root with a toolbar search field, a sidebar holding a list, and an editor.
fn tree(bindings: Vec<KeyBinding>) -> DispatchTree<App> {
    let mut tree = DispatchTree::new(bindings);
    tree.set_root(ROOT);
    tree.set_key_context(ROOT, Some(KeyContext::parse("Workspace").unwrap()));
    tree.insert(ROOT, TOOLBAR_SEARCH, None);
    tree.insert(ROOT, SIDEBAR, None);
    tree.insert(SIDEBAR, LIST, None);
    tree.insert(ROOT, EDITOR, None);
    tree.set_key_context(EDITOR, Some(KeyContext::parse("Editor").unwrap()));
    for n in [TOOLBAR_SEARCH, LIST, EDITOR] {
        tree.set_focusable(n, Some(Focusable::default()));
    }
    tree
}

#[test]
fn a_binding_fires_in_its_context_and_only_there() {
    let mut tree = tree(vec![KeyBinding::new("secondary-k", Ping, Some("Editor"))]);
    tree.on_action(EDITOR, |_: &Ping, cx| cx.cx.log.push("ping"));
    let mut app = App::default();

    assert!(tree.focus(EDITOR));
    assert_eq!(tree.dispatch_key(&key("secondary-k"), &mut app), KeyDispatch::Handled);
    assert_eq!(app.log, ["ping"]);

    tree.focus(LIST);
    assert_eq!(tree.dispatch_key(&key("secondary-k"), &mut app), KeyDispatch::Unhandled);
    assert_eq!(app.log, ["ping"]);
}

#[test]
fn actions_bubble_from_focus_to_root_and_stop_unless_propagated() {
    let mut tree = tree(vec![KeyBinding::new("secondary-s", Save, None)]);
    tree.on_action(LIST, |_: &Save, cx| {
        cx.cx.log.push("list");
        cx.propagate();
    });
    tree.on_action(SIDEBAR, |_: &Save, cx| cx.cx.log.push("sidebar"));
    tree.on_action(ROOT, |_: &Save, cx| cx.cx.log.push("root"));
    let mut app = App::default();
    tree.focus(LIST);
    assert_eq!(tree.dispatch_key(&key("secondary-s"), &mut app), KeyDispatch::Handled);
    assert_eq!(app.log, ["list", "sidebar"]);
}

#[test]
fn the_deeper_context_wins_a_shared_keystroke() {
    let mut tree = tree(vec![
        KeyBinding::new("secondary-k", Save, Some("Workspace")),
        KeyBinding::new("secondary-k", Ping, Some("Editor")),
    ]);
    tree.on_action(ROOT, |_: &Save, cx| cx.cx.log.push("save"));
    tree.on_action(ROOT, |_: &Ping, cx| cx.cx.log.push("ping"));
    let mut app = App::default();
    tree.focus(EDITOR);
    tree.dispatch_key(&key("secondary-k"), &mut app);
    tree.focus(LIST);
    tree.dispatch_key(&key("secondary-k"), &mut app);
    assert_eq!(app.log, ["ping", "save"]);
}

#[test]
fn a_bound_action_nobody_handles_leaves_the_key_to_the_platform() {
    let mut tree = tree(vec![KeyBinding::new("a", Ping, None)]);
    let mut app = App::default();
    tree.focus(EDITOR);
    assert_eq!(tree.dispatch_key(&key("a"), &mut app), KeyDispatch::Unhandled);
}

#[test]
fn multi_stroke_bindings_hold_the_prefix() {
    let mut tree =
        tree(vec![KeyBinding::new("ctrl-x ctrl-s", Chord, None), KeyBinding::new("secondary-s", Save, None)]);
    tree.on_action(ROOT, |_: &Chord, cx| cx.cx.log.push("chord"));
    tree.on_action(ROOT, |_: &Save, cx| cx.cx.log.push("save"));
    let mut app = App::default();
    tree.focus(EDITOR);
    assert_eq!(tree.dispatch_key(&key("ctrl-x"), &mut app), KeyDispatch::Pending);
    assert_eq!(tree.dispatch_key(&key("ctrl-s"), &mut app), KeyDispatch::Handled);
    // A prefix followed by an unrelated binding drops the prefix and runs the binding.
    assert_eq!(tree.dispatch_key(&key("ctrl-x"), &mut app), KeyDispatch::Pending);
    assert_eq!(tree.dispatch_key(&key("secondary-s"), &mut app), KeyDispatch::Handled);
    assert_eq!(app.log, ["chord", "save"]);
}

#[test]
fn tab_walks_tree_order_and_wraps() {
    let mut tree = tree(vec![]);
    let mut app = App::default();
    assert_eq!(tree.focus_order(), [TOOLBAR_SEARCH, LIST, EDITOR]);
    let mut seen = Vec::new();
    for _ in 0..4 {
        assert_eq!(tree.dispatch_key(&key("tab"), &mut app), KeyDispatch::Handled);
        seen.push(tree.focused().unwrap());
    }
    assert_eq!(seen, [TOOLBAR_SEARCH, LIST, EDITOR, TOOLBAR_SEARCH]);
    tree.dispatch_key(&key("shift-tab"), &mut app);
    assert_eq!(tree.focused(), Some(EDITOR));
}

#[test]
fn tab_index_orders_before_tree_order_and_tab_stop_false_is_skipped() {
    let mut tree = tree(vec![]);
    tree.set_focusable(EDITOR, Some(Focusable { tab_index: -1, tab_stop: true }));
    tree.set_focusable(LIST, Some(Focusable { tab_index: 0, tab_stop: false }));
    assert_eq!(tree.focus_order(), [EDITOR, TOOLBAR_SEARCH]);
    // Out of traversal, still focusable directly.
    assert!(tree.focus(LIST));
}

#[test]
fn an_app_listener_can_take_over_tab() {
    let mut tree = tree(vec![]);
    tree.on_action(EDITOR, |_: &nd_ui::actions::FocusNext, cx| {
        cx.cx.log.push("indent");
    });
    let mut app = App::default();
    tree.focus(EDITOR);
    tree.dispatch_key(&key("tab"), &mut app);
    assert_eq!(app.log, ["indent"]);
    assert_eq!(tree.focused(), Some(EDITOR));
}

#[test]
fn listeners_request_focus_after_the_chain_returns() {
    let mut tree = tree(vec![KeyBinding::new("secondary-f", Ping, None)]);
    tree.on_action(ROOT, |_: &Ping, cx| cx.focus(TOOLBAR_SEARCH));
    let mut app = App::default();
    tree.focus(EDITOR);
    tree.take_focus_change();
    tree.dispatch_key(&key("secondary-f"), &mut app);
    assert_eq!(tree.focused(), Some(TOOLBAR_SEARCH));
    assert_eq!(tree.take_focus_change(), Some(Some(TOOLBAR_SEARCH)));
    assert_eq!(tree.take_focus_change(), None);
}

#[test]
fn platform_focus_is_mirrored_without_echo() {
    let mut tree = tree(vec![]);
    assert!(tree.platform_focused(Some(EDITOR)));
    assert_eq!(tree.focused(), Some(EDITOR));
    assert_eq!(tree.take_focus_change(), None);
    // A platform responder nd does not know keeps nd focus where it was.
    assert!(!tree.platform_focused(Some(NodeId(99))));
    assert_eq!(tree.focused(), Some(EDITOR));
}

#[test]
fn removing_the_focused_subtree_clears_focus() {
    let mut tree = tree(vec![]);
    tree.focus(LIST);
    tree.take_focus_change();
    tree.remove(SIDEBAR);
    assert_eq!(tree.focused(), None);
    assert_eq!(tree.take_focus_change(), Some(None));
    assert_eq!(tree.focus_order(), [TOOLBAR_SEARCH, EDITOR]);
}

#[test]
fn focus_moves_reach_the_backend_through_one_transaction() {
    use nd_ui::backend::NodeKind;
    use std::rc::Rc;

    let window = WindowId(1);
    let mut backend = TestBackend::new();
    let mut cache = MeasureCache::default();
    let mut tx = Transaction::new(window);
    for (node, parent, kind) in [
        (ROOT, None, NodeKind::Root),
        (TOOLBAR_SEARCH, Some(ROOT), NodeKind::View),
        (SIDEBAR, Some(ROOT), NodeKind::View),
        (LIST, Some(SIDEBAR), NodeKind::View),
        (EDITOR, Some(ROOT), NodeKind::View),
    ] {
        tx.push(Op::Create { node, kind, props: Rc::new(()) });
        if let Some(parent) = parent {
            tx.push(Op::Insert { parent, node, before: None });
        }
    }
    tx.commit(&mut backend, &mut cache);

    let mut tree = tree(vec![]);
    let mut app = App::default();
    for expected in [TOOLBAR_SEARCH, LIST] {
        tree.dispatch_key(&key("tab"), &mut app);
        let mut tx = Transaction::new(window);
        if let Some(node) = tree.take_focus_change() {
            tx.push(Op::Focus { node });
        }
        tx.commit(&mut backend, &mut cache);
        assert_eq!(backend.focused(), Some(expected));
    }
}

/// `key_context` and `on_action` set on elements reach the window's dispatch tree through the
/// reconciler, on `TestApp`.
mod elements {
    use nd_ui::test_app::TestApp;
    use nd_ui::*;

    use super::{KeyDispatch, Ping, Save, key};

    const WINDOW: Size = Size { width: 400., height: 300. };

    struct Workspace {
        label: &'static str,
        handles_ping: bool,
        log: Vec<String>,
    }

    impl Workspace {
        fn save(&mut self, _: &Save, _window: &mut Window, cx: &mut Context<Self>) {
            self.log.push(format!("save {}", self.label));
            cx.notify();
        }
    }

    impl Render for Workspace {
        fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let this = cx.weak_entity();
            v_flex()
                .key_context("Workspace")
                .on_action(cx.listener(Self::save))
                .when(self.handles_ping, |d| {
                    d.on_action(move |_: &Ping, _, cx| {
                        this.update(cx, |w, _| w.log.push("ping first".into()));
                        cx.propagate();
                    })
                    .on_action(cx.listener(|w, _: &Ping, _, _| w.log.push("ping second".into())))
                })
                .child(text("body"))
        }
    }

    fn open(cx: &mut TestApp, bindings: Vec<KeyBinding>) -> WindowHandle<Workspace> {
        cx.bind_keys(bindings);
        cx.open_window(WINDOW, |_, cx| cx.new(|_| Workspace { label: "a", handles_ping: true, log: Vec::new() }))
    }

    fn log(cx: &TestApp, window: &WindowHandle<Workspace>) -> Vec<String> {
        window.root().read(cx).log.clone()
    }

    #[test]
    fn a_binding_in_the_elements_key_context_reaches_its_on_action() {
        let mut cx = TestApp::new();
        let window = open(&mut cx, vec![KeyBinding::new("secondary-s", Save, Some("Workspace"))]);
        assert_eq!(cx.dispatch_keystroke(window.id(), &key("secondary-s")), KeyDispatch::Handled);
        assert_eq!(log(&cx, &window), ["save a"]);
    }

    #[test]
    fn a_binding_in_a_context_no_element_declares_stays_unhandled() {
        let mut cx = TestApp::new();
        let window = open(&mut cx, vec![KeyBinding::new("secondary-s", Save, Some("Editor"))]);
        assert_eq!(cx.dispatch_keystroke(window.id(), &key("secondary-s")), KeyDispatch::Unhandled);
        assert!(log(&cx, &window).is_empty());
    }

    #[test]
    fn listeners_are_replaced_on_every_render_without_a_backend_op() {
        let mut cx = TestApp::new();
        let window = open(&mut cx, vec![KeyBinding::new("secondary-s", Save, None)]);
        let ops = cx.backend().ops().count();
        window.root().update(&mut cx, |w, cx| {
            w.label = "b";
            cx.notify();
        });
        cx.run_until_idle();
        assert_eq!(cx.backend().ops().count(), ops, "swapping listeners sends nothing to the backend");
        assert!(cx.dispatch_action(window.id(), &Save));
        assert_eq!(log(&cx, &window), ["save b"]);
    }

    #[test]
    fn propagate_continues_to_the_next_listener_and_a_dropped_listener_is_gone() {
        let mut cx = TestApp::new();
        let window = open(&mut cx, Vec::new());
        assert!(cx.dispatch_action(window.id(), &Ping));
        assert_eq!(log(&cx, &window), ["ping first", "ping second"]);

        window.root().update(&mut cx, |w, cx| {
            w.handles_ping = false;
            cx.notify();
        });
        cx.run_until_idle();
        assert!(!cx.dispatch_action(window.id(), &Ping));
        assert_eq!(log(&cx, &window).len(), 2);
    }

    /// Three focusable rows; `second_first` gives the middle one a lower `tab_index`.
    struct Rows {
        second_first: bool,
        show_last: bool,
        log: Vec<String>,
    }

    impl Render for Rows {
        fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let row = |name: &'static str| div().id(name).focusable().child(text(name));
            v_flex()
                .key_context("Rows")
                .child(row("one"))
                .child(
                    row("two")
                        .key_context("Item")
                        .when(self.second_first, |d| d.tab_index(-1))
                        .on_action(cx.listener(|r, _: &Ping, _, _| r.log.push("item".into()))),
                )
                .when(self.show_last, |d| d.child(row("three")))
                .child(div().id("plain").child(text("not focusable")))
                .on_action(cx.listener(|r, _: &Ping, _, _| r.log.push("rows".into())))
        }
    }

    fn row_node(cx: &TestApp, name: &str) -> NodeId {
        let label = cx
            .backend()
            .find(|_, p| p.downcast_ref::<TextProps>().is_some_and(|t| t.text.as_ref() == name))
            .first()
            .copied()
            .expect("row label");
        cx.backend().parent(label).expect("row")
    }

    fn rows(cx: &mut TestApp) -> WindowHandle<Rows> {
        cx.bind_keys([KeyBinding::new("secondary-k", Ping, None)]);
        cx.open_window(WINDOW, |_, cx| cx.new(|_| Rows { second_first: false, show_last: true, log: Vec::new() }))
    }

    fn press(cx: &mut TestApp, window: &WindowHandle<Rows>, keystroke: &str) -> KeyDispatch {
        let result = cx.dispatch_keystroke(window.id(), &key(keystroke));
        cx.run_until_idle();
        result
    }

    #[test]
    fn tab_walks_focusable_elements_and_the_backend_follows() {
        let mut cx = TestApp::new();
        let window = rows(&mut cx);
        let [one, two, three] = ["one", "two", "three"].map(|n| row_node(&cx, n));
        assert_eq!(cx.focus_order(window.id()), [one, two, three], "only focusable elements, in tree order");

        for expected in [one, two, three, one] {
            assert_eq!(press(&mut cx, &window, "tab"), KeyDispatch::Handled);
            assert_eq!(cx.focused(window.id()), Some(expected));
            assert_eq!(cx.backend().focused(), Some(expected), "the focus op reached the backend");
        }
        assert_eq!(press(&mut cx, &window, "shift-tab"), KeyDispatch::Handled);
        assert_eq!(cx.backend().focused(), Some(three));
    }

    #[test]
    fn bindings_match_in_the_focused_elements_contexts() {
        let mut cx = TestApp::new();
        let window = rows(&mut cx);
        let two = row_node(&cx, "two");
        press(&mut cx, &window, "secondary-k");
        assert!(cx.focus(window.id(), two));
        press(&mut cx, &window, "secondary-k");
        assert_eq!(window.root().read(&cx).log, ["rows", "item"]);
        let plain = row_node(&cx, "not focusable");
        assert!(!cx.focus(window.id(), plain), "a plain div takes no focus");
    }

    #[test]
    fn tab_index_reorders_and_removing_the_focused_element_clears_focus() {
        let mut cx = TestApp::new();
        let window = rows(&mut cx);
        let [one, two, three] = ["one", "two", "three"].map(|n| row_node(&cx, n));
        window.root().update(&mut cx, |r, cx| {
            r.second_first = true;
            cx.notify();
        });
        cx.run_until_idle();
        assert_eq!(cx.focus_order(window.id()), [two, one, three]);

        cx.focus(window.id(), three);
        cx.run_until_idle();
        assert_eq!(cx.backend().focused(), Some(three));
        window.root().update(&mut cx, |r, cx| {
            r.show_last = false;
            cx.notify();
        });
        cx.run_until_idle();
        assert_eq!(cx.focused(window.id()), None);
        assert_eq!(cx.backend().focused(), None);
    }

    #[test]
    fn platform_focus_changes_are_mirrored_without_a_focus_op() {
        let mut cx = TestApp::new();
        let window = rows(&mut cx);
        let two = row_node(&cx, "two");
        cx.backend_mut().take_log();
        assert!(cx.platform_focused(window.id(), Some(two)));
        cx.run_until_idle();
        assert_eq!(cx.focused(window.id()), Some(two));
        assert!(!cx.backend().ops().any(|op| matches!(op, nd_ui::transaction::Op::Focus { .. })));
    }
}
