//! The frame clock on `TestBackend`: renders read the window's display time, which only the
//! test clock moves (5.3).

use std::cell::RefCell;
use std::rc::Rc;

use nd_ui::test_app::TestApp;
use nd_ui::*;

struct Clocked {
    seen: Rc<RefCell<Vec<(u64, f64)>>>,
}

impl Render for Clocked {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        self.seen.borrow_mut().push((window.frame_time().frame, window.now()));
        div()
    }
}

#[test]
fn renders_sample_the_test_clock() {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let mut cx = TestApp::new();
    let log = seen.clone();
    let handle = cx.open_window(Size { width: 100., height: 100. }, |_, cx| cx.new(|_| Clocked { seen: log }));
    assert_eq!(*seen.borrow(), [(0, 0.)], "the clock stands still until a test moves it");

    for _ in 0..3 {
        cx.advance_clock(1. / 120.);
        handle.root().update(&mut cx, |_, cx| cx.notify());
        cx.run_until_idle();
    }
    let seen = seen.borrow();
    assert_eq!(seen.len(), 4);
    assert_eq!(seen[3].0, 3);
    assert!((seen[3].1 - 3. / 120.).abs() < 1e-12);
    assert!((cx.now() - 3. / 120.).abs() < 1e-12);
    let time = cx.window(handle.id()).unwrap().frame_time();
    assert!((time.interval() - 1. / 120.).abs() < 1e-12);
}

#[test]
fn a_host_tick_sets_the_window_time() {
    let mut cx = TestApp::new();
    let handle =
        cx.open_window(Size { width: 100., height: 100. }, |_, cx| cx.new(|_| Clocked { seen: Rc::default() }));
    let tick = FrameTime { frame: 9, timestamp: 100., target: 100.004 };
    cx.tick_window(handle.id(), tick);
    assert_eq!(cx.window(handle.id()).unwrap().frame_time(), tick);
    assert_eq!(cx.now(), 100.004);
}
