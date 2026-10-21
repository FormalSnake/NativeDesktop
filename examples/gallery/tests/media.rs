//! Tier 1: the Media page on the test backend. Every media kind shows up with its role, the
//! player bar drives the shared clock that the lyrics follow, and the composer sends into the
//! virtual thread.

use gallery::{Gallery, Page, Start};
use nd::prelude::*;
use nd::test::{Role, SemanticAction, SemanticNode, Size, TestApp};

struct Rig {
    cx: TestApp,
    win: WindowHandle<Gallery>,
}

fn open() -> Rig {
    let mut cx = TestApp::new();
    let win = cx.open_window(Size { width: 960., height: 2400. }, |_, cx| {
        cx.new(|cx| Gallery::new(&Start { page: Page::Media, ..Start::default() }, cx))
    });
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

    fn has(&self, id: &str) -> bool {
        self.cx.semantic_tree().nodes().any(|(_, n)| n.test_id.as_deref() == Some(id))
    }

    fn act(&mut self, id: &str, action: SemanticAction) {
        let node = self.sem(id).id;
        self.cx.perform(node, &action).unwrap_or_else(|e| panic!("{id}: {e}"));
        self.cx.run_until_idle();
    }
}

#[test]
fn the_media_page_shows_every_media_kind() {
    let rig = open();
    assert_eq!(rig.win.root().read(&rig.cx).page(), Page::Media);
    for (id, role) in [
        ("img-rounded", Role::Image),
        ("img-animated", Role::Image),
        ("player.seek", Role::Slider),
        ("player.volume", Role::Slider),
        ("mini.play", Role::Button),
        ("lyrics", Role::List),
        ("shelf", Role::ScrollArea),
        ("avatar-group", Role::Image),
        ("composer.field", Role::TextArea),
    ] {
        assert_eq!(rig.sem(id).role, role, "{id}");
    }
    assert_eq!(rig.sem("shelf").name, "Recently played");
    assert_eq!(rig.sem("avatar-group").name, "Ada Lovelace, Grace Hopper and 2 more");
    // The thread follows its tail: the newest message is on screen.
    assert_eq!(rig.sem("msg-4").value.as_deref(), Some("Sent"));
}

#[test]
fn seeking_the_player_moves_the_lyrics() {
    let mut rig = open();
    assert_eq!(rig.sem("lyrics").value.as_deref(), Some("Turn the radio up and drive"));
    rig.act("player.seek", SemanticAction::SetValue("0:55".into()));
    assert_eq!(rig.sem("player.seek").value.as_deref(), Some("0:55 of 4:03"));
    assert_eq!(rig.sem("mini.seek").value.as_deref(), Some("0:55 of 4:03"), "both bars share the clock");
    assert_eq!(rig.sem("lyrics").value.as_deref(), Some("Carry all our words away"));
    // Icon-only buttons are named by their tooltip, so the full bar's play button names what the
    // mini bar's press did to the shared state.
    let before = rig.sem("player.play").name.clone();
    rig.act("mini.play", SemanticAction::Click);
    let after = rig.sem("player.play").name.clone();
    assert_ne!(before, after, "both bars share the play state");
    assert!(["Play", "Pause"].contains(&after.as_str()), "named by its tooltip: {after}");
}

#[test]
fn the_composer_sends_into_the_thread() {
    let mut rig = open();
    assert!(!rig.has("composer.send"), "send only shows with something to send");
    rig.act("composer.field", SemanticAction::Type("See you there".into()));
    assert!(rig.sem("composer.send").enabled);
    rig.act("composer.send", SemanticAction::Click);
    assert_eq!(rig.sem("msg-5").name, "See you there");
    assert_eq!(rig.sem("composer.field").value.as_deref(), Some(""));
    assert!(!rig.has("composer.send"));
    rig.act("composer.attach", SemanticAction::Click);
    assert!(rig.sem("composer.send").enabled, "an attachment alone can send");
}
