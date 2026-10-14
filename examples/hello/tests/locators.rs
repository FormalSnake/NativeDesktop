//! Tier 3: the `hello` binary as its own process, driven with `nd-test` locators over the
//! automation socket. The `real_` prefix puts it in nextest's `real` profile (`nd test --tier real`).

#![cfg(not(target_arch = "wasm32"))]

// docs:begin tier3-locators
use nd_test::{LaunchOptions, Role, expect, launch_app};

#[test]
fn real_add_one_updates_the_count() {
    let app = launch_app(LaunchOptions::new(env!("CARGO_BIN_EXE_hello"))).expect("launch hello");
    expect(&app.get_by_role(Role::Text).name("0").exact()).to_be_visible().unwrap();
    app.get_by_test_id("add").click().unwrap();
    expect(&app.get_by_role(Role::Text).name("1").exact()).to_be_visible().unwrap();
}
// docs:end tier3-locators
