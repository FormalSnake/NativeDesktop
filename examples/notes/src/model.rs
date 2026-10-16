use std::collections::HashMap;
use std::path::PathBuf;

use nd::prelude::*;
use nd::settings::Store;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "nd::schemars")]
pub struct NoteId(pub u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Folder {
    All,
    Pinned,
    Archive,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Note {
    pub id: NoteId,
    pub title: String,
    pub body: String,
    pub tag: Option<Tag>,
    pub pinned: bool,
    pub archived: bool,
    pub updated: jiff::Timestamp,
    /// Bumped on every edit; list rows re-render only when it changes.
    #[serde(default)]
    pub version: u64,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub enum Tag {
    Red,
    Orange,
    Green,
    Blue,
}

impl Tag {
    pub fn color(self, theme: &Theme) -> Hsla {
        match self {
            Tag::Red => theme.danger,
            Tag::Orange => theme.warning,
            Tag::Green => theme.success,
            Tag::Blue => theme.accent,
        }
    }
}

/// What a list row or the palette shows of a note: everything but the body.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub id: NoteId,
    pub title: SharedString,
    pub preview: SharedString,
    pub tag: Option<Tag>,
    pub pinned: bool,
    pub archived: bool,
    pub updated: jiff::Timestamp,
    pub version: u64,
}

impl Row {
    fn of(note: &Note) -> Self {
        Self {
            id: note.id,
            title: note.title.clone().into(),
            preview: note.body.lines().skip(1).find(|l| !l.trim().is_empty()).unwrap_or("").to_owned().into(),
            tag: note.tag,
            pinned: note.pinned,
            archived: note.archived,
            updated: note.updated,
            version: note.version,
        }
    }
}

impl Note {
    pub fn title_of(body: &str) -> String {
        body.lines().next().filter(|l| !l.is_empty()).unwrap_or("Untitled").to_owned()
    }
}

/// Persisted library. `Store` writes `library.json` under the app data dir, debounced, via tmp+rename.
#[derive(Default, Clone, Serialize, Deserialize)]
pub struct LibraryData {
    pub next_id: u64,
    pub notes: Vec<Note>,
}

/// A library to open instead of the user's own: `NOTES_DEMO=1` and the tests set it so
/// screenshots and assertions start from fixed notes and never touch real data.
pub struct Seed(pub LibraryData);

const STORE_VERSION: u32 = 1;

/// The store plus indexes kept up to date on each mutation, so a keystroke in the editor costs
/// one note's worth of work, not a sort of the whole library.
pub struct Library {
    store: Store<LibraryData>,
    rows: HashMap<NoteId, Row>,
    /// Pinned first, then most recently edited.
    order: Vec<NoteId>,
    /// Lowercased bodies for search, by id.
    search_text: HashMap<NoteId, String>,
    /// Per-folder counts, recomputed on create, delete and pin, not on text edits.
    counts: HashMap<Folder, usize>,
}

impl Library {
    pub fn open(cx: &mut App) -> Entity<Self> {
        let store = match cx.try_global::<Seed>().map(|seed| seed.0.clone()) {
            Some(data) => {
                let store = Store::open(scratch_path(), STORE_VERSION, None);
                store.update(|d| *d = data);
                store
            }
            None => Store::open(nd::core::paths::app_data_dir("notes").join("library.json"), STORE_VERSION, None),
        };
        cx.new(|_| {
            let mut library = Self {
                store,
                rows: HashMap::new(),
                order: Vec::new(),
                search_text: HashMap::new(),
                counts: HashMap::new(),
            };
            library.reindex();
            library
        })
    }

    pub fn note(&self, id: &NoteId) -> Option<&Row> {
        self.rows.get(id)
    }

    pub fn body(&self, id: &NoteId) -> String {
        self.store.get().notes.iter().find(|n| n.id == *id).map(|n| n.body.clone()).unwrap_or_default()
    }

    pub fn count(&self, folder: Folder) -> usize {
        self.counts.get(&folder).copied().unwrap_or(0)
    }

    /// Keys with versions for `ListState::set_items_versioned`.
    pub fn rows(&self, folder: Folder, query: &str) -> Vec<(NoteId, u64)> {
        let query = query.to_lowercase();
        self.order
            .iter()
            .filter_map(|id| self.rows.get(id))
            .filter(|n| folder.contains(n))
            .filter(|n| query.is_empty() || self.search_text.get(&n.id).is_some_and(|t| t.contains(&query)))
            .map(|n| (n.id, n.version))
            .collect()
    }

    pub fn create(&mut self, cx: &mut Context<Self>) -> NoteId {
        let note = self.store.update(|data| {
            data.next_id += 1;
            let note = Note {
                id: NoteId(data.next_id),
                title: "Untitled".into(),
                body: String::new(),
                tag: None,
                pinned: false,
                archived: false,
                updated: jiff::Timestamp::now(),
                version: 0,
            };
            data.notes.push(note.clone());
            note
        });
        let id = note.id;
        self.rows.insert(id, Row::of(&note));
        self.search_text.insert(id, String::new());
        self.reorder(id);
        self.recount();
        cx.notify();
        id
    }

    pub fn delete(&mut self, id: NoteId, cx: &mut Context<Self>) {
        self.store.update(|data| data.notes.retain(|n| n.id != id));
        self.rows.remove(&id);
        self.order.retain(|n| *n != id);
        self.search_text.remove(&id);
        self.recount();
        cx.notify();
    }

    pub fn toggle_pin(&mut self, id: NoteId, cx: &mut Context<Self>) {
        self.edit(id, cx, |note| note.pinned = !note.pinned);
        self.reorder(id);
        self.recount();
    }

    pub fn duplicate(&mut self, id: NoteId, cx: &mut Context<Self>) -> Option<NoteId> {
        let tag = self.rows.get(&id)?.tag;
        let body = self.body(&id);
        let copy = self.create(cx);
        self.set_body(copy, body, cx);
        self.edit(copy, cx, |note| note.tag = tag);
        Some(copy)
    }

    /// Called for user edits only (`TextEvent::Edited`), so opening a note never reorders the list.
    pub fn set_body(&mut self, id: NoteId, body: String, cx: &mut Context<Self>) {
        self.search_text.insert(id, body.to_lowercase());
        self.edit(id, cx, |note| {
            note.title = Note::title_of(&body);
            note.body = body;
            note.updated = jiff::Timestamp::now();
        });
        self.reorder(id);
    }

    fn edit(&mut self, id: NoteId, cx: &mut Context<Self>, f: impl FnOnce(&mut Note)) {
        let row = self.store.update(|data| {
            let note = data.notes.iter_mut().find(|n| n.id == id)?;
            f(note);
            note.version += 1;
            Some(Row::of(note))
        });
        if let Some(row) = row {
            self.rows.insert(id, row);
        }
        cx.notify();
    }

    /// An edited note moves to the front of its group (pinned or not); everything else keeps order.
    fn reorder(&mut self, id: NoteId) {
        self.move_to_front(id);
        let (mut front, back): (Vec<_>, Vec<_>) =
            self.order.iter().partition(|n| self.rows.get(n).is_some_and(|r| r.pinned));
        front.extend(back);
        self.order = front;
    }

    fn move_to_front(&mut self, id: NoteId) {
        self.order.retain(|n| *n != id);
        self.order.insert(0, id);
    }

    fn reindex(&mut self) {
        let data = self.store.get();
        let mut notes: Vec<&Note> = data.notes.iter().collect();
        notes.sort_by(|a, b| b.pinned.cmp(&a.pinned).then(b.updated.cmp(&a.updated)));
        self.order = notes.iter().map(|n| n.id).collect();
        self.search_text = notes.iter().map(|n| (n.id, n.body.to_lowercase())).collect();
        self.rows = notes.iter().map(|n| (n.id, Row::of(n))).collect();
        drop(notes);
        drop(data);
        self.recount();
    }

    fn recount(&mut self) {
        self.counts = Folder::ALL.iter().map(|f| (*f, self.rows.values().filter(|r| f.contains(r)).count())).collect();
    }
}

/// Where a seeded library lives: nowhere a user would look, and distinct per library.
fn scratch_path() -> PathBuf {
    use std::sync::atomic::{AtomicU32, Ordering};
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    #[cfg(not(target_arch = "wasm32"))]
    let root = std::env::temp_dir();
    #[cfg(target_arch = "wasm32")]
    let root = PathBuf::from("notes-scratch");
    root.join(format!("notes-library-{}-{n}", std::process::id())).join("library.json")
}

impl Folder {
    pub const ALL: [Folder; 3] = [Folder::All, Folder::Pinned, Folder::Archive];

    pub fn id(self) -> &'static str {
        match self {
            Folder::All => "folder-all",
            Folder::Pinned => "folder-pinned",
            Folder::Archive => "folder-archive",
        }
    }

    pub fn from_id(id: &str) -> Option<Folder> {
        Folder::ALL.into_iter().find(|f| f.id() == id)
    }

    pub fn label(self) -> &'static str {
        match self {
            Folder::All => "All Notes",
            Folder::Pinned => "Pinned",
            Folder::Archive => "Archive",
        }
    }

    pub fn icon(self) -> Icon {
        match self {
            Folder::All => Icon::Notebook,
            Folder::Pinned => Icon::Pin,
            Folder::Archive => Icon::Archive,
        }
    }

    fn contains(self, note: &Row) -> bool {
        match self {
            Folder::All => !note.archived,
            Folder::Pinned => note.pinned && !note.archived,
            Folder::Archive => note.archived,
        }
    }
}
