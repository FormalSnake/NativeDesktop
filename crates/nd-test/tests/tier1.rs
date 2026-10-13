//! Tier 1: selectors, locators and `expect` against an in-process app on `TestBackend`.

mod common;

use common::harness;
use nd_test::selector::{self, Part, TextMatch};
use nd_test::{Error, Role, WaitState, expect};
use nd_ui::semantics::Probe;

#[test]
fn selector_grammar_round_trips() {
    for s in [
        "testid=save-btn",
        "role=button[name=\"Save\"] >> nth=1",
        "type=Button >> has-text=Save >> nth=0",
        "role=checkbox[checked=false][disabled]",
        "text=/ad+ one/i >> has=(role=text >> text=\"0\")",
    ] {
        let parts = selector::parse(s).unwrap();
        assert_eq!(selector::parse(&selector::format(&parts)).unwrap(), parts, "{s}");
    }
    let parts = selector::parse("role=button[name=Save][exact] >> first").unwrap();
    assert_eq!(
        parts,
        vec![
            Part::Role {
                role: "button".into(),
                name: Some(TextMatch::Exact("Save".into())),
                checked: None,
                disabled: None
            },
            Part::Nth(0),
        ]
    );
    // `>>` inside quotes and brackets does not split.
    assert_eq!(selector::parse("text=\"a >> b\"").unwrap(), vec![Part::Text(TextMatch::Exact("a >> b".into()))]);
    for bad in ["nope", "bogus=x", "role=", "role=button[size=2]", "nth=x", "text=/(/"] {
        assert!(matches!(selector::parse(bad), Err(Error::Selector(_))), "{bad} should not parse");
    }
}

#[test]
fn click_by_test_id_and_expect_the_label_to_change() {
    let app = harness();
    expect(&app.get_by_role(Role::Text).name("0").exact()).to_be_visible().unwrap();
    app.get_by_test_id("add").click().unwrap();
    expect(&app.get_by_test_id("result").get_by_role(Role::Text).first()).to_have_text("1").unwrap();
    app.get_by_role(Role::Button).name("add").click().unwrap();
    expect(&app.get_by_text("Many")).to_be_visible().unwrap();
    expect(&app.get_by_test_id("result")).to_contain_text("2 clicks").unwrap();
}

#[test]
fn engines_descend_and_filters_refine() {
    let app = harness();
    assert_eq!(app.get_by_role(Role::Button).count().unwrap(), 2);
    assert_eq!(app.locator("testid=toolbar >> role=button").count().unwrap(), 2);
    assert_eq!(app.locator("testid=toolbar >> role=button >> last").text().unwrap(), "Reset");
    assert_eq!(app.locator("role=group >> has=(text=clicks)").count().unwrap(), 2, "the column and result");
    assert_eq!(app.locator("testid=result >> role=text").count().unwrap(), 2);
    // Both groups holding "clicks" contain the same "0"; a node found twice counts once.
    assert_eq!(app.get_by_role(Role::Group).filter_has_text("clicks").get_by_text("0").count().unwrap(), 1);
    assert_eq!(app.locator("type=Button >> nth=-1").text().unwrap(), "Reset");
    assert_eq!(app.get_by_text("CLICKS").count().unwrap(), 1, "text= is case-insensitive");
    assert_eq!(app.get_by_text("click").exact().count().unwrap(), 0);
    assert_eq!(app.locator("role=button[name=/^add/i]").count().unwrap(), 1);
}

#[test]
fn strict_mode_refuses_two_matches() {
    let app = harness();
    let err = app.get_by_role(Role::Button).click().unwrap_err();
    assert!(matches!(&err, Error::Strict { count: 2, .. }), "{err}");
    assert!(err.to_string().contains("[testid=reset]"), "{err}");
}

#[test]
fn actionability_ladder_reports_why() {
    let app = harness();
    // Invisible: present but zero height.
    let err = app.get_by_test_id("collapsed").click().unwrap_err();
    assert!(matches!(&err, Error::Timeout { reason, .. } if reason.contains("not visible")), "{err}");

    // Disabled, as the native control reports it.
    let add = app.get_by_test_id("add").node().unwrap().id;
    app.app().backend_mut().set_probe(add, Probe { enabled: Some(false), ..Probe::default() });
    let err = app.get_by_test_id("add").click().unwrap_err();
    assert!(matches!(&err, Error::Timeout { reason, .. } if reason.contains("disabled")), "{err}");
    expect(&app.get_by_test_id("add")).to_be_disabled().unwrap();

    // No match: the error ranks the nearest nodes.
    let err = app.get_by_test_id("ad").click().unwrap_err();
    let Error::Timeout { reason, candidates, .. } = &err else { panic!("{err}") };
    assert_eq!(reason, "no node matches");
    assert!(candidates[0].contains("[testid=add]"), "{err}");
}

#[test]
fn expect_negation_counts_and_waits() {
    let app = harness();
    expect(&app.get_by_text("Many")).not().to_be_visible().unwrap();
    expect(&app.get_by_text("Many")).to_be_hidden().unwrap();
    expect(&app.get_by_role(Role::Button)).to_have_count(2).unwrap();
    expect(&app.get_by_test_id("collapsed")).to_be_attached().unwrap();
    expect(&app.get_by_test_id("collapsed")).to_be_hidden().unwrap();
    expect(&app.get_by_text("tucked away")).to_be_visible().unwrap();
    let err = expect(&app.get_by_text("Many")).to_be_visible().unwrap_err();
    assert!(err.to_string().contains("- button \"Add one\""), "a failed expect prints the tree: {err}");

    app.get_by_test_id("add").dblclick().unwrap();
    app.get_by_text("Many").wait_for(WaitState::Visible).unwrap();
    app.get_by_test_id("reset").click().unwrap();
    app.get_by_text("Many").wait_for(WaitState::Detached).unwrap();
}

#[test]
fn readers_and_raw_rpcs() {
    let app = harness();
    let add = app.get_by_test_id("add");
    assert!(add.is_visible().unwrap() && add.is_enabled().unwrap() && !add.is_checked().unwrap());
    let b = add.bounding_box().unwrap().unwrap();
    assert_eq!((b.x, b.y, b.width, b.height), (16., 16., 80., 24.));

    let resolved = app.call("resolve", serde_json::json!({ "testId": "add" })).unwrap();
    assert_eq!(resolved["refs"].as_array().unwrap().len(), 1);
    let waited =
        app.wait_for(serde_json::json!({ "testId": "result", "state": "visible" }), std::time::Duration::from_secs(1));
    assert!(waited.is_ok(), "{waited:?}");
    let timeout = app.wait_for(serde_json::json!({ "textContains": "nowhere" }), std::time::Duration::from_millis(10));
    assert!(matches!(timeout, Err(Error::Rpc(e)) if e.code == -32002));
    let snapshot = app.snapshot().unwrap();
    assert!(snapshot.contains("[testid=add]"), "{snapshot}");
}

#[test]
fn keys_without_native_input_go_through_nd_dispatch() {
    let app = harness();
    // TestBackend cannot synthesize events, so keystrokes reach nd's keymap directly.
    let reply = app.keyboard().press("cmd-a enter").unwrap();
    assert_eq!(reply["keys"], serde_json::json!(["unhandled", "unhandled"]));
    let err = app.mouse().click(10., 10.).unwrap_err();
    assert!(matches!(err, Error::Rpc(e) if e.code == -32003));
}
