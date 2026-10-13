use nd::prelude::*;
use serde::{Deserialize, Serialize};

/// Notes settings. Lives in `config.toml` under the app's config dir, can be set from Nix through
/// `programs.notes.settings`, and renders as the settings sheet below.
#[derive(Settings, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct NotesSettings {
    /// One-line rows without a preview.
    pub compact_rows: bool,
    /// Editor text size in points, before pinch zoom.
    #[settings(range = 11.0..=24.0, step = 1.0)]
    pub editor_font_size: f32,
    /// Underline misspelled words.
    pub spellcheck: bool,
    /// Where new notes start.
    pub new_note_folder: NewNoteFolder,
}

#[derive(Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum NewNoteFolder {
    /// The folder shown in the sidebar.
    Current,
    /// Always pinned.
    Pinned,
}

impl Default for NotesSettings {
    fn default() -> Self {
        Self { compact_rows: false, editor_font_size: 15.0, spellcheck: true, new_note_folder: NewNoteFolder::Current }
    }
}

pub struct SettingsSheet;

impl SettingsSheet {
    pub fn new(_window: &mut Window, _cx: &mut Context<Self>) -> Self {
        Self
    }
}

impl Render for SettingsSheet {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        sheet_layout("Settings")
            .w(px(460.))
            // Rows come from the schema: doc comments become help text, ranges become sliders,
            // enums become pickers. Keys managed by Nix show as read-only with their option path.
            .child(settings_form::<NotesSettings>())
            .footer(
                button("done", "Done")
                    .primary()
                    .on_click(cx.listener(|_, _: &ClickEvent, window, cx| window.close_sheet(cx))),
            )
    }
}
