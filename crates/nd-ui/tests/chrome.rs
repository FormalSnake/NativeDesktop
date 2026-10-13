//! Backend-neutral window chrome (ARCHITECTURE.md 5.4) on `TestBackend`: drag regions reach the
//! backend as props, `titlebar_insets()` lays out around the room the OS controls take.

use nd_gpui::gpui::px;
use nd_ui::backend::Props;
use nd_ui::test_app::TestApp;
use nd_ui::test_backend::{Recorded, TestBackend};
use nd_ui::*;

const WINDOW: Size = Size { width: 400., height: 300. };

struct TitleRow {
    renders: u32,
    seen: TitlebarInsets,
}

impl Render for TitleRow {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        self.renders += 1;
        self.seen = window.titlebar_insets();
        v_flex().size_full().child(
            drag_region()
                .id("title")
                .titlebar_insets()
                .flex()
                .flex_row()
                .px(px(8.))
                .child(div().id("leading").w(px(20.)).h(px(10.)))
                .child(div().flex_1())
                .child(div().id("trailing").w(px(20.)).h(px(10.))),
        )
    }
}

fn div_props(props: &Props) -> Option<&DivProps> {
    props.downcast_ref::<DivProps>()
}

fn node_with(cx: &TestApp, f: impl Fn(&DivProps) -> bool) -> NodeId {
    cx.backend().find(|_, p| div_props(p).is_some_and(&f))[0]
}

fn open(cx: &mut TestApp) -> WindowHandle<TitleRow> {
    cx.open_window(WINDOW, |_, cx| cx.new(|_| TitleRow { renders: 0, seen: TitlebarInsets::default() }))
}

mod gate_m2 {
    use super::*;

    #[test]
    fn a_drag_region_reaches_the_backend_as_a_prop() {
        let mut cx = TestApp::new();
        open(&mut cx);
        let title = node_with(&cx, |p| p.drag_region);
        assert!(div_props(cx.backend().props(title).unwrap()).unwrap().titlebar_insets);
        assert_eq!(cx.backend().find(|_, p| div_props(p).is_some_and(|p| p.drag_region)).len(), 1);
    }

    #[test]
    fn titlebar_insets_pad_the_row_and_reach_the_view() {
        let mut cx = TestApp::new();
        let handle = open(&mut cx);
        let title = node_with(&cx, |p| p.drag_region);
        let children = cx.backend().children(title).to_vec();
        let (leading, trailing) = (children[0], children[2]);
        assert_eq!(cx.backend().frame(leading).unwrap().origin.x, 8.);
        assert_eq!(cx.backend().frame(trailing).unwrap().origin.x, 400. - 8. - 20.);

        let insets = TitlebarInsets { leading: 70., trailing: 0., height: 28. };
        cx.set_titlebar_insets(handle.id(), insets);
        cx.run_until_idle();
        assert_eq!(cx.backend().frame(leading).unwrap().origin.x, 78.);
        assert_eq!(cx.backend().frame(title).unwrap().size.height, 28.);
        assert_eq!(handle.root().read(&cx).seen, insets);
        assert_eq!(handle.root().read(&cx).renders, 2);

        // Caption buttons on the trailing side, as on Windows.
        cx.set_titlebar_insets(handle.id(), TitlebarInsets { leading: 0., trailing: 138., height: 32. });
        cx.run_until_idle();
        assert_eq!(cx.backend().frame(leading).unwrap().origin.x, 8.);
        assert_eq!(cx.backend().frame(trailing).unwrap().origin.x, 400. - 8. - 138. - 20.);
        assert_eq!(cx.backend().frame(title).unwrap().size.height, 32.);
    }

    #[test]
    fn unchanged_insets_render_nothing() {
        let mut cx = TestApp::new();
        let handle = open(&mut cx);
        cx.backend_mut().take_log();
        cx.set_titlebar_insets(handle.id(), TitlebarInsets::default());
        cx.run_until_idle();
        assert_eq!(handle.root().read(&cx).renders, 1);
        assert_eq!(cx.backend().commits(), 0);
    }

    #[test]
    fn a_backend_that_lays_out_itself_is_never_asked_to_measure() {
        let mut cx = TestApp::with_backend(TestBackend::new().without_frames());
        let handle =
            cx.open_window(WINDOW, |_, cx| cx.new(|_| TitleRow { renders: 0, seen: TitlebarInsets::default() }));
        cx.resize_window(handle.id(), Size { width: 500., height: 300. });
        cx.run_until_idle();
        let measured = cx.backend().log().iter().any(|r| matches!(r, Recorded::Measure(..)));
        assert!(!measured, "frameless backends lay out themselves");
    }
}
