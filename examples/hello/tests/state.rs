//! Tier 1: a model entity, a view that observes it, and an update from outside the view.

use nd::prelude::*;
use nd::test::{Size, TestApp, text as window_text};

// docs:begin observe
struct Tally {
    total: u32,
}

struct Readout {
    shown: u32,
}

impl Readout {
    fn new(tally: &Entity<Tally>, cx: &mut Context<Self>) -> Self {
        cx.observe(tally, |this, tally, cx| {
            this.shown = tally.read(cx).total;
            cx.notify();
        })
        .detach();
        Self { shown: tally.read(cx).total }
    }
}

impl Render for Readout {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        text(format!("total {}", self.shown))
    }
}
// docs:end observe

#[nd::test]
fn a_view_follows_the_model_it_observes(cx: &mut TestApp) {
    // docs:begin observe-test
    let tally = cx.new(|_| Tally { total: 0 });
    let model = tally.clone();
    let window = cx.open_window(Size { width: 400., height: 200. }, move |_, cx| cx.new(|cx| Readout::new(&model, cx)));
    assert!(window_text(cx, window.id()).contains("total 0"));

    tally.update(cx, |tally, cx| {
        tally.total += 3;
        cx.notify();
    });
    assert!(window_text(cx, window.id()).contains("total 3"));
    // docs:end observe-test
}
