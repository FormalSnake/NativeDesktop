//! The theme file watcher on the real file system: a direct write, a half-written file that
//! must keep the last theme, and an atomic rename that replaces the inode.

use std::sync::mpsc;
use std::time::Duration;

use nd_theme::{MatugenTheme, ThemeFileWatcher};

fn json(primary: &str) -> String {
    format!(r##"{{"colors":{{"primary":{{"light":"{primary}","dark":"{primary}"}}}}}}"##)
}

fn accent(theme: &MatugenTheme) -> String {
    nd_theme::color::to_hex(theme.palettes.light.accent.expect("accent"))
}

#[test]
fn reads_writes_skips_half_files_and_survives_renames() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("theme.json");
    std::fs::write(&path, json("#111111")).unwrap();
    let (tx, rx) = mpsc::channel();
    let _watcher = ThemeFileWatcher::spawn(&path, MatugenTheme::parse, move |t| tx.send(accent(&t)).unwrap()).unwrap();
    let next = || rx.recv_timeout(Duration::from_secs(3)).expect("a theme within 3 s");

    assert_eq!(next(), "#111111", "the file is read at start");

    std::fs::write(&path, json("#222222")).unwrap();
    assert_eq!(next(), "#222222", "a write is picked up");

    std::fs::write(&path, r##"{"colors":{"primary":{"light":"#33"##).unwrap();
    assert!(rx.recv_timeout(Duration::from_millis(400)).is_err(), "a half-written file keeps the last theme");

    let tmp = dir.path().join("theme.json.tmp");
    std::fs::write(&tmp, json("#444444")).unwrap();
    std::fs::rename(&tmp, &path).unwrap();
    assert_eq!(next(), "#444444", "an atomic rename is picked up");

    std::fs::write(&path, json("#555555")).unwrap();
    assert_eq!(next(), "#555555", "the watch survived the rename");
}
