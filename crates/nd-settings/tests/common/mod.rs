#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};

use nd_settings::{Options, Paths, Settings};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Mirrors `examples/notes/src/settings.rs`, which cannot build until M6.
#[derive(Settings, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
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

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
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

/// Sections, optionals, lists, a restart-only key and a kebab-case rename.
#[derive(Settings, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Rich {
    pub name: String,
    #[settings(reload = "restart")]
    pub worker_threads: u32,
    pub proxy: Option<String>,
    pub tags: Vec<String>,
    #[settings(section)]
    pub editor: Editor,
    #[serde(rename = "dark-mode")]
    pub dark_mode: bool,
}

#[derive(Settings, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Editor {
    /// Tab width in columns.
    #[settings(range = 1..=8)]
    pub tab_width: u8,
    pub wrap: bool,
}

impl Default for Rich {
    fn default() -> Self {
        Self {
            name: "rich".into(),
            worker_threads: 2,
            proxy: None,
            tags: Vec::new(),
            editor: Editor::default(),
            dark_mode: false,
        }
    }
}

impl Default for Editor {
    fn default() -> Self {
        Self { tab_width: 4, wrap: true }
    }
}

pub struct Dirs {
    pub root: tempfile::TempDir,
    pub config: PathBuf,
    pub store: PathBuf,
}

impl Dirs {
    pub fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("config");
        let store = root.path().join("store");
        fs::create_dir_all(&config).unwrap();
        fs::create_dir_all(&store).unwrap();
        Self { root, config, store }
    }

    pub fn user(&self) -> PathBuf {
        self.config.join("config.toml")
    }

    pub fn local(&self) -> PathBuf {
        self.config.join("config.local.toml")
    }

    pub fn write(&self, rel: &str, text: &str) -> PathBuf {
        let path = self.config.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, text).unwrap();
        path
    }

    pub fn options(&self, app: &str) -> Options {
        Options::new(app).paths(Paths::in_dir(&self.config)).env(Vec::new()).store_roots(vec![self.store.clone()])
    }
}

pub fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap()
}
