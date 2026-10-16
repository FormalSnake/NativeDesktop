//! Rich text, tier 1: runs reach the backend on the text node, semantics read the plain text,
//! and a link click reaches the handler (which stops the system opening it).

use std::cell::RefCell;
use std::rc::Rc;

use nd_ui::test_app::TestApp;
use nd_ui::*;

const WINDOW: Size = Size { width: 400., height: 300. };

struct Message {
    clicked: Rc<RefCell<Vec<String>>>,
}

impl Render for Message {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let clicked = self.clicked.clone();
        v_flex()
            .child(
                rich_text([
                    span("Shipped "),
                    span("v2").bold(),
                    span(", see "),
                    span("notes").link("https://example.com/notes"),
                    span(" @ben").mention(),
                    span(" big").scale(2.),
                ])
                .id("body")
                .on_link(move |e: &LinkClicked, _, _| clicked.borrow_mut().push(e.link.to_string())),
            )
            .child(rich_text([span("plain")]).id("plain"))
    }
}

#[test]
fn runs_reach_the_backend_and_links_reach_the_handler() {
    let clicked = Rc::new(RefCell::new(Vec::new()));
    let mut cx = TestApp::new();
    let c = clicked.clone();
    cx.open_window(WINDOW, move |_, cx| cx.new(|_| Message { clicked: c }));
    let tree = cx.semantic_tree();
    let body = tree.nodes().find(|(_, n)| n.test_id.as_deref() == Some("body")).map(|(_, n)| n.clone()).unwrap();
    assert_eq!(body.name, "Shipped v2, see notes @ben big", "semantics read the plain text");
    let props = cx.backend().props(body.id).and_then(|p| p.downcast_ref::<TextProps>()).cloned().unwrap();
    let rich = props.rich.expect("runs reach the backend");
    assert_eq!(rich.spans().len(), 6);
    assert!(rich.spans()[1].style.bold);
    assert_eq!(rich.uniform_scale(), None, "one run is bigger");
    assert!(!props.opens_links, "the app handles its links");

    let plain = tree.nodes().find(|(_, n)| n.test_id.as_deref() == Some("plain")).map(|(_, n)| n.id).unwrap();
    let plain = cx.backend().props(plain).and_then(|p| p.downcast_ref::<TextProps>()).cloned().unwrap();
    assert!(plain.rich.is_none(), "runs that change nothing are plain text");
    assert!(plain.opens_links);

    cx.emit(body.id, LinkClicked { link: "https://example.com/notes".into() });
    cx.run_until_idle();
    assert_eq!(*clicked.borrow(), ["https://example.com/notes"]);
}
