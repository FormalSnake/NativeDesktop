//! Tier 1: the tab group's logic on the test backend. New tabs from the action and from the tab
//! bar's `+`, closes from the action and from the platform (held until the app drops the page),
//! and the address field opening a page in place of the status page.

use nd::prelude::*;
use nd::test::{Role, TestApp, by_test_id};
use nd_ui::WindowId;

fn windows(cx: &TestApp) -> Vec<WindowId> {
    cx.window_ids().collect()
}

#[test]
fn tabs_open_from_the_action_and_the_plus_and_close_from_both_paths() {
    let mut cx = TestApp::new();
    let browser = tabs::init(&mut cx);
    cx.run_until_idle();
    let first = windows(&cx)[0];
    let tree = cx.semantic_tree();
    assert_eq!(tree.windows[0].tab_group.as_deref(), Some(tabs::GROUP));
    let status = by_test_id(&mut cx, first, "new-tab").expect("a new tab shows its status page");
    assert_eq!((status.role, status.name.as_str()), (Role::Status, "New Tab"));

    assert!(cx.dispatch_action(first, &tabs::NewTab));
    cx.run_until_idle();
    assert_eq!(windows(&cx).len(), 2);
    let second = windows(&cx)[1];
    assert!(cx.request_new_tab(second), "the tab bar's + reaches the app");
    cx.run_until_idle();
    assert_eq!(browser.read(&cx).tabs.len(), 3);
    assert!(cx.semantic_tree().windows.iter().all(|w| w.tab_group.as_deref() == Some(tabs::GROUP)));

    // The platform's close (red button, tab close) is held: the app drops the page and closes.
    assert!(!cx.request_close(second));
    cx.run_until_idle();
    assert!(cx.window(second).is_none());
    assert_eq!(browser.read(&cx).closed, vec![2]);

    assert!(cx.dispatch_action(first, &tabs::CloseTab));
    cx.run_until_idle();
    assert!(cx.window(first).is_none());
    assert_eq!(browser.read(&cx).closed, vec![2, 1]);
    assert_eq!(browser.read(&cx).tabs.len(), 1);
}

#[test]
fn an_address_replaces_the_status_page_with_the_page() {
    let mut cx = TestApp::new();
    let browser = tabs::init(&mut cx);
    cx.run_until_idle();
    let window = windows(&cx)[0];
    let field = by_test_id(&mut cx, window, "address").expect("address field");
    cx.emit(field.id, TextSubmitted { text: "example.com".into() });
    let page = browser.read(&cx).tabs[0].page.clone();
    assert_eq!(page.read(&cx).url.as_deref(), Some("https://example.com"));
    assert_eq!(cx.window(window).unwrap().title().as_ref(), "example.com", "the tab takes the address as its title");
    assert!(by_test_id(&mut cx, window, "new-tab").is_none());
    assert!(by_test_id(&mut cx, window, "page-1").is_some(), "the page shows in a web view");
}
