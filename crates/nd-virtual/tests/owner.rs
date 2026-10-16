//! A `ListState` driven the way views often drive it: keys handed over from `render`.

use std::cell::Cell;
use std::rc::Rc;

use nd_ui::prelude::*;
use nd_ui::test_app::TestApp;
use nd_virtual::{ListState, list};

struct Owner {
    state: ListState<u64>,
    renders: Rc<Cell<u32>>,
}

impl Render for Owner {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.renders.set(self.renders.get() + 1);
        self.state.set_items_versioned((0..3).map(|k| (k, 0)), cx);
        list("rows", &self.state, |key: u64, _, _| div().id(format!("row-{key}")).h(px(10.)))
    }
}

#[test]
fn the_same_keys_from_render_settle_instead_of_rendering_again() {
    let renders = Rc::new(Cell::new(0));
    let mut cx = TestApp::new();
    let r = renders.clone();
    let _handle = cx.open_window(nd_ui::Size { width: 300., height: 200. }, move |_, cx| {
        cx.new(|cx| Owner { state: ListState::new(cx), renders: r })
    });
    // Before the fix every hand-over bumped the model's generation and re-rendered the owner:
    // `run_until_idle` panicked after 100 frames.
    cx.run_until_idle();
    assert!(renders.get() <= 3, "the owner rendered {} times", renders.get());
}
