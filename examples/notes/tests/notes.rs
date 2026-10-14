//! Tier 1: the app on the test backend, seeded with the demo library, settings in a temp dir.

use nd::prelude::*;
use nd::test::{Role, SemanticAction, SemanticNode, Size, TestApp};
use nd::virtual_list::RowsSelected;
use nd_settings::{Options, Paths, Store};
use notes::actions::{DeleteNote, NewNote, OpenPalette, OpenSettings};
use notes::app::Notes;
use notes::model::{Folder, Library, LibraryData, NoteId};
use notes::{demo, settings};
use tempfile::TempDir;

struct Rig {
    cx: TestApp,
    win: WindowHandle<Notes>,
    dir: TempDir,
}

fn open() -> Rig {
    let dir = TempDir::new().unwrap();
    let config = dir.path().join("config");
    std::fs::create_dir_all(&config).unwrap();
    let mut cx = TestApp::new();
    settings::load(&mut cx, Options::new("notes").paths(Paths::in_dir(&config)).env(Vec::new()));
    let store = Store::open(dir.path().join("library.json"), LibraryData::VERSION, None);
    store.update(|data| *data = demo::library());
    let library = Library::open(store, &mut cx);
    let win =
        cx.open_window(Size { width: 960., height: 600. }, |window, cx| cx.new(|cx| Notes::new(library, window, cx)));
    Rig { cx, win, dir }
}

impl Rig {
    fn items(&self) -> Vec<u64> {
        self.win.root().read(&self.cx).notes().read(&self.cx).list().items().into_iter().map(|id| id.0).collect()
    }

    fn selected(&self) -> Option<u64> {
        self.win.root().read(&self.cx).notes().read(&self.cx).list().selected().first().map(|id| id.0)
    }

    fn title(&self, id: u64) -> String {
        let library = self.win.root().read(&self.cx).library().read(&self.cx);
        library.note(&NoteId(id)).map(|n| n.title.to_string()).unwrap_or_default()
    }

    fn sem(&self, id: &str) -> SemanticNode {
        let tree = self.cx.semantic_tree();
        match tree.nodes().find(|(_, n)| n.test_id.as_deref() == Some(id)) {
            Some((_, n)) => n.clone(),
            None => panic!("no node with id {id:?} in\n{}", tree.snapshot()),
        }
    }

    fn dispatch(&mut self, action: &dyn Action) {
        let window = self.win.id();
        self.cx.dispatch_action(window, action);
        self.cx.run_until_idle();
    }

    fn act(&mut self, id: &str, action: SemanticAction) {
        let node = self.sem(id).id;
        self.cx.perform(node, &action).unwrap_or_else(|e| panic!("{id}: {e}"));
        self.cx.run_until_idle();
    }

    fn emit<E: 'static>(&mut self, id: &str, event: E) {
        let node = self.sem(id).id;
        self.cx.emit(node, event);
        self.cx.run_until_idle();
    }
}

#[test]
fn pinned_notes_sort_first() {
    let rig = open();
    assert_eq!(rig.items(), [1, 2, 3, 4]);
    assert_eq!(rig.title(1), "Groceries");
}

#[test]
fn folders_count_and_filter() {
    let mut rig = open();
    assert_eq!(rig.sem("folders").role, Role::Tree);
    let counts = Folder::ALL.map(|f| rig.win.root().read(&rig.cx).library().read(&rig.cx).count(f));
    assert_eq!(counts, [4, 1, 0]);
    rig.emit("folders", SourceSelected { id: "pinned".into() });
    assert_eq!(rig.items(), [1]);
}

#[test]
fn new_note_is_selected_and_edits_retitle_it() {
    let mut rig = open();
    rig.dispatch(&NewNote);
    assert_eq!(rig.items().first(), Some(&5));
    assert_eq!(rig.selected(), Some(5));
    assert_eq!(rig.title(5), "Untitled");
    rig.act("body", SemanticAction::SetValue("Plan the week\nMonday: ship".into()));
    assert_eq!(rig.title(5), "Plan the week");
}

#[test]
fn delete_removes_the_row() {
    let mut rig = open();
    rig.dispatch(&DeleteNote(NoteId(1)));
    assert_eq!(rig.items(), [2, 3, 4]);
}

#[test]
fn search_filters_the_list() {
    let mut rig = open();
    rig.act("search", SemanticAction::SetValue("tram".into()));
    assert_eq!(rig.items(), [2]);
    rig.act("search", SemanticAction::SetValue(String::new()));
    assert_eq!(rig.items(), [1, 2, 3, 4]);
}

#[test]
fn palette_completes_and_opens_a_note() {
    let mut rig = open();
    rig.dispatch(&OpenPalette);
    assert!(rig.win.root().read(&rig.cx).palette_open());
    rig.emit("palette", PaletteQueryChanged { text: "tri".into() });
    let items = rig.win.root().read(&rig.cx).palette_items(&rig.cx);
    assert_eq!(items[0].id.as_ref(), "note:2");
    assert_eq!(items[0].completion.as_ref().map(|c| c.as_ref()), Some("Trip to Lisbon"));
    rig.emit("palette", PaletteActivated { id: "note:2".into() });
    assert!(!rig.win.root().read(&rig.cx).palette_open());
    assert_eq!(rig.selected(), Some(2));
    assert!(rig.sem("body").value.unwrap_or_default().contains("Tram 28"));
}

#[test]
fn settings_sheet_round_trips_to_toml() {
    let mut rig = open();
    rig.dispatch(&OpenSettings);
    rig.act("compact_rows", SemanticAction::Click);
    let toml = std::fs::read_to_string(rig.dir.path().join("config/config.toml")).unwrap();
    assert!(toml.contains("compact_rows = true"), "{toml}");
    assert!(settings::current(&rig.cx).compact_rows);
}

#[test]
fn opening_a_note_does_not_reorder_the_list() {
    let mut rig = open();
    let row = rig.win.root().read(&rig.cx).notes().read(&rig.cx).list().row_of(&NoteId(4)).unwrap();
    rig.emit("notes", RowsSelected(vec![row]));
    assert!(rig.sem("body").value.unwrap_or_default().contains("Ideas"));
    assert_eq!(rig.items(), [1, 2, 3, 4]);
}

#[test]
fn pinch_zoom_snaps_at_one_hundred() {
    let mut rig = open();
    let editor = rig.win.root().read(&rig.cx).editor().clone();
    let pinch = |scale| PinchEvent { position: Default::default(), scale, phase: GesturePhase::Changed };
    rig.emit("editor", pinch(0.8));
    assert_eq!(editor.read(&rig.cx).zoom(), 0.8);
    rig.emit("editor", pinch(1.3));
    assert_eq!(editor.read(&rig.cx).zoom(), 1.0, "crossed 100%, snapped back");
    assert_eq!(rig.cx.backend().haptics(), [Haptic::Alignment]);
}

#[test]
fn the_window_shows_the_toolbar_and_editor() {
    let rig = open();
    let tree = rig.cx.semantic_tree();
    let roles: Vec<Role> = tree.nodes().map(|(_, n)| n.role).collect();
    for role in [Role::SearchBox, Role::TextArea, Role::Button] {
        assert!(roles.contains(&role), "no {role:?} in\n{}", tree.snapshot());
    }
}
