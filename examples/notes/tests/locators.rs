//! The same app through Playwright-shaped locators (`nd-test`): roles and test ids, no node refs.
//! Written against a `TestHarness` here, these read the same against a launched app.

#![cfg(not(target_arch = "wasm32"))]
#![allow(clippy::expect_used, clippy::unwrap_used)]

use nd::test::{Size, TestApp};
use nd_test::{TestHarness, expect};
use notes::app::Notes;

fn harness() -> TestHarness {
    let mut cx = TestApp::new();
    notes::actions::init(&mut cx);
    notes::demo::seed(&mut cx);
    let win = cx.open_window(Size { width: 960., height: 640. }, |window, cx| cx.new(|cx| Notes::new(window, cx)));
    // No native table reports a viewport on the test backend, so the list gets the window's.
    let list = win.root().read(&cx).note_list().clone();
    let state = list.read(&cx).list().clone();
    state.handle().0.borrow_mut().set_viewport(300., 640.);
    state.sync(&mut cx);
    cx.run_until_idle();
    TestHarness::new(cx)
}

#[test]
fn the_new_note_shortcut_works_from_the_editor() {
    let app = harness();
    expect(&app.get_by_test_id("note-5")).not().to_be_attached().unwrap();
    app.get_by_test_id("body").focus().unwrap();
    app.keyboard().press("secondary-n").unwrap();
    expect(&app.get_by_test_id("note-5")).to_be_attached().unwrap();
    expect(&app.get_by_test_id("body")).to_be_focused().unwrap();
    app.get_by_test_id("body").type_text("Plan the week").unwrap();
    expect(&app.get_by_test_id("note-5")).to_contain_text("Plan the week").unwrap();
}
