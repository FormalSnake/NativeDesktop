//! Focus and keyboard rules (ARCHITECTURE.md 6.2) and the gesture element API (6.1), tier 1.

mod gate_m4 {
    use std::cell::RefCell;
    use std::rc::Rc;

    use nd_ui::dispatch::{DismissRequested, FocusScope};
    use nd_ui::gesture::GestureEvent;
    use nd_ui::test_app::TestApp;
    use nd_ui::*;

    const WINDOW: Size = Size { width: 400., height: 300. };

    fn key(source: &str) -> Keystroke {
        Keystroke::parse(source).expect("keystroke")
    }

    fn node(cx: &TestApp, test_id: &str) -> NodeId {
        let tree = cx.semantic_tree();
        tree.nodes().find(|(_, n)| n.test_id.as_deref() == Some(test_id)).map(|(_, n)| n.id).expect(test_id)
    }

    #[derive(Default)]
    struct Log(Rc<RefCell<Vec<String>>>);

    impl Log {
        fn push(&self, s: impl Into<String>) {
            self.0.borrow_mut().push(s.into());
        }
        fn take(&self) -> Vec<String> {
            std::mem::take(&mut self.0.borrow_mut())
        }
    }

    impl Clone for Log {
        fn clone(&self) -> Self {
            Log(self.0.clone())
        }
    }

    /// A form with two fields, a notes field that keeps Return, a default Save button, and a
    /// popover that mounts while `open`.
    struct Form {
        log: Log,
        open: bool,
    }

