use nd::prelude::*;

use crate::model::{LibraryData, Note, NoteId, Seed, Tag};

/// `NOTES_DEMO=1` seeds a fixed library: screenshots, docs captures and tests start from it.
pub fn enabled() -> bool {
    #[cfg(not(target_arch = "wasm32"))]
    return std::env::var_os("NOTES_DEMO").is_some();
    #[cfg(target_arch = "wasm32")]
    false
}

/// The fixed "now" of the demo library: row times read the same on every run.
pub fn now() -> jiff::Timestamp {
    at(50)
}

fn at(minutes: i64) -> jiff::Timestamp {
    jiff::Timestamp::UNIX_EPOCH + jiff::SignedDuration::from_mins(29_000_000 + minutes)
}

pub fn seed(cx: &mut App) {
    let note = |id, body: &str, tag, pinned, minutes| Note {
        id: NoteId(id),
        title: Note::title_of(body),
        body: body.to_owned(),
        tag,
        pinned,
        archived: false,
        updated: at(minutes),
        version: 0,
    };
    cx.set_global(Seed(LibraryData {
        next_id: 4,
        notes: vec![
            note(1, "Groceries\nOat milk, lemons, the good bread.", Some(Tag::Green), true, 40),
            note(2, "Trip to Lisbon\nTram 28 before nine or it is full.", Some(Tag::Blue), false, 30),
            note(3, "Release checklist\nBump, tag, sign, notarize, publish feed.", Some(Tag::Red), false, 20),
            note(4, "Ideas\nA file manager with a column view that never stutters.", None, false, 10),
        ],
    }));
}
