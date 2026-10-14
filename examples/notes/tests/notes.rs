#![allow(clippy::expect_used, clippy::unwrap_used)]

//! Tier 1: the whole app on the test backend, with a fixed library. Runs natively and, with
//! `--target wasm32-unknown-unknown`, in headless Chromium (`just web-test`).

use std::cell::RefCell;
use std::rc::Rc;

use nd::components::services::{AlertCallback, Chrome, ChromeServices, OpenCallback, SaveCallback};
use nd::components::{
    Alert, ContextMenu, ContextMenuRequested, Menu, MenuItem, OpenDialog, PaletteActivated, PaletteQueryChanged,
    SaveDialog, SourceSelected,
};
use nd::core::{NodeId, WindowId};
use nd::prelude::*;
use nd::test::{SemanticAction, SemanticNode, Size, TestApp, by_test_id, click, text, tree};
use nd::virtual_list::{RowActionInvoked, RowEdge, RowId, RowsSelected};
use notes::actions::{DeleteNote, OpenPalette};
use notes::app::Notes;
use notes::model::NoteId;

const WINDOW: Size = Size { width: 960., height: 640. };

/// A platform that keeps the menus and context menus it was asked to show.
#[derive(Default)]
struct Platform {
    context: RefCell<Vec<Vec<String>>>,
}

impl ChromeServices for Platform {
    fn set_menus(&self, _: &mut App, _: Vec<Menu>) {}
    fn show_context_menu(&self, _: &mut App, _: WindowId, _: NodeId, _: f32, _: f32, menu: ContextMenu) {
        let labels = menu
            .items
            .iter()
            .map(|item| match item {
                MenuItem::Action { label, .. } => label.to_string(),
                MenuItem::Separator => "-".to_owned(),
                _ => "?".to_owned(),
            })
            .collect();
        self.context.borrow_mut().push(labels);
    }
    fn alert(&self, _: &mut App, _: WindowId, _: Alert, _: AlertCallback) {}
    fn open_dialog(&self, cx: &mut App, _: WindowId, _: OpenDialog, done: OpenCallback) {
        done(None, cx);
    }
    fn save_dialog(&self, cx: &mut App, _: WindowId, _: SaveDialog, done: SaveCallback) {
        done(None, cx);
    }
}

struct Rig {
    cx: TestApp,
    win: WindowHandle<Notes>,
    platform: Rc<Platform>,
}

fn open() -> Rig {
    let mut cx = TestApp::new();
    let platform = Rc::new(Platform::default());
    cx.set_global(Chrome(platform.clone()));
    notes::actions::init(&mut cx);
    notes::demo::seed(&mut cx);
    let win = cx.open_window(WINDOW, |window, cx| cx.new(|cx| Notes::new(window, cx)));
    let mut rig = Rig { cx, win, platform };
    rig.size_list();
    rig
}

impl Rig {
    /// The test backend has no native table to report a viewport, so the list gets the window's.
    fn size_list(&mut self) {
        let list = self.win.root().read(&self.cx).note_list().clone();
        let state = list.read(&self.cx).list().clone();
        state.handle().0.borrow_mut().set_viewport(300., WINDOW.height as f64);
        state.sync(&mut self.cx);
        self.cx.run_until_idle();
    }

    fn id(&self) -> WindowId {
        self.win.id()
    }

    fn node(&mut self, test_id: &str) -> SemanticNode {
        let id = self.id();
        by_test_id(&mut self.cx, id, test_id)
            .unwrap_or_else(|| panic!("no `{test_id}` in\n{}", self.cx.semantic_tree().snapshot()))
    }

    /// The notes in the list, top to bottom.
    fn rows(&mut self) -> Vec<String> {
        self.size_list();
        let id = self.id();
        let root = tree(&mut self.cx, id);
        let list = root.iter().find(|n| n.test_id.as_deref() == Some("notes")).expect("the list").clone();
        list.iter().filter_map(|n| n.test_id.as_deref().filter(|t| t.starts_with("note-")).map(str::to_owned)).collect()
    }

    fn act(&mut self, test_id: &str, action: SemanticAction) {
        let node = self.node(test_id).id;
        self.cx.perform(node, &action).unwrap_or_else(|e| panic!("{test_id}: {e:?}"));
        self.cx.run_until_idle();
    }
}

#[nd::test]
fn pinned_notes_sort_first() {
    let mut rig = open();
    assert_eq!(rig.rows(), ["note-1", "note-2", "note-3", "note-4"]);
    let id = rig.id();
    assert!(nd::test::text(&mut rig.cx, id).contains("Groceries"));
}

impl Rig {
    fn row_id(&self, note: u64) -> RowId {
        let list = self.win.root().read(&self.cx).note_list().clone();
        list.read(&self.cx).list().row_of(&NoteId(note)).expect("the note is in the list")
    }

    fn all_text(&mut self) -> String {
        let id = self.id();
        text(&mut self.cx, id)
    }

    fn editor_text(&mut self) -> String {
        self.node("body").value.unwrap_or_default()
    }
}

#[nd::test]
fn a_new_note_is_first_selected_and_focused() {
    let mut rig = open();
    let id = rig.id();
    click(&mut rig.cx, id, "new-note");
    assert_eq!(rig.rows(), ["note-1", "note-5", "note-2", "note-3", "note-4"], "under the pinned note, above the rest");
    assert!(rig.node("note-5").subtree_text().contains("Untitled"));
    let body = rig.node("body").id;
    assert_eq!(rig.cx.focused(id), Some(body), "the caret is in the editor");

    rig.act("body", SemanticAction::Type("Plan the week\nMonday: ship".into()));
    assert!(rig.node("note-5").subtree_text().contains("Plan the week"), "{}", rig.all_text());
    assert_eq!(rig.rows()[1], "note-5", "an edited note stays first among the unpinned");
}

