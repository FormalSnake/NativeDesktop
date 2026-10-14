use nd::prelude::*;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::model::NoteId;

actions!(notes, [NewNote, OpenPalette, OpenSettings, ToggleSidebar, ReportIssue]);

#[derive(Clone, PartialEq, Debug, Deserialize, JsonSchema, Action)]
#[action(namespace = notes)]
pub struct DeleteNote(pub NoteId);

#[derive(Clone, PartialEq, Debug, Deserialize, JsonSchema, Action)]
#[action(namespace = notes)]
pub struct PinNote(pub NoteId);

#[derive(Clone, PartialEq, Debug, Deserialize, JsonSchema, Action)]
#[action(namespace = notes)]
pub struct DuplicateNote(pub NoteId);

pub const CONTEXT: &str = "Notes";

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("secondary-n", NewNote, Some(CONTEXT)),
        KeyBinding::new("secondary-k", OpenPalette, Some(CONTEXT)),
        KeyBinding::new("secondary-,", OpenSettings, None),
        KeyBinding::new("secondary-alt-s", ToggleSidebar, Some(CONTEXT)),
    ]);

    set_menus(
        cx,
        [
            Menu::app([MenuItem::action("Settings…", OpenSettings)]),
            Menu::new("File").items([MenuItem::action("New Note", NewNote)]),
            Menu::new("View").items([
                MenuItem::action("Command Palette", OpenPalette),
                MenuItem::action("Toggle Sidebar", ToggleSidebar),
            ]),
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
