//! The tier 1 fixture shared by the tier 1 tests and the M1 gate: a counter with two buttons.

use nd_gpui::gpui::px;
use nd_test::TestHarness;
use nd_ui::backend::Props;
use nd_ui::test_app::TestApp;
use nd_ui::test_backend::TestBackend;
use nd_ui::*;

pub struct Counter {
    pub count: u32,
}

impl Render for Counter {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .p_4()
            .gap_2()
            .child(
                h_flex()
                    .id("toolbar")
                    .child(button("add", "Add one").on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.count += 1;
                        cx.notify();
                    })))
                    .child(button("reset", "Reset").on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.count = 0;
                        cx.notify();
                    }))),
            )
            .child(div().id("result").child(text(self.count.to_string())).child(text("clicks")))
            .when(self.count >= 2, |d| d.child(text("Many").id("many")))
            // A zero-height row: present in the tree, never visible itself. Its text still is,
            // since containers do not clip.
            .child(div().id("collapsed").h(px(0.)).child(text("tucked away")))
    }
}

/// Text measures 8 points per character by 20 high; widgets 80 by 24.
pub fn harness() -> TestHarness {
    let mut backend = TestBackend::new();
    backend.on_measure(|_, props: &Props, _| match props.downcast_ref::<TextProps>() {
        Some(t) => Size { width: 8. * t.text.len() as f32, height: 20. },
        None => Size { width: 80., height: 24. },
    });
    let mut app = TestApp::with_backend(backend);
    app.open_window(Size { width: 400., height: 300. }, |_, cx| cx.new(|_| Counter { count: 0 }));
    TestHarness::new(app)
}
