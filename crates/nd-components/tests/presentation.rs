//! Tier 1 behaviour of a presentation on `TestBackend`: what each frame draws while it closes.

mod common;

use common::*;
use nd_components::*;
use nd_ui::prelude::*;
use nd_ui::{DivProps, Size};

const FRAME: f64 = 1. / 60.;

struct Shown {
    expanded: Presentation,
}

fn view(s: &Shown, _cx: &mut Context<Host<Shown>>) -> AnyElement {
    let page = div().size_full().child(div().id("thumb").absolute().left(px(10.)).top(px(580.)).size(px(48.)));
    let content = div()
        .size_full()
        .child(div().id("cover").absolute().size(px(400.)).hero_with(&s.expanded, "thumb"))
        .child(div().id("title").h(px(20.)).reveal_with(&s.expanded));
    presentation("now-playing", &s.expanded).base(page).content(content).into_any_element()
}

fn set_open(rig: &mut Rig<Shown>, open: bool) {
    rig.win.root().clone().update(&mut rig.cx, |h, cx| {
        h.s.expanded.set_open(open);
        cx.notify();
    });
}

#[test]
fn reduce_motion_never_draws_the_hero_big_after_close() {
    let mut rig = Rig::sized(Size { width: 480., height: 640. }, |_| Shown { expanded: Presentation::new() }, view);
    rig.cx.set_reduce_motion(true);
    set_open(&mut rig, true);
    while rig.cx.needs_frame() {
        rig.cx.advance_clock(FRAME);
        rig.cx.frame();
    }
    let cover = rig.node("cover");
    assert_eq!(rig.props::<DivProps>("cover").style.opacity, None, "shown in place");
    set_open(&mut rig, false);
    let mut frames = 0;
    while rig.state().expanded.is_shown() {
        rig.cx.frame();
        if let Some(props) = rig.cx.backend().props(cover) {
            let opacity = props.downcast_ref::<DivProps>().expect("div").style.opacity;
            assert_eq!(opacity, Some(0.), "frame {frames} after close draws the cover");
        }
        rig.cx.advance_clock(FRAME);
        frames += 1;
        assert!(frames < 120, "closes");
    }
    rig.cx.frame();
    assert!(rig.cx.backend().props(cover).is_none(), "torn down");
}
