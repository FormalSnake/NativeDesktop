//! The appearance audit must catch what shipped on e1504g: text whose colour does not follow
//! the active theme. A literal colour is not a token in any variant and goes unreadable in
//! dark.

use nd::prelude::*;

struct Stale;

impl Render for Stale {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        v_flex().size_full().child(text("follows the theme")).child(text("stuck on light").text_color(rgb(0x1a1a1a)))
    }
}

fn main() {
    nd::audit::Audit::new("catches", || Application::new(nd::manifest!()))
        .window("stale", |cx| {
            cx.open_window(WindowOptions::main("stale").title("Stale"), |_, cx| cx.new(|_| Stale)).id()
        })
        .must_fail()
        .run()
}
