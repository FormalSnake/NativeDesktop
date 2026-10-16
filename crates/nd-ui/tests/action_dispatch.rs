//! An action dispatched from inside an event handler (a button's click) reaches listeners that
//! need the window, which that handler holds, and they can move focus.

use std::cell::Cell;
use std::rc::Rc;

use nd_gpui::gpui::{self, actions};
use nd_ui::semantics::SemanticAction;
use nd_ui::test_app::TestApp;
use nd_ui::*;

actions!(dispatch_test, [Go]);

struct View {
    ran: Rc<Cell<u32>>,
}

impl Render for View {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ran = self.ran.clone();
        v_flex()
            .on_action(cx.listener(move |_, _: &Go, window, cx| {
                ran.set(ran.get() + 1);
                window.focus_id(cx, "field");
            }))
            .child(button("go", "Go").on_click(|_: &ClickEvent, window, cx| {
                cx.dispatch_action(window.id(), &Go);
            }))
            .child(div().id("field").focusable())
    }
}

fn node(cx: &TestApp, test_id: &str) -> NodeId {
    cx.semantic_tree().nodes().find(|(_, n)| n.test_id.as_deref() == Some(test_id)).map(|(_, n)| n.id).expect(test_id)
}

#[test]
fn a_click_can_dispatch_an_action_that_needs_the_window_and_focus() {
    let ran = Rc::new(Cell::new(0));
    let mut cx = TestApp::new();
    let window = cx.open_window(Size { width: 300., height: 200. }, |_, cx| cx.new(|_| View { ran: ran.clone() }));
    let go = node(&cx, "go");
    cx.perform(go, &SemanticAction::Click).expect("click");
    cx.run_until_idle();
    assert_eq!(ran.get(), 1, "the listener ran once the click handler returned");
    assert_eq!(cx.focused(window.id()), Some(node(&cx, "field")), "and its focus request landed");
}
