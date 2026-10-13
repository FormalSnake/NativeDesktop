use std::fs;
use std::time::{Duration, Instant};

use nd_settings::Store;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
struct Machine {
    last_folder: String,
    recents: Vec<String>,
}

fn wait_for(what: &str, mut ok: impl FnMut() -> bool) {
    let start = Instant::now();
    while !ok() {
        assert!(start.elapsed() < Duration::from_secs(5), "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn dir_entries(dir: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> =
        fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into()).collect();
    names.sort();
    names
}

#[test]
fn writes_are_debounced_and_leave_no_temp_files() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.json");
    let store = Store::<Machine>::open(&path, 1, None);
    assert!(!path.exists(), "opening does not write");
    for i in 0..20 {
        store.update(|m| m.recents.push(format!("note-{i}")));
    }
    assert!(!path.exists(), "a burst of updates has not hit the disk yet");
    wait_for("debounced write", || path.exists());
    let doc: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(doc["version"], 1);
    assert_eq!(doc["data"]["recents"].as_array().unwrap().len(), 20);
    assert_eq!(dir_entries(dir.path()), ["state.json"]);
}

#[test]
fn drop_flushes_pending_changes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.json");
    {
        let store = Store::<Machine>::open(&path, 1, None);
        store.update(|m| m.last_folder = "Inbox".into());
    }
    let reopened = Store::<Machine>::open(&path, 1, None);
    assert_eq!(reopened.get().last_folder, "Inbox");
    assert!(reopened.load_error().is_none());
}

#[test]
fn old_versions_migrate_on_load_and_are_rewritten() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.json");
    fs::write(&path, json!({ "version": 1, "data": { "folder": "Inbox" } }).to_string()).unwrap();
    fn migrate(from: u32, mut data: Value) -> Result<Value, String> {
        assert_eq!(from, 1);
        let folder = data["folder"].take();
        Ok(json!({ "last_folder": folder }))
    }
    let store = Store::<Machine>::open(&path, 2, Some(migrate));
    assert_eq!(store.get().last_folder, "Inbox");
    store.flush().unwrap();
    let doc: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(doc["version"], 2);
    assert_eq!(doc["data"]["last_folder"], "Inbox");
}

#[test]
fn corrupt_files_are_moved_aside_and_defaults_win() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.json");
    fs::write(&path, "{ not json").unwrap();
    let store = Store::<Machine>::open(&path, 1, None);
    assert_eq!(*store.get(), Machine::default());
    assert!(store.load_error().unwrap().contains("not JSON"));
    let names = dir_entries(dir.path());
    assert!(names.iter().any(|n| n.starts_with("state.json.corrupt-")), "{names:?}");

    // A newer file than this build understands is also kept, not overwritten.
    let newer = dir.path().join("newer.json");
    fs::write(&newer, json!({ "version": 9, "data": {} }).to_string()).unwrap();
    let store = Store::<Machine>::open(&newer, 1, None);
    assert!(store.load_error().unwrap().contains("newer"));
}
