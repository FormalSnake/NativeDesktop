//! Tier 1: the overrides showcase on the test backend, natively and in a browser
//! (`#[nd::test]`).

use nd::prelude::*;
use nd::test::{TestApp, by_test_id, text};

#[nd::test]
fn every_override_layer_renders(cx: &mut TestApp) {
    let window = overrides::open(cx, Appearance::Dark).id();
    for id in ["share", "rename", "ok", "no"] {
        assert!(by_test_id(cx, window, id).is_some(), "no `{id}` in {}", text(cx, window));
    }
    assert!(text(cx, window).contains("Overrides"));
}

#[nd::test]
fn the_window_override_forces_its_appearance(cx: &mut TestApp) {
    let dark = overrides::open(cx, Appearance::Dark).id();
    let light = overrides::open(cx, Appearance::Light).id();
    let titles = [text(cx, dark), text(cx, light)];
    assert_eq!(titles[0], titles[1], "the same tree in both appearances");
}
