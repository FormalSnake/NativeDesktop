//! Tier 1: the demo over a fake system. Calls run on nd-system's task threads; the test waits
//! for the fake to see them, then runs the continuations a host would deliver.

use std::time::{Duration, Instant};

use nd::system::{MediaCommand, System, SystemEvent};
use nd::test::{TestApp, click, text};

fn eventually(what: &str, mut ok: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ok() {
        assert!(Instant::now() < deadline, "{what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Lets the task threads finish and delivers what they posted.
fn settle(cx: &mut TestApp) {
    std::thread::sleep(Duration::from_millis(50));
    nd::system::run_pending(cx);
    cx.run_until_idle();
}

#[test]
fn every_button_reaches_its_service() {
    let mut cx = TestApp::new();
    let (sys, fake) = System::fake();
    let window = system_demo::open(&mut cx, sys).id();
    assert!(text(&mut cx, window).contains("Drop files on this window"), "{}", text(&mut cx, window));

    click(&mut cx, window, "copy-text");
    eventually("the copy reached the clipboard", || fake.state().clipboard_text.is_some());
    click(&mut cx, window, "remember");
    eventually("the secret was stored", || !fake.state().credentials.is_empty());
    click(&mut cx, window, "tray");
    eventually("the tray item was shown", || fake.state().trays.contains_key("system-demo"));
    fake.answer_message(1);
    click(&mut cx, window, "alert");
    settle(&mut cx);
    let shown = text(&mut cx, window);
    assert!(shown.contains("copy: ()") && shown.contains("alert: 1"), "{shown}");

    fake.emit(SystemEvent::OpenUrls(vec!["nd-system-demo://hello".into()]));
    fake.emit(SystemEvent::Media(MediaCommand::Next));
    settle(&mut cx);
    let shown = text(&mut cx, window);
    assert!(shown.contains("nd-system-demo://hello") && shown.contains("Next"), "{shown}");
}
