mod common;

use std::path::PathBuf;

use common::{Dirs, NotesSettings, Rich, read};
use nd_settings::{KeyState, LayerKind, SettingsHandle, WriteError, fspec, schema_json};

fn notes_app() -> fspec::App {
    fspec::App::new("notes", "https://github.com/FormalStudio/notes.git").display_name("Notes")
}

/// `ND_BLESS=1 cargo test -p nd-settings` rewrites the committed fixture.
#[test]
fn committed_star_matches_the_rust_struct() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/notes/fspec.star");
    let current = fspec::star_module(&notes_app(), &schema_json::<NotesSettings>());
    if std::env::var_os("ND_BLESS").is_some() {
        std::fs::write(&path, &current).unwrap();
    }
    let committed = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(committed == current, "{} is stale, run with ND_BLESS=1", path.display());
}

#[test]
fn star_flattens_sections_and_types_each_key() {
    let star = fspec::star_module(&fspec::App::new("rich", "https://example.com/rich"), &schema_json::<Rich>());
    assert!(star.contains("_SECTIONS = [\"editor\"]"), "{star}");
    assert!(star.contains("\"editor.tab_width\": {\"t\": \"int\", \"min\": 1, \"max\": 8},"), "{star}");
    assert!(star.contains("\"tags\": {\"t\": \"list\", \"of\": {\"t\": \"str\"}},"), "{star}");
    assert!(star.contains("\"proxy\": {\"t\": \"str\"},"), "{star}");
    assert!(star.contains("\"dark-mode\": {\"t\": \"bool\"},"), "{star}");
    assert!(star.contains("\ndef rich(settings = {}"), "{star}");
    assert!(star.contains("cargo build --release --locked -p rich"), "{star}");
}

#[test]
fn literals_are_starlark() {
    let value: toml::Value =
        toml::from_str::<toml::Table>("v = { a = true, b = [1, 2.5], c = \"x\\\"y\" }").unwrap()["v"].clone();
    assert_eq!(fspec::literal(&value), r#"{"a": True, "b": [1, 2.5], "c": "x\"y"}"#);
    assert_eq!(fspec::option_path("notes", "editor.font_size"), r#"notes(settings = {"editor": {"font_size": ...}})"#);
}

#[test]
fn fspec_file_wins_over_the_user_file_and_its_keys_are_read_only() {
    // docs:begin fspec
    let dirs = Dirs::new();
    dirs.write("config.toml", "editor_font_size = 13\ncompact_rows = true\n");
    dirs.write("fspec.toml", "editor_font_size = 20\n");

    let settings = SettingsHandle::<NotesSettings>::load(dirs.options("notes")).unwrap();
    assert_eq!(settings.get().editor_font_size, 20.0);
    assert_eq!(settings.source("editor_font_size").unwrap().kind, LayerKind::Fspec);
    assert!(matches!(settings.key_state("editor_font_size"), KeyState::ManagedByFspec { .. }));
    assert!(matches!(settings.set("editor_font_size", &12.0), Err(WriteError::ManagedByFspec { .. })));
    assert_eq!(settings.copy_as_fspec("editor_font_size").unwrap(), r#"settings = {"editor_font_size": 20}"#);

    // Keys the spec leaves alone stay the user's, in config.toml.
    assert_eq!(settings.key_state("compact_rows"), KeyState::Editable);
    settings.set("compact_rows", &false).unwrap();
    assert!(read(&dirs.user()).contains("compact_rows = false"));
    // docs:end fspec
}
