use nd::prelude::*;
use serde::Deserialize;

use crate::model::NoteId;

actions!(notes, [NewNote, OpenPalette, OpenSettings, FocusSearch, ReportIssue]);

#[derive(Clone, PartialEq, Debug, Deserialize, JsonSchema, Action)]
#[schemars(crate = "nd::schemars")]
#[action(namespace = notes)]
pub struct DeleteNote(pub NoteId);

#[derive(Clone, PartialEq, Debug, Deserialize, JsonSchema, Action)]
#[schemars(crate = "nd::schemars")]
#[action(namespace = notes)]
pub struct PinNote(pub NoteId);

#[derive(Clone, PartialEq, Debug, Deserialize, JsonSchema, Action)]
#[schemars(crate = "nd::schemars")]
#[action(namespace = notes)]
pub struct DuplicateNote(pub NoteId);

pub const CONTEXT: &str = "Notes";

/// The key combination shown next to a command: palette hints and the empty state.
pub fn shortcut(action: &dyn Action) -> Option<&'static str> {
    let mac = cfg!(target_os = "macos");
    let label = |mac_label, other: &'static str| Some(if mac { mac_label } else { other });
    match action.name() {
        "notes::NewNote" => label("⌘N", "Ctrl+N"),
        "notes::OpenPalette" => label("⌘K", "Ctrl+K"),
        "notes::OpenSettings" => label("⌘,", "Ctrl+,"),
        "nd::ToggleSidebar" => label("⌃⌘S", "F9"),
        _ => None,
    }
}

pub fn init(cx: &mut App) {
    cx.bind_keys(nd::actions::standard_bindings());
    cx.bind_keys([
        KeyBinding::new("secondary-n", NewNote, Some(CONTEXT)),
        KeyBinding::new("secondary-k", OpenPalette, Some(CONTEXT)),
        KeyBinding::new("secondary-f", FocusSearch, Some(CONTEXT)),
        KeyBinding::new("secondary-,", OpenSettings, None),
    ]);

    set_menus(
        cx,
        [
            Menu::app([MenuItem::action("Settings…", OpenSettings)]),
            Menu::new("File").items([MenuItem::action("New Note", NewNote)]),
            Menu::new("View")
                .items([MenuItem::action("Command Palette", OpenPalette), MenuItem::role(MenuRole::ToggleSidebar)]),
            Menu::help([MenuItem::action("Report an Issue…", ReportIssue)]),
        ],
    );
}

/// Writes an incident bundle (tree, last inputs, frames, logs) and reveals it.
pub fn report_issue() {
    if let Some(bundle) = nd::report_issue("user report") {
        let _ = nd::system::reveal_path(&bundle);
    }
}
