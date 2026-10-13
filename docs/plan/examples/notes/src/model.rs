use nd::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize, JsonSchema)]
pub struct NoteId(pub u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize, JsonSchema)]
pub enum Folder {
    All,
    Pinned,
    Archive,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Note {
    pub id: NoteId,
    pub title: SharedString,
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

impl Note {
    pub fn preview(&self) -> SharedString {
        self.body.lines().skip(1).find(|l| !l.trim().is_empty()).unwrap_or("").to_owned().into()
    }
}

/// Persisted library. `Store` writes `library.json` under the app data dir, debounced, via tmp+rename.
#[derive(Default, Serialize, Deserialize)]
pub struct LibraryData {
    pub next_id: u64,
    pub notes: Vec<Note>,
}

impl Versioned for LibraryData {
    const VERSION: u32 = 1;
}

/// The store plus indexes kept up to date on each mutation, so a keystroke in the editor costs
/// one note's worth of work, not a sort of the whole library.
pub struct Library {
    store: Store<LibraryData>,
    /// Pinned first, then most recently edited.
    order: Vec<NoteId>,
    /// Lowercased bodies for search, by id.
    search_text: HashMap<NoteId, String>,
    /// Per-folder counts, recomputed on create, delete and pin, not on text edits.
    counts: HashMap<Folder, usize>,
}

impl Library {
    pub fn open(cx: &mut App) -> Entity<Self> {
        let store = Store::<LibraryData>::load("library", cx);
        cx.new(|_| {
            let mut library = Self { store, order: Vec::new(), search_text: HashMap::default(), counts: HashMap::default() };
            library.reindex();
            library
        })
    }

    pub fn note(&self, id: &NoteId) -> Option<&Note> {
        self.store.get().notes.iter().find(|n| n.id == *id)
    }

    pub fn count(&self, folder: Folder) -> usize {
        self.counts.get(&folder).copied().unwrap_or(0)
    }

    /// Keys with versions for `ListState::set_items_versioned`.
    pub fn rows(&self, folder: Folder, query: &str) -> Vec<(NoteId, u64)> {
        let query = query.to_lowercase();
        self.order
            .iter()
            .filter_map(|id| self.note(id))
            .filter(|n| folder.contains(n))
            .filter(|n| query.is_empty() || self.search_text.get(&n.id).is_some_and(|t| t.contains(&query)))
            .map(|n| (n.id, n.version))
            .collect()
    }

    pub fn create(&mut self, cx: &mut Context<Self>) -> NoteId {
        let id = self.store.update(cx, |data| {
            data.next_id += 1;
            let id = NoteId(data.next_id);
            data.notes.push(Note {
                id,
                title: "Untitled".into(),
                body: String::new(),
                tag: None,
                pinned: false,
                archived: false,
                updated: nd::time::now(),
                version: 0,
            });
            id
        });
        self.search_text.insert(id, String::new());
        self.move_to_front(id);
        self.recount();
        cx.notify();
        id
    }

    pub fn delete(&mut self, id: NoteId, cx: &mut Context<Self>) {
        self.store.update(cx, |data| data.notes.retain(|n| n.id != id));
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
        let source = self.note(&id)?.clone();
        let copy = self.create(cx);
        self.set_body(copy, source.body, cx);
        self.edit(copy, cx, |note| note.tag = source.tag);
        Some(copy)
    }

    /// Called for user edits only (`TextEvent::Edited`), so opening a note never reorders the list.
    pub fn set_body(&mut self, id: NoteId, body: String, cx: &mut Context<Self>) {
        self.search_text.insert(id, body.to_lowercase());
        self.edit(id, cx, |note| {
            note.title = body.lines().next().filter(|l| !l.is_empty()).unwrap_or("Untitled").to_owned().into();
            note.body = body;
            note.updated = nd::time::now();
        });
        self.reorder(id);
    }

    fn edit(&mut self, id: NoteId, cx: &mut Context<Self>, f: impl FnOnce(&mut Note)) {
        self.store.update(cx, |data| {
            if let Some(note) = data.notes.iter_mut().find(|n| n.id == id) {
                f(note);
                note.version += 1;
            }
        });
        cx.notify();
    }

    /// An edited note moves to the front of its group (pinned or not); everything else keeps order.
    fn reorder(&mut self, id: NoteId) {
        self.move_to_front(id);
        let pinned = |library: &Self, id: &NoteId| library.note(id).is_some_and(|n| n.pinned);
        let (mut front, back): (Vec<_>, Vec<_>) = self.order.iter().partition(|n| pinned(self, n));
        front.extend(back);
        self.order = front;
    }

    fn move_to_front(&mut self, id: NoteId) {
        self.order.retain(|n| *n != id);
        self.order.insert(0, id);
    }

    fn reindex(&mut self) {
        let mut notes: Vec<&Note> = self.store.get().notes.iter().collect();
        notes.sort_by(|a, b| b.pinned.cmp(&a.pinned).then(b.updated.cmp(&a.updated)));
        self.order = notes.iter().map(|n| n.id).collect();
        self.search_text = notes.iter().map(|n| (n.id, n.body.to_lowercase())).collect();
        self.recount();
    }

    fn recount(&mut self) {
        let notes = &self.store.get().notes;
        self.counts = Folder::ALL.iter().map(|f| (*f, notes.iter().filter(|n| f.contains(n)).count())).collect();
    }
}

impl Folder {
    pub const ALL: [Folder; 3] = [Folder::All, Folder::Pinned, Folder::Archive];

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

    fn contains(self, note: &Note) -> bool {
        match self {
            Folder::All => !note.archived,
            Folder::Pinned => note.pinned && !note.archived,
            Folder::Archive => note.archived,
        }
    }
}
