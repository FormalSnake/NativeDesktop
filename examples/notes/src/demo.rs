use crate::model::{LibraryData, Note, NoteId, Tag};

/// `NOTES_DEMO=1` seeds a fixed library: screenshots, docs captures and tests start from it.
pub fn enabled() -> bool {
    std::env::var_os("NOTES_DEMO").is_some()
}

pub fn library() -> LibraryData {
    let at = |minutes: i64| jiff::Timestamp::UNIX_EPOCH + jiff::SignedDuration::from_mins(29_000_000 + minutes);
    let note = |id, body: &str, tag, pinned, minutes| Note {
        id: NoteId(id),
        title: body.lines().next().unwrap_or_default().to_owned().into(),
        body: body.to_owned(),
        tag,
        pinned,
        archived: false,
        updated: at(minutes),
        version: 0,
    };
    LibraryData {
        next_id: 4,
        notes: vec![
            note(1, "Groceries\nOat milk, lemons, the good bread.", Some(Tag::Green), true, 40),
            note(2, "Trip to Lisbon\nTram 28 before nine or it is full.", Some(Tag::Blue), false, 30),
            note(3, "Release checklist\nBump, tag, sign, notarize, publish feed.", Some(Tag::Red), false, 20),
            note(4, "Ideas\nA file manager with a column view that never stutters.", None, false, 10),
        ],
    }
}
