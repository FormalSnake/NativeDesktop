//! Overlay rules on `TestBackend` (ARCHITECTURE.md 6.3): an overlay takes no room in its
//! anchor's layout, Escape closes the innermost overlay that takes input (through the dispatch
//! tree's focus scopes), the app losing activation closes light-dismiss overlays only, and closing
//! hands focus back.

use std::cell::RefCell;
use std::rc::Rc;

use nd_gpui::gpui::px;
use nd_ui::backend::Props;
use nd_ui::dispatch::KeyDispatch;
use nd_ui::test_app::TestApp;
use nd_ui::test_backend::TestBackend;
use nd_ui::*;

const WINDOW: Size = Size { width: 600., height: 400. };

fn text_props(props: &Props) -> Option<&TextProps> {
    props.downcast_ref::<TextProps>()
}

fn backend() -> TestBackend {
    let mut backend = TestBackend::new();
    backend.on_measure(|_, props, _| match text_props(props) {
        Some(t) => Size { width: 8. * t.text.len() as f32, height: 20. },
        None => Size { width: 80., height: 24. },
    });
    backend
}

fn find_text(cx: &TestApp, s: &str) -> Option<NodeId> {
    cx.backend().find(|_, p| text_props(p).is_some_and(|t| t.text.as_ref() == s)).first().copied()
}

fn overlay_node(cx: &TestApp, kind: OverlayKind) -> Option<NodeId> {
    cx.backend().find(|_, p| p.downcast_ref::<OverlayProps>().is_some_and(|o| o.kind == kind)).first().copied()
}

#[derive(Default)]
struct Screen {
    popover: bool,
    nested: bool,
    panel: bool,
    tooltip: bool,
    log: Rc<RefCell<Vec<String>>>,
}

impl Render for Screen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .child(div().id("field").focusable().w(px(200.)).h(px(20.)))
            .child(
                div()
                    .id("anchor")
                    .w(px(120.))
                    .h(px(30.))
                    .when(self.popover, |d| {
                        d.child(
                            popover("menu")
                                .p(px(8.))
                                .child(text("popover body"))
                                .child(div().id("inner").focusable().w(px(40.)).h(px(10.)).when(self.nested, |d| {
                                    d.child(popover("nested").child(text("nested body")).on_dismiss(cx.listener(
                                        |this, e: &Dismissed, _, cx| {
                                            this.log.borrow_mut().push(format!("nested {:?}", e.reason));
                                            this.nested = false;
                                            cx.notify();
                                        },
                                    )))
                                }))
                                .on_dismiss(cx.listener(|this, e: &Dismissed, _, cx| {
                                    this.log.borrow_mut().push(format!("popover {:?}", e.reason));
                                    this.popover = false;
                                    cx.notify();
                                })),
                        )
                    })
                    .when(self.tooltip, |d| d.child(tooltip().child(text("hint")))),
            )
            .child(text("below"))
            .when(self.panel, |d| {
                d.child(panel("palette").child(div().id("query").focusable().w(px(300.)).h(px(24.))).on_dismiss(
                    cx.listener(|this, e: &Dismissed, _, cx| {
                        this.log.borrow_mut().push(format!("panel {:?}", e.reason));
                        this.panel = false;
                        cx.notify();
                    }),
                ))
            })
    }
}

fn open() -> (TestApp, WindowHandle<Screen>) {
    let mut cx = TestApp::with_backend(backend());
    let window = cx.open_window(WINDOW, |_, cx| cx.new(|_| Screen::default()));
    (cx, window)
}

fn set(cx: &mut TestApp, window: &WindowHandle<Screen>, f: impl FnOnce(&mut Screen)) {
    window.root().clone().update(cx, |s, cx| {
        f(s);
        cx.notify();
    });
    cx.run_until_idle();
}

fn log(cx: &TestApp, window: &WindowHandle<Screen>) -> Vec<String> {
    window.root().read(cx).log.borrow().clone()
}

fn escape() -> Keystroke {
    Keystroke::parse("escape").unwrap()
}

mod gate_m4_overlays {
    use super::*;

    #[test]
    fn overlay_takes_no_room_in_its_anchor() {
        let (mut cx, window) = open();
        let below = find_text(&cx, "below").unwrap();
        let before = cx.backend().frame(below).unwrap();
        set(&mut cx, &window, |s| s.popover = true);
        assert_eq!(cx.backend().frame(below), Some(before), "the popover moved its anchor's siblings");
        let popover = overlay_node(&cx, OverlayKind::Popover).expect("popover node");
        let frame = cx.backend().frame(popover).unwrap();
        // Sized by its content: the 12-character text plus 8 points of padding a side.
        assert_eq!(frame.size.width, 8. * 12. + 16.);
        let anchor = cx.backend().parent(popover).unwrap();
        assert_eq!(cx.backend().frame(anchor).unwrap().size.height, 30.);
    }

    #[test]
    fn escape_closes_the_innermost_overlay() {
        let (mut cx, window) = open();
        let id = window.id();
        set(&mut cx, &window, |s| {
            s.popover = true;
            s.nested = true;
            s.tooltip = true;
        });
        assert_eq!(cx.dispatch_keystroke(id, &escape()), KeyDispatch::Handled);
        cx.run_until_idle();
        assert_eq!(log(&cx, &window), ["nested Escape"]);
        cx.dispatch_keystroke(id, &escape());
        cx.run_until_idle();
        assert_eq!(log(&cx, &window)[1..], ["popover Escape"], "the tooltip is passive, so the popover is next");
        assert!(overlay_node(&cx, OverlayKind::Popover).is_none());
        assert_eq!(cx.dispatch_keystroke(id, &escape()), KeyDispatch::Unhandled, "a passive tooltip takes no Escape");
    }

    #[test]
    fn deactivation_closes_light_dismiss_overlays_only() {
        let (mut cx, window) = open();
        set(&mut cx, &window, |s| {
            s.popover = true;
            s.panel = true;
            s.tooltip = true;
        });
        cx.app_deactivated();
        cx.run_until_idle();
        assert_eq!(log(&cx, &window), ["popover Deactivated"]);
        assert!(overlay_node(&cx, OverlayKind::Panel).is_some(), "a modal panel survives deactivation");
        assert!(overlay_node(&cx, OverlayKind::Tooltip).is_some(), "a passive tooltip is not asked");
    }

    #[test]
    fn closing_hands_focus_back() {
        let (mut cx, window) = open();
        let id = window.id();
        let field = cx.semantic_window(id).and_then(|w| by_test_id(&w.root, "field")).unwrap();
        assert!(cx.focus(id, field));
        set(&mut cx, &window, |s| s.panel = true);
        let query = cx.semantic_window(id).and_then(|w| by_test_id(&w.root, "query")).unwrap();
        assert!(cx.focus(id, query));
        cx.run_until_idle();
        assert_eq!(cx.backend().focused(), Some(query));
        cx.dispatch_keystroke(id, &escape());
        cx.run_until_idle();
        assert_eq!(log(&cx, &window), ["panel Escape"]);
        assert_eq!(cx.focused(id), Some(field), "focus returns to the field that had it");
        assert_eq!(cx.backend().focused(), Some(field), "and the backend follows");
    }

    fn by_test_id(node: &SemanticNode, test_id: &str) -> Option<NodeId> {
        if node.test_id.as_deref() == Some(test_id) {
            return Some(node.id);
        }
        node.children.iter().find_map(|c| by_test_id(c, test_id))
    }
}
