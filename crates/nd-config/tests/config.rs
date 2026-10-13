use std::path::{Path, PathBuf};

use nd_config::Config;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn every_example_file_parses() {
    let mut seen = 0;
    for entry in std::fs::read_dir(repo().join("examples")).unwrap() {
        let path = entry.unwrap().path().join("nativedesktop.toml");
        if path.exists() {
            Config::load(&path).unwrap_or_else(|e| panic!("{e}"));
            seen += 1;
        }
    }
    assert!(seen >= 4, "found {seen} example configs");
}

#[test]
fn notes_example_fields_land_in_the_typed_tables() {
    let config = Config::load(&repo().join("examples/notes/nativedesktop.toml")).unwrap();
    assert_eq!(config.app.id, "dev.formalsnake.notes");
    assert_eq!(config.app.url_schemes, ["notes"]);
    assert_eq!(config.linux.formats, [nd_config::LinuxFormat::Appimage]);
    assert!(matches!(config.updates.publish, Some(nd_config::Publish::S3 { .. })));
}

#[test]
fn unknown_keys_name_the_file_and_the_key() {
    let err = Config::parse("[app]\nid = \"a.b\"\ndisplay_nmae = \"x\"\n", Path::new("nativedesktop.toml"))
        .unwrap_err()
        .to_string();
    assert!(err.contains("nativedesktop.toml") && err.contains("display_nmae"), "{err}");
}

#[test]
fn validation_rejects_bad_ids_and_half_configured_feeds() {
    let parse = |t: &str| Config::parse(t, Path::new("n.toml")).unwrap_err().to_string();
    assert!(parse("[app]\nid = \"notes\"\n").contains("reverse-DNS"));
    assert!(parse("").contains("app.id is required"));
    assert!(parse("[app]\nid = \"a.b\"\n[updates]\nfeed = \"https://x\"\n").contains("public_key"));
}

#[test]
fn schema_is_stable_and_marks_the_tables() {
    let schema: serde_json::Value = serde_json::from_str(&nd_config::schema_json()).unwrap();
    let props = &schema["properties"];
    for table in ["app", "patterns", "webview", "updates", "diagnostics", "macos", "windows", "linux", "web"] {
        assert!(props.get(table).is_some(), "schema lacks [{table}]");
    }
    assert_eq!(schema["additionalProperties"], false);
}
