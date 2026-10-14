//! Tier 1: the gallery on the test backend, natively and in a browser (`#[nd::test]`).

use nd::test::{TestApp, by_test_id, click, text};

#[nd::test]
fn opens_on_the_palette_page(cx: &mut TestApp) {
    let window = gallery::init(cx).id();
    assert!(by_test_id(cx, window, "open-palette").is_some(), "{}", text(cx, window));
}

#[nd::test]
fn the_banner_toggles(cx: &mut TestApp) {
    let start = gallery::Start { page: gallery::Page::Banner, ..gallery::Start::default() };
    let window = gallery::open(cx, start).id();
    assert!(text(cx, window).contains("Hide banner"), "{}", text(cx, window));
    click(cx, window, "toggle-banner");
    assert!(text(cx, window).contains("Show banner"), "{}", text(cx, window));
}
