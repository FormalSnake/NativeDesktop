//! The semantic tree and semantic actions on `TestBackend` (10.1).

use nd_ui::backend::Props;
use nd_ui::semantics::{ActionError, Probe, SemanticAction};
use nd_ui::test_app::TestApp;
use nd_ui::test_backend::TestBackend;
use nd_ui::*;

const WINDOW: Size = Size { width: 400., height: 300. };

/// Text measures 8 points per character by 20 high; widgets 100 by 24.
fn measuring_backend() -> TestBackend {
    let mut backend = TestBackend::new();
    backend.on_measure(|_, props: &Props, _| match props.downcast_ref::<TextProps>() {
        Some(t) => Size { width: 8. * t.text.len() as f32, height: 20. },
        None => Size { width: 100., height: 24. },
    });
    backend
}

struct Counter {
    count: u32,
}

impl Render for Counter {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .p_4()
            .child(button("add", "Add one").on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.count += 1;
                cx.notify();
            })))
            .child(div().id("result").child(text(self.count.to_string())))
    }
}

fn counter() -> TestApp {
    let mut cx = TestApp::with_backend(measuring_backend());
    cx.open_window(WINDOW, |_, cx| cx.new(|_| Counter { count: 0 }));
    cx
}

#[test]
fn tree_has_roles_names_test_ids_and_window_bounds() {
    let cx = counter();
    let tree = cx.semantic_tree();
    assert_eq!(tree.windows.len(), 1);
    let window = &tree.windows[0];
    assert!(window.key && window.visible, "the only window is key without an OS");
    assert_eq!(window.root.role, Role::Window);
    assert_eq!(window.root.bounds, Some(Rect::new(0., 0., 400., 300.)));

    let (_, add) = tree.nodes().find(|(_, n)| n.test_id.as_deref() == Some("add")).expect("button");
    assert_eq!((add.role, add.name.as_str(), add.kind.as_str()), (Role::Button, "Add one", "Button"));
    // p_4 is 16 points of padding; the column stretches its children across.
    assert_eq!(add.bounds, Some(Rect::new(16., 16., 368., 24.)));
    assert!(add.visible && add.enabled && !add.focused);

    let (_, result) = tree.nodes().find(|(_, n)| n.test_id.as_deref() == Some("result")).expect("result");
    assert_eq!(result.role, Role::Group);
    let label = &result.children[0];
    assert_eq!((label.role, label.name.as_str()), (Role::Text, "0"));
    // Frames accumulate through the result div to window coordinates.
    assert_eq!(label.bounds.unwrap().origin, Point { x: 16., y: 40. });
    assert_eq!(result.subtree_text(), "0");
}

#[test]
fn click_goes_through_the_backend_and_the_tree_follows() {
    let mut cx = counter();
    let add = cx.semantic_tree().nodes().find(|(_, n)| n.role == Role::Button).unwrap().1.id;
    cx.perform(add, &SemanticAction::Click).unwrap();
    cx.run_until_idle();
    assert_eq!(cx.backend().performed(), &[(add, SemanticAction::Click)]);
    let tree = cx.semantic_tree();
    assert!(tree.nodes().any(|(_, n)| n.role == Role::Text && n.name == "1"), "{}", tree.snapshot());
}

#[test]
fn probes_override_and_disabled_controls_refuse() {
    let mut cx = counter();
    let add = cx.semantic_tree().nodes().find(|(_, n)| n.role == Role::Button).unwrap().1.id;
    cx.backend_mut().set_probe(add, Probe { enabled: Some(false), value: Some("v".into()), ..Probe::default() });
    let tree = cx.semantic_tree();
    let node = tree.find(add).unwrap().1;
    assert!(!node.enabled);
    assert_eq!(node.value.as_deref(), Some("v"));
    assert!(matches!(cx.perform(add, &SemanticAction::Click), Err(ActionError::NotActionable(_))));
    assert!(matches!(cx.perform(NodeId(9999), &SemanticAction::Click), Err(ActionError::NotActionable(_))));
}

#[test]
fn focus_refuses_a_node_nd_cannot_focus() {
    // The button is not `focusable`, and the test backend has no native first responder to fall
    // back to.
    let mut cx = counter();
    let add = cx.semantic_tree().nodes().find(|(_, n)| n.role == Role::Button).unwrap().1.id;
    assert!(matches!(cx.perform(add, &SemanticAction::Focus), Err(ActionError::NotActionable(_))));
    assert!(!cx.semantic_tree().find(add).unwrap().1.focused);
}

#[test]
fn snapshot_is_one_line_per_node() {
    let cx = counter();
    let snapshot = cx.semantic_tree().snapshot();
    assert!(snapshot.starts_with("window \"\" [id=1] [key]\n"), "{snapshot}");
    assert!(snapshot.contains("- button \"Add one\" [ref=e"), "{snapshot}");
    assert!(snapshot.contains("[testid=add]"), "{snapshot}");
}

#[test]
fn focus_on_a_focusable_element_moves_nd_focus_and_the_tree_reports_it() {
    struct Form;
    impl Render for Form {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            v_flex().child(button("save", "Save").focusable()).child(div().id("field").focusable())
        }
    }
    let mut cx = TestApp::with_backend(measuring_backend());
    cx.open_window(WINDOW, |_, cx| cx.new(|_| Form));
    let field = cx.semantic_tree().nodes().find(|(_, n)| n.test_id.as_deref() == Some("field")).unwrap().1.id;
    cx.perform(field, &SemanticAction::Focus).unwrap();
    cx.run_until_idle();
    assert!(cx.semantic_tree().find(field).unwrap().1.focused);
    assert_eq!(cx.backend().focused(), Some(field), "the focus op reached the backend");
    assert!(cx.backend().performed().is_empty(), "nd focus needs no backend action");
}

#[test]
fn a_node_the_backend_clips_out_is_not_visible_whatever_its_frame() {
    let mut cx = counter();
    let add = cx.semantic_tree().nodes().find(|(_, n)| n.role == Role::Button).unwrap().1.id;
    assert!(cx.semantic_tree().find(add).unwrap().1.visible);
    cx.backend_mut().set_probe(add, Probe { visible: Some(false), ..Probe::default() });
    let tree = cx.semantic_tree();
    let node = tree.find(add).unwrap().1;
    assert!(node.bounds.is_some_and(|b| b.size.width > 0.), "it keeps its frame");
    assert!(!node.visible, "a row scrolled out of its list or a collapsed pane");
}
