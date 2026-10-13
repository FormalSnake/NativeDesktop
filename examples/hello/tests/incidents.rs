//! M1 incident gates (ARCHITECTURE.md 9.3, 9.4, 16) against the real `hello` binary. Each test
//! launches `hello` with its own `ND_APP_DATA_DIR` and a debug trigger (`ND_DEBUG_PANIC`,
//! `ND_DEBUG_HANG_MS`, `ND_DEBUG_CRASH`, see `nd-backend-appkit/src/diag.rs`) and asserts the
//! bundle it leaves. They open a real window, so they are tier 3 (`real_`): `just gate m1` or
//! `cargo nextest run --profile real -p hello`.

#![cfg(target_os = "macos")]

mod gate_m1 {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, ExitStatus};
    use std::time::{Duration, Instant};

    use serde_json::Value;

    fn launch(data: &Path, envs: &[(&str, &str)]) -> Child {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_hello"));
        cmd.env("ND_APP_DATA_DIR", data).env_remove("ND_DIAG_CRASH_HANDLER");
        for (k, v) in envs {
            cmd.env(k, v);
        }
        cmd.spawn().expect("launch hello")
    }

    fn wait(child: &mut Child, limit: Duration) -> ExitStatus {
        let start = Instant::now();
        loop {
            if let Some(status) = child.try_wait().expect("wait") {
                return status;
            }
            if start.elapsed() > limit {
                let _ = child.kill();
                panic!("hello still running after {limit:?}");
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn bundles(data: &Path, kind: &str) -> Vec<PathBuf> {
        let Ok(entries) = fs::read_dir(data.join("diagnostics")) else { return Vec::new() };
        let suffix = format!("-{kind}");
        let mut found: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir() && p.file_name().is_some_and(|n| n.to_string_lossy().ends_with(&suffix)))
            .collect();
        found.sort();
        found
    }

    fn json(path: &Path) -> Value {
        serde_json::from_slice(&fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display())))
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }

    fn lines(path: &Path) -> Vec<Value> {
        fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
            .lines()
            .map(|l| serde_json::from_str(l).expect("ndjson line"))
            .collect()
    }

    fn find<'a>(node: &'a Value, pred: &dyn Fn(&Value) -> bool) -> Option<&'a Value> {
        if pred(node) {
            return Some(node);
        }
        node["children"].as_array()?.iter().find_map(|c| find(c, pred))
    }

    fn assert_minidump(dir: &Path) {
        let dump = fs::read(dir.join("minidump.dmp")).expect("minidump.dmp");
        assert!(dump.starts_with(b"MDMP"), "minidump.dmp is not a minidump ({} bytes)", dump.len());
    }

    /// The committed semantic tree (the automation schema), the native hierarchy, the profile, layout diffs and meta: what every
    /// bundle carries besides its primary file.
    fn assert_snapshot(dir: &Path, kind: &str) {
        let tree = json(&dir.join("tree.json"));
        let root = &tree["windows"][0]["root"];
        assert_eq!(tree["windows"][0]["title"], "Hello");
        let button = find(root, &|n| n["role"] == "button").expect("button in tree.json");
        assert_eq!(button["name"], "Add one");
        assert_eq!(button["testId"], "add");
        assert!(find(root, &|n| n["role"] == "text" && n["name"] == "0").is_some(), "counter text in tree.json");
        assert!(button["bounds"]["width"].as_f64().is_some_and(|w| w > 0.), "button has bounds: {button}");
        assert!(button["ref"].is_u64(), "nodes carry their automation ref");

        let profile = json(&dir.join("profile.json"));
        let window = profile["windows"].as_object().and_then(|w| w.values().next()).expect("profile window");
        assert!(window["summary"]["frames"].as_u64() >= Some(1), "frames in profile.json");
        let frame = &window["frames"][0];
        assert!(frame["interval_ms"].as_f64().is_some_and(|i| i > 0.), "frame interval from the display clock");

        let layout = lines(&dir.join("frames/layout.ndjson"));
        assert!(
            layout.first().is_some_and(|d| d["added"].as_array().is_some_and(|a| a.len() >= 3)),
            "first layout diff adds the tree"
        );

        let meta = json(&dir.join("meta.json"));
        assert_eq!(meta["app"], "hello");
        assert_eq!(meta["kind"], kind);
        assert_eq!(meta["backend"], "appkit");

        let log = lines(&dir.join("log.ndjson"));
        assert!(log.iter().any(|l| l["message"] == "diagnostics ready"), "log ring");
    }

    #[test]
    fn real_panic_bundle() {
        let data = tempfile::tempdir().expect("tempdir");
        let mut child = launch(data.path(), &[("ND_DEBUG_PANIC", "1")]);
        let status = wait(&mut child, Duration::from_secs(60));
        assert_eq!(status.code(), Some(101), "hello exits 101 after a panic");
        let found = bundles(data.path(), "panic");
        assert_eq!(found.len(), 1, "one panic bundle: {found:?}");
        let dir = &found[0];

        let panic = fs::read_to_string(dir.join("panic.txt")).expect("panic.txt");
        assert!(panic.contains("ND_DEBUG_PANIC: forced panic"), "{panic}");
        assert!(panic.contains("panicked at"), "{panic}");
        assert!(panic.contains("after_first_frame"), "backtrace names the trigger:\n{panic}");
        assert_snapshot(dir, "panic");

        let tree = json(&dir.join("tree.json"));
        let native = &tree["native"]["windows"][0];
        assert!(native["content"]["class"].as_str().is_some_and(|c| c.contains("Nd")), "native hierarchy: {native}");

        let inputs = lines(&dir.join("input.ndjson"));
        let kinds: Vec<&str> = inputs.iter().filter_map(|i| i["kind"].as_str()).collect();
        assert_eq!(kinds, ["mouse_down", "mouse_up", "key_up"]);
        assert_eq!(inputs[2]["text"], "n", "debug builds keep typed text");
        assert!(inputs[0]["position"].is_array());

        let log = lines(&dir.join("log.ndjson"));
        assert!(
            log.iter().any(|l| l["level"] == "ERROR" && l["message"].as_str().is_some_and(|m| m.starts_with("panic:")))
        );
        assert_minidump(dir);
        assert!(fs::read_dir(data.path().join("logs")).expect("logs dir").count() >= 1, "rolling log file");
    }

    #[test]
    fn real_hang_bundle() {
        let data = tempfile::tempdir().expect("tempdir");
        let mut child = launch(data.path(), &[("ND_DEBUG_HANG_MS", "3000"), ("ND_DIAG_HANG_MS", "500")]);
        let start = Instant::now();
        let dir = loop {
            if let Some(dir) = bundles(data.path(), "hang").pop()
                && dir.join("meta.json").exists()
                && dir.join("minidump.dmp").exists()
            {
                break dir;
            }
            assert!(child.try_wait().expect("wait").is_none(), "hello exited during the hang");
            assert!(start.elapsed() < Duration::from_secs(60), "no hang bundle");
            std::thread::sleep(Duration::from_millis(100));
        };
        // The dump is written while the app is suspended; give the file a moment to settle.
        std::thread::sleep(Duration::from_millis(500));
        let hang = json(&dir.join("hang.json"));
        assert_eq!(hang["thread"], "main");
        assert!(hang["blocked_ms"].as_u64() >= Some(500), "{hang}");
        assert_snapshot(&dir, "hang");
        assert_minidump(&dir);
        assert!(child.try_wait().expect("wait").is_none(), "a hang does not end the app");
        let _ = child.kill();
        let _ = child.wait();
    }

    #[test]
    fn real_crash_bundle() {
        let data = tempfile::tempdir().expect("tempdir");
        let mut child = launch(data.path(), &[("ND_DEBUG_CRASH", "1")]);
        let status = wait(&mut child, Duration::from_secs(60));
        assert_eq!(status.code(), Some(70), "hello exits 70 after a dumped crash");
        let start = Instant::now();
        let dir = loop {
            if let Some(dir) = bundles(data.path(), "crash").pop()
                && dir.join("crash.json").exists()
            {
                break dir;
            }
            assert!(start.elapsed() < Duration::from_secs(10), "no crash bundle");
            std::thread::sleep(Duration::from_millis(100));
        };
        let crash = json(&dir.join("crash.json"));
        assert_eq!(crash["minidump"], true, "{crash}");
        assert_minidump(&dir);
        assert_snapshot(&dir, "crash");
    }
}
