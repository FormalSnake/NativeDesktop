//! `canvas`, tier 1: the paint closure runs for the laid-out bounds, its display list reaches
//! the backend as props, a resize paints again, and a backend that lays out itself reports the
//! size it gave the canvas.

use std::cell::Cell;
use std::rc::Rc;

use nd_gpui::gpui::{hsla, px};
use nd_ui::test_app::TestApp;
use nd_ui::*;

const WINDOW: Size = Size { width: 400., height: 300. };

struct Gauge {
    wide: bool,
    runs: Rc<Cell<u32>>,
}

impl Render for Gauge {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let runs = self.runs.clone();
        let w = if self.wide { 200. } else { 100. };
        v_flex().child(
            canvas(move |bounds, paint, _| {
                runs.set(runs.get() + 1);
                paint.fill_circle(bounds.center(), bounds.size.height / 2., hsla(0.6, 1., 0.5, 1.));
                paint.stroke_arc(bounds.center(), px(10.), 0., 0.25, px(2.), hsla(0., 1., 0.5, 1.));
            })
            .id("gauge")
            .w(px(w))
            .h(px(40.)),
        )
    }
}

fn props(cx: &TestApp) -> (NodeId, CanvasProps) {
    let node = cx.backend().find(|kind, _| kind == backend::NodeKind::Canvas)[0];
    (node, cx.backend().props(node).and_then(|p| p.downcast_ref::<CanvasProps>()).cloned().unwrap())
}

#[test]
fn the_display_list_follows_the_laid_out_size() {
    let runs = Rc::new(Cell::new(0));
    let mut cx = TestApp::new();
    let r = runs.clone();
    let handle = cx.open_window(WINDOW, move |_, cx| cx.new(|_| Gauge { wide: false, runs: r }));
    cx.run_until_idle();
    let (_, p) = props(&cx);
    assert_eq!((p.width, p.height), (100., 40.));
    assert_eq!(p.ops.len(), 2);
    assert!(matches!(p.ops[0], PaintOp::FillCircle { center: (50., 20.), radius: 20., .. }), "{:?}", p.ops[0]);

    handle.root().update(&mut cx, |g, cx| {
        g.wide = true;
        cx.notify();
    });
    cx.run_until_idle();
    let (_, p) = props(&cx);
    assert_eq!(p.width, 200.);
    assert!(matches!(p.ops[0], PaintOp::FillCircle { center: (100., 20.), .. }));
    let before = runs.get();
    cx.run_until_idle();
    assert_eq!(runs.get(), before, "an idle window paints nothing");
}

#[test]
fn a_backend_that_lays_out_itself_reports_the_size() {
    let mut cx = TestApp::with_backend(test_backend::TestBackend::new().without_frames());
    cx.open_window(WINDOW, |_, cx| cx.new(|_| Gauge { wide: false, runs: Rc::default() }));
    cx.run_until_idle();
    let (node, p) = props(&cx);
    assert_eq!(p.width, 100., "painted for the size its style gives");
    cx.emit(node, CanvasResized { width: 120., height: 40. });
    cx.run_until_idle();
    let (_, p) = props(&cx);
    assert_eq!(p.width, 120.);
    assert!(matches!(p.ops[0], PaintOp::FillCircle { center: (60., 20.), .. }));
}
