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
