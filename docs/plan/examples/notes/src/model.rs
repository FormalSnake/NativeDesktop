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
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
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
        self.body.lines().skip(1).find(|l| !l.trim().is_empty()).unwrap_or("").into()
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

pub struct Library {
    store: Store<LibraryData>,
}

impl Library {
    pub fn open(cx: &mut App) -> Entity<Self> {
        let store = Store::<LibraryData>::load("library", cx);
        cx.new(|_| Self { store })
    }

    pub fn note(&self, id: &NoteId) -> Option<&Note> {
        self.store.get().notes.iter().find(|n| n.id == *id)
    }

    pub fn count(&self, folder: Folder) -> usize {
        self.store.get().notes.iter().filter(|n| folder.contains(n)).count()
    }

    /// Pinned first, then most recently edited.
    pub fn visible_ids(&self, folder: Folder, query: &str) -> Vec<NoteId> {
        let mut notes: Vec<&Note> = self
            .store
            .get()
            .notes
            .iter()
            .filter(|n| folder.contains(n))
            .filter(|n| query.is_empty() || n.body.to_lowercase().contains(&query.to_lowercase()))
            .collect();
        notes.sort_by(|a, b| b.pinned.cmp(&a.pinned).then(b.updated.cmp(&a.updated)));
        notes.into_iter().map(|n| n.id).collect()
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
                updated: jiff::Timestamp::now(),
            });
            id
        });
        cx.notify();
        id
    }

    pub fn delete(&mut self, id: NoteId, cx: &mut Context<Self>) {
        self.store.update(cx, |data| data.notes.retain(|n| n.id != id));
        cx.notify();
    }

    pub fn toggle_pin(&mut self, id: NoteId, cx: &mut Context<Self>) {
        self.edit(id, cx, |note| note.pinned = !note.pinned);
    }

    pub fn duplicate(&mut self, id: NoteId, cx: &mut Context<Self>) -> Option<NoteId> {
        let source = self.note(&id)?.clone();
        let copy = self.create(cx);
        self.edit(copy, cx, |note| {
            note.title = format!("{} copy", source.title).into();
            note.body = source.body.clone();
            note.tag = source.tag;
        });
        Some(copy)
    }

    pub fn set_body(&mut self, id: NoteId, body: String, cx: &mut Context<Self>) {
        self.edit(id, cx, |note| {
            note.title = body.lines().next().filter(|l| !l.is_empty()).unwrap_or("Untitled").to_owned().into();
            note.body = body;
        });
    }

    fn edit(&mut self, id: NoteId, cx: &mut Context<Self>, f: impl FnOnce(&mut Note)) {
        self.store.update(cx, |data| {
            if let Some(note) = data.notes.iter_mut().find(|n| n.id == id) {
                f(note);
                note.updated = jiff::Timestamp::now();
            }
        });
        cx.notify();
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
