//! Tier 1: the counter on the test backend. `#[nd::test]` runs the same tests natively and, with
//! `--target wasm32-unknown-unknown`, in headless Chromium (`just web-test`).

use nd::test::{TestApp, click, text};

// docs:begin tier1
#[nd::test]
fn starts_at_zero(cx: &mut TestApp) {
    let window = hello::open(cx).id();
    assert!(text(cx, window).contains("0 clicks"), "{}", text(cx, window));
}

#[nd::test]
fn add_one_counts_each_click(cx: &mut TestApp) {
    let window = hello::open(cx).id();
    click(cx, window, "add");
    click(cx, window, "add");
    assert!(text(cx, window).contains("2 clicks"), "{}", text(cx, window));
}
// docs:end tier1
