mod common;

use std::path::PathBuf;

use common::{NotesSettings, Rich};
use nd_settings::{FieldKind, Reload, SettingsSchema, nix, schema_json};

fn fixture(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/notes").join(rel)
}

/// `ND_BLESS=1 cargo test -p nd-settings` rewrites the committed fixtures.
fn assert_committed(rel: &str, current: &str) {
    let path = fixture(rel);
    if std::env::var_os("ND_BLESS").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, current).unwrap();
    }
    let committed = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(committed == current, "{} is stale, run with ND_BLESS=1", path.display());
}

#[test]
fn committed_schema_and_options_match_the_rust_struct() {
    let schema = schema_json::<NotesSettings>();
    let mut text = serde_json::to_string_pretty(&schema).unwrap();
    text.push('\n');
    assert_committed("settings.schema.json", &text);
    assert_committed("nix/settings-options.nix", &nix::options_module(&schema));
}

#[test]
fn form_metadata_follows_declaration_order_and_attributes() {
    let schema = SettingsSchema::of::<NotesSettings>();
    let paths: Vec<&str> = schema.fields.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, ["compact_rows", "editor_font_size", "spellcheck", "new_note_folder"]);

    let size = schema.field("editor_font_size").unwrap();
    assert_eq!(size.kind, FieldKind::Number { min: Some(11.0), max: Some(24.0) });
    assert_eq!(size.step, Some(1.0));
    assert_eq!(size.title, "Editor font size");
    assert_eq!(size.help.as_deref(), Some("Editor text size in points, before pinch zoom."));
    assert_eq!(size.default, Some(serde_json::json!(15.0)));

    let FieldKind::Choice(choices) = &schema.field("new_note_folder").unwrap().kind else { panic!() };
    assert_eq!(choices.iter().map(|c| c.value.as_str()).collect::<Vec<_>>(), ["current", "pinned"]);
    assert_eq!(choices[0].help.as_deref(), Some("The folder shown in the sidebar."));
}

#[test]
fn sections_optionals_lists_and_restart_keys_reach_the_form() {
    let schema = SettingsSchema::of::<Rich>();
    let paths: Vec<&str> = schema.fields.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, ["name", "worker_threads", "proxy", "tags", "editor.tab_width", "editor.wrap", "dark-mode"]);
    assert_eq!(schema.sections.iter().map(|s| s.path.as_str()).collect::<Vec<_>>(), ["editor"]);
    assert_eq!(schema.field("worker_threads").unwrap().reload, Reload::Restart);
    assert_eq!(schema.field("name").unwrap().reload, Reload::Live);
    assert!(schema.field("proxy").unwrap().optional);
    assert_eq!(schema.field("tags").unwrap().kind, FieldKind::List(Box::new(FieldKind::Text)));
    assert_eq!(schema.field("editor.tab_width").unwrap().kind, FieldKind::Integer { min: Some(1.0), max: Some(8.0) });
}

#[test]
fn nix_options_cover_sections_and_quote_odd_names() {
    let module = nix::options_module(&schema_json::<Rich>());
    for expected in [
        "worker_threads = opt t.ints.unsigned",
        "proxy = opt t.str",
        "tags = opt (t.listOf t.str)",
        "editor = lib.mkOption",
        "tab_width = opt (t.ints.between 1 8)",
        "dark-mode = opt t.bool",
    ] {
        assert!(module.contains(expected), "missing `{expected}` in:\n{module}");
    }
    assert_eq!(nix::option_path("notes", "editor.tab_width"), "programs.notes.settings.editor.tab_width");
    assert_eq!(nix::option_path("notes", "needs quoting"), "programs.notes.settings.\"needs quoting\"");
}

#[test]
fn nix_literals_escape_strings_and_nest() {
    let value: toml::Value = toml::from_str("a = \"x${y}\\\"\"\nb = [1, 2.5]\n[c]\nd = true\n").unwrap();
    assert_eq!(nix::literal(&value), r#"{ a = "x\${y}\""; b = [ 1 2.5 ]; c = { d = true; }; }"#);
}