    impl Render for Form {
        fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let log = self.log.clone();
            let save_log = self.log.clone();
            v_flex()
                .child(div().id("name").focusable())
                .child(div().id("notes").focusable().takes_return())
                .child(button("cancel", "Cancel"))
                .child(
                    button("save", "Save")
                        .variant(ButtonVariant::Primary)
                        .on_click(move |_: &ClickEvent, _, _| save_log.push("save")),
                )
                .when(self.open, |col| {
                    col.child(
                        div()
                            .id("popover")
                            .focus_scope(FocusScope { trap: true, dismiss_on_escape: true })
                            .on(cx.listener(move |this: &mut Form, _: &DismissRequested, _, cx| {
                                log.push("dismiss");
                                this.open = false;
                                cx.notify();
                            }))
                            .child(div().id("search").focusable())
                            .child(div().id("result").focusable()),
                    )
                })
        }
    }

    fn form() -> (TestApp, WindowId, Entity<Form>, Log) {
        let log = Log::default();
        let mut cx = TestApp::new();
        let l = log.clone();
        let handle = cx.open_window(WINDOW, move |_, cx| cx.new(|_| Form { log: l, open: false }));
        (cx, handle.id(), handle.root().clone(), log)
    }

    fn press(cx: &mut TestApp, window: WindowId, keys: &str) -> dispatch::KeyDispatch {
        let result = cx.dispatch_keystroke(window, &key(keys));
        cx.run_until_idle();
        result
    }

    #[test]
    fn return_activates_the_default_button_unless_the_field_keeps_it() {
        let (mut cx, window, _, log) = form();
        let (name, notes) = (node(&cx, "name"), node(&cx, "notes"));
        cx.focus(window, name);
        assert_eq!(press(&mut cx, window, "enter"), dispatch::KeyDispatch::Handled);
        assert_eq!(log.take(), ["save"]);

        cx.focus(window, notes);
        assert_eq!(press(&mut cx, window, "enter"), dispatch::KeyDispatch::Unhandled, "the text view gets it");
        assert!(log.take().is_empty());
    }

    #[test]
    fn focus_loss_never_activates() {
        let (mut cx, window, _, log) = form();
        let name = node(&cx, "name");
        cx.focus(window, name);
        cx.blur(window);
        cx.run_until_idle();
        assert!(log.take().is_empty());
    }

    #[test]
    fn an_overlay_takes_focus_traps_tab_closes_on_escape_and_restores_focus() {
        let (mut cx, window, form, log) = form();
        let name = node(&cx, "name");
        cx.focus(window, name);
        cx.run_until_idle();

        form.update(&mut cx, |f, cx| {
            f.open = true;
            cx.notify();
        });
        cx.run_until_idle();
        let (search, result) = (node(&cx, "search"), node(&cx, "result"));
        assert_eq!(cx.focused(window), Some(search), "a modal scope focuses its first stop");
        assert_eq!(cx.backend().focused(), Some(search));
        assert_eq!(cx.focus_order(window), [search, result]);
        press(&mut cx, window, "tab");
        press(&mut cx, window, "tab");
        assert_eq!(cx.focused(window), Some(search), "Tab cycles inside the overlay");

        // Return inside the overlay finds no default button there: nothing is saved.
        press(&mut cx, window, "enter");
        assert!(log.take().is_empty());

        assert_eq!(press(&mut cx, window, "escape"), dispatch::KeyDispatch::Handled);
        assert_eq!(log.take(), ["dismiss"]);
        assert_eq!(cx.focused(window), Some(name), "focus is back where it was");
        assert_eq!(cx.backend().focused(), Some(name));
        assert_eq!(press(&mut cx, window, "escape"), dispatch::KeyDispatch::Unhandled, "nothing left to close");
    }

    #[test]
    fn focus_rings_show_for_keys_and_hide_for_pointers() {
        let (mut cx, window, _, _) = form();
        let name = node(&cx, "name");
        cx.focus(window, name);
        assert!(!cx.focus_visible(window), "programmatic focus after launch shows no ring");
        press(&mut cx, window, "tab");
        assert!(cx.focus_visible(window));
        cx.pointer_down(window);
        assert!(!cx.focus_visible(window));
    }

    // Gestures through the element API.

    struct Canvas {
        log: Log,
    }

    impl Render for Canvas {
        fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let (a, b, c) = (self.log.clone(), self.log.clone(), self.log.clone());
            div()
                .id("canvas")
                .size_full()
                .overflow_y_scroll()
                .on_pinch(cx.listener(move |_: &mut Canvas, e: &PinchEvent, _, cx| {
                    a.push(format!("pinch {}", e.scale));
                    if e.scale > 1. {
                        cx.haptic(Haptic::Alignment);
                    }
                }))
                .on_tap(move |e: &TapEvent, _, _| b.push(format!("tap {}", e.count)))
                .on_double_tap(move |_: &DoubleTapEvent, _, _| c.push("double"))
        }
    }

    #[test]
    fn gesture_modifiers_mark_the_node_and_deliver_typed_events() {
        let log = Log::default();
        let mut cx = TestApp::new();
        let l = log.clone();
        let handle = cx.open_window(WINDOW, move |_, cx| cx.new(|_| Canvas { log: l }));
        let canvas = node(&cx, "canvas");
        let props = cx.backend().props(canvas).unwrap().downcast_ref::<DivProps>().unwrap().clone();
        assert!(props.gestures.pinch && props.gestures.tap && props.gestures.double_tap);
        assert!(props.gestures.scroll.vertical && !props.gestures.scroll.horizontal, "overflow_y_scroll scrolls");

        let emit = cx.events().emitter(canvas);
        let pinch =
            GestureEvent::Pinch(PinchEvent { position: Point::default(), scale: 1.5, phase: GesturePhase::Changed });
        events::emit_gesture(&emit, pinch);
        events::emit_gesture(&emit, GestureEvent::Tap(TapEvent { position: Point::default(), count: 1 }));
        events::emit_gesture(&emit, GestureEvent::Tap(TapEvent { position: Point::default(), count: 2 }));
        cx.run_until_idle();
        assert_eq!(log.take(), ["pinch 1.5", "tap 1", "double"]);
        assert_eq!(cx.backend().haptics(), [Haptic::Alignment]);
        let _ = handle;
    }

    use nd_gpui::gpui;
    gpui::actions!(focus_tests, [Compose]);

    /// A toolbar-style button whose click dispatches a command handled at the root.
    struct Dispatcher {
        log: Log,
    }

    impl Render for Dispatcher {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            let log = self.log.clone();
            v_flex().on_action(move |_: &Compose, _, _| log.push("compose")).child(
                button("compose", "Compose").on_click(|_: &ClickEvent, window, cx| {
                    let window = window.id();
                    cx.dispatch_action(window, &Compose);
                }),
            )
        }
    }

    #[test]
    fn a_click_handler_can_dispatch_an_action_to_an_ancestor() {
        let log = Log::default();
        let mut cx = TestApp::new();
        let l = log.clone();
        let handle = cx.open_window(WINDOW, move |_, cx| cx.new(|_| Dispatcher { log: l }));
        let compose = node(&cx, "compose");
        cx.emit(compose, ClickEvent);
        cx.run_until_idle();
        assert_eq!(log.take(), ["compose"], "the window was lent to the click handler, so the action waits for it");
        let _ = handle;
    }

    /// A reply banner: clicking Reply mounts a field and focuses it by id in the same handler.
    struct Reply {
        open: bool,
    }

    impl Render for Reply {
        fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let this = cx.weak_entity();
            v_flex()
                .child(button("reply", "Reply").on_click(move |_: &ClickEvent, window, cx| {
                    this.update(cx, |this, cx| {
                        this.open = true;
                        cx.notify();
                    });
                    window.focus("composer");
                }))
                .when(self.open, |d| d.child(div().id("composer").child(div().id("field").focusable())))
                .child(div().id("plain"))
                .child(button("nowhere", "Nowhere").on_click(|_: &ClickEvent, window, _| window.focus("plain")))
        }
    }

    #[test]
    fn focus_by_id_reaches_an_element_mounted_in_the_same_handler() {
        let mut cx = TestApp::new();
        let handle = cx.open_window(WINDOW, |_, cx| cx.new(|_| Reply { open: false }));
        cx.emit(node(&cx, "reply"), ClickEvent);
        cx.run_until_idle();
        let field = node(&cx, "field");
        assert_eq!(cx.focused(handle.id()), Some(field), "the container's first focusable descendant takes focus");
        assert_eq!(cx.backend().focused(), Some(field), "the backend follows");
    }

    #[test]
    fn an_id_nothing_focusable_carries_leaves_focus_alone() {
        let mut cx = TestApp::new();
        let handle = cx.open_window(WINDOW, |_, cx| cx.new(|_| Reply { open: true }));
        let field = node(&cx, "field");
        assert!(cx.focus(handle.id(), field));
        cx.run_until_idle();
        cx.emit(node(&cx, "nowhere"), ClickEvent);
        cx.run_until_idle();
        assert_eq!(cx.focused(handle.id()), Some(field));
    }

    struct Counter(u32);

    #[test]
    fn globals_update_in_place() {
        let mut cx = TestApp::new();
        cx.set_global(Counter(1));
        cx.global_mut::<Counter>().0 += 1;
        let seen = cx.update_global(|c: &mut Counter, cx| {
            c.0 += 1;
            cx.try_global::<Counter>().is_none()
        });
        assert!(seen, "the global is out of the app while update_global runs");
        assert_eq!(cx.global::<Counter>().0, 3);
    }
}
