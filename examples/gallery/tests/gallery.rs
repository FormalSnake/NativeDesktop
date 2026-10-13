//! Tier 1: the Forms page on the test backend. Every control family shows up in the semantic
//! tree with its role, and one control of each family answers automation.

use gallery::Gallery;
use nd::prelude::*;
use nd::test::{Role, SemanticAction, SemanticNode, Size, TestApp};

struct Rig {
    cx: TestApp,
    win: WindowHandle<Gallery>,
}

fn open() -> Rig {
    let mut cx = TestApp::new();
    let win = cx.open_window(Size { width: 960., height: 2400. }, |_, cx| cx.new(|cx| Gallery::new(cx)));
    Rig { cx, win }
}

impl Rig {
    fn sem(&self, id: &str) -> SemanticNode {
        let tree = self.cx.semantic_tree();
        match tree.nodes().find(|(_, n)| n.test_id.as_deref() == Some(id)) {
            Some((_, n)) => n.clone(),
            None => panic!("no node with id {id:?} in\n{}", tree.snapshot()),
        }
    }

    fn act(&mut self, id: &str, action: SemanticAction) {
        let node = self.sem(id).id;
        self.cx.perform(node, &action).unwrap_or_else(|e| panic!("{id}: {e}"));
        self.cx.run_until_idle();
    }

    fn set(&mut self, id: &str, value: &str) {
        self.act(id, SemanticAction::SetValue(value.into()));
    }

    fn value(&self, id: &str) -> Option<String> {
        self.sem(id).value
    }
}

#[test]
fn the_forms_page_is_the_first_page_and_shows_every_control_role() {
    let rig = open();
    assert_eq!(rig.win.root().read(&rig.cx).current().as_ref(), "forms");
    let tree = rig.cx.semantic_tree();
    let roles: Vec<Role> = tree.nodes().map(|(_, n)| n.role).collect();
    for role in [
        Role::TextField,
        Role::SearchField,
        Role::TextArea,
        Role::ComboBox,
        Role::Button,
        Role::Checkbox,
        Role::Switch,
        Role::RadioGroup,
        Role::SegmentedControl,
        Role::PopUpButton,
        Role::Slider,
        Role::Incrementor,
        Role::DatePicker,
        Role::ColorWell,
        Role::ProgressIndicator,
        Role::Text,
        Role::Link,
    ] {
        assert!(roles.contains(&role), "no {role:?} in\n{}", tree.snapshot());
    }
}

#[test]
fn text_controls_take_values() {
    let mut rig = open();
    rig.set("name", "Ada Lovelace");
    assert_eq!(rig.value("name").as_deref(), Some("Ada Lovelace"));
    rig.act("name", SemanticAction::Type("!".into()));
    assert_eq!(rig.value("name").as_deref(), Some("Ada Lovelace!"));
    rig.set("password", "hunter2");
    assert_eq!(rig.value("password"), None, "a secure field never reads back");
    rig.set("search", "wifi");
    assert_eq!(rig.value("search").as_deref(), Some("wifi"));
    rig.set("zone", "Atlantic/Canary");
    assert_eq!(rig.value("zone").as_deref(), Some("Atlantic/Canary"));
}

#[test]
fn toggles_follow_their_group() {
    let mut rig = open();
    assert_eq!(rig.value("copy-all").as_deref(), Some("mixed"));
    rig.act("copy-all", SemanticAction::Click);
    for id in ["copy-all", "copy-photos", "copy-videos"] {
        assert_eq!(rig.sem(id).checked, Some(true), "{id}");
    }
    rig.act("copy-videos", SemanticAction::Click);
    assert_eq!(rig.value("copy-all").as_deref(), Some("mixed"));
    rig.act("wifi", SemanticAction::Click);
    assert_eq!(rig.sem("wifi").checked, Some(false));
    rig.act("bluetooth", SemanticAction::Click);
    assert_eq!(rig.sem("bluetooth").checked, Some(true));
    assert_eq!(rig.sem("bluetooth").name, "Bluetooth");
}

#[test]
fn choices_pick_by_title() {
    let mut rig = open();
    assert_eq!(rig.value("appearance").as_deref(), Some("Automatic"));
    rig.set("appearance", "Dark");
    rig.set("range", "Month");
    rig.set("sort", "Size");
    assert_eq!(
        [rig.value("appearance"), rig.value("range"), rig.value("sort")],
        [Some("Dark".into()), Some("Month".into()), Some("Size".into())]
    );
}

#[test]
fn values_snap_and_wrap() {
    let mut rig = open();
    rig.set("volume", "73");
    assert_eq!(rig.value("volume").as_deref(), Some("70"), "snaps to the step");
    rig.set("copies", "99");
    assert_eq!(rig.value("copies").as_deref(), Some("20"), "clamps to the range");
    rig.set("day", "8");
    assert_eq!(rig.value("day").as_deref(), Some("1"), "wraps past the end");
}

#[test]
fn pickers_clamp_and_parse() {
    let mut rig = open();
    rig.set("due", "2020-05-01");
    assert_eq!(rig.value("due").as_deref(), Some("2026-01-01"), "before the minimum");
    rig.set("alarm", "06:45");
    assert_eq!(rig.value("alarm").as_deref(), Some("06:45:00"));
    rig.set("tint", "#336699");
    assert_eq!(rig.value("tint").as_deref(), Some("#336699"));
}

#[test]
fn buttons_and_progress_react() {
    let mut rig = open();
    assert_eq!(rig.value("download").as_deref(), Some("25%"));
    rig.act("advance", SemanticAction::Click);
    assert_eq!(rig.value("download").as_deref(), Some("50%"));
    assert_eq!(rig.sem("action").name, "None yet");
    rig.act("destructive", SemanticAction::Click);
    assert_eq!(rig.sem("action").name, "Pushed the destructive button");
}

#[test]
fn rows_navigate_and_validate() {
    let mut rig = open();
    rig.act("about", SemanticAction::Click);
    assert_eq!(rig.sem("action").name, "Opened About");
    rig.act("docs", SemanticAction::Click);
    assert_eq!(rig.sem("action").name, "Activated the link");

    let before = rig.cx.semantic_tree().snapshot();
    assert!(!before.contains("Enter an address"), "no error before typing");
    rig.set("signup", "nobody");
    assert!(rig.cx.semantic_tree().snapshot().contains("Enter an address like name@example.com"));
    rig.set("signup", "ada@example.com");
    assert!(!rig.cx.semantic_tree().snapshot().contains("Enter an address"));
    assert!(rig.cx.semantic_tree().snapshot().contains("Used for receipts"));
}

#[test]
fn the_sidebar_lists_the_pages_and_selecting_keeps_forms_state() {
    let mut rig = open();
    rig.set("name", "Grace");
    let list = rig.sem("pages");
    assert_eq!(list.test_id.as_deref(), Some("pages"));
    let node = list.id;
    rig.cx.emit(node, SourceSelected { id: "forms".into() });
    assert_eq!(rig.value("name").as_deref(), Some("Grace"));
}