#[nd::test]
fn the_context_menu_offers_pin_duplicate_and_delete() {
    let mut rig = open();
    let row = rig.node("note-2").id;
    rig.cx.emit(row, ContextMenuRequested { node: row, x: 4., y: 4. });
    let pinned_row = rig.node("note-1").id;
    rig.cx.emit(pinned_row, ContextMenuRequested { node: pinned_row, x: 4., y: 4. });
    let menus = rig.platform.context.borrow().clone();
    assert_eq!(menus[0], ["Pin", "Duplicate", "-", "Delete"]);
    assert_eq!(menus[1][0], "Unpin", "a pinned note offers to unpin");

    // Items dispatch from the focused node, as a key binding would.
    let id = rig.id();
    assert!(rig.cx.dispatch_action(id, &DeleteNote(NoteId(2))));
    rig.cx.run_until_idle();
    assert_eq!(rig.rows(), ["note-1", "note-3", "note-4"]);
}

#[nd::test]
fn swiping_a_row_reaches_delete_and_pin() {
    let mut rig = open();
    let list = rig.node("notes").id;
    let row = rig.row_id(3);
    rig.cx.emit(list, RowActionInvoked { row, edge: RowEdge::Trailing, index: 0 });
    assert_eq!(rig.rows(), ["note-1", "note-2", "note-4"], "the trailing action deletes");

    let row = rig.row_id(4);
    rig.cx.emit(list, RowActionInvoked { row, edge: RowEdge::Leading, index: 0 });
    assert_eq!(rig.rows()[0], "note-4", "the leading action pins, and pinned notes sort first");
}

#[nd::test]
fn the_palette_completes_and_opens_a_note() {
    let mut rig = open();
    let id = rig.id();
    assert!(rig.cx.dispatch_action(id, &OpenPalette));
    rig.cx.run_until_idle();
    assert!(rig.win.root().read(&rig.cx).palette_open(), "cmd-k opens the palette");

    let palette = rig.node("palette").id;
    rig.cx.emit(palette, PaletteQueryChanged { text: "tri".into() });
    let rows = rig.win.root().read(&rig.cx).palette_rows(&rig.cx);
    assert_eq!(rows[0].id, "note:2", "the matching note comes first");
    assert_eq!(rows[0].completion.as_deref(), Some("Trip to Lisbon"), "and offers its title inline");
    assert!(rows.iter().all(|r| r.id != "note:1"), "the query filters rows");

    rig.cx.emit(palette, PaletteActivated { id: "note:2".into() });
    assert!(!rig.win.root().read(&rig.cx).palette_open(), "activating closes the palette");
    assert!(rig.editor_text().contains("Tram 28"));
}

#[nd::test]
fn opening_a_note_does_not_reorder_the_list() {
    let mut rig = open();
    let list = rig.node("notes").id;
    let row = rig.row_id(4);
    rig.cx.emit(list, RowsSelected(vec![row]));
    assert!(rig.editor_text().contains("A file manager"), "{}", rig.editor_text());
    assert_eq!(rig.rows(), ["note-1", "note-2", "note-3", "note-4"], "set_text records Replaced, not Edited");
}

#[nd::test]
fn folders_and_search_filter_the_list() {
    let mut rig = open();
    let folders = rig.node("folders").id;
    rig.cx.emit(folders, SourceSelected { id: "folder-pinned".into() });
    assert_eq!(rig.rows(), ["note-1"]);
    rig.cx.emit(folders, SourceSelected { id: "folder-all".into() });
    rig.act("search", SemanticAction::SetValue("lisbon".into()));
    assert_eq!(rig.rows(), ["note-2"]);
}

#[nd::test]
fn pinch_zoom_snaps_back_at_one_hundred() {
    use nd::prelude::{GesturePhase, Haptic, PinchEvent};
    let mut rig = open();
    let editor = rig.node("editor").id;
    let pinch = |scale| PinchEvent { position: nd::core::Point { x: 0., y: 0. }, scale, phase: GesturePhase::Changed };
    rig.cx.emit(editor, pinch(0.8));
    assert!(rig.node("zoom-label").subtree_text().contains("80%"));
    rig.cx.emit(editor, pinch(1.3));
    let id = rig.id();
    assert!(by_test_id(&mut rig.cx, id, "zoom-label").is_none(), "crossed 100% and snapped back");
    assert_eq!(rig.cx.backend().haptics(), [Haptic::Alignment]);
}

#[cfg(not(target_arch = "wasm32"))]
#[nd::test]
fn the_settings_sheet_writes_config_toml() {
    use nd::settings::{Options, Paths, SettingsHandle};
    let dir = tempfile::tempdir().expect("temp dir");
    let mut rig = open();
    let handle = SettingsHandle::<notes::settings::NotesSettings>::load(
        Options::new("notes").paths(Paths::in_dir(dir.path())).env(Vec::new()),
    )
    .expect("settings load");
    nd::components::forms::settings::provide(&mut rig.cx, handle);

    let main = rig.id();
    assert!(rig.cx.dispatch_action(main, &notes::actions::OpenSettings));
    rig.cx.run_until_idle();
    let sheet = rig.cx.window_ids().find(|w| *w != main).expect("the settings sheet opened");
    click(&mut rig.cx, sheet, "compact_rows");
    click(&mut rig.cx, sheet, "done");

    let toml = std::fs::read_to_string(dir.path().join("config.toml")).expect("config.toml written");
    assert!(toml.contains("compact_rows = true"), "{toml}");
}
