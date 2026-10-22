//! Tier 1: bundles, retention and the hang watchdog on a `Diag` rooted in a temp dir, with
//! nothing installed process-wide.

use std::fs;
use std::path::Path;
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime};

use nd_core::semantic::{Role, SemanticNode, SemanticWindow};
use nd_core::{NodeId, Rect, WindowId};
use nd_diag::{Config, Diag, FrameRecord, InputRecord, Kind, Retention, enforce_retention};
use serde_json::Value;

fn diag(dir: &Path, hang: Duration) -> Diag {
    let mut config = Config::new("diag-test", "1.2.3", "test");
    config.data_dir = dir.to_owned();
    config.hang_threshold = hang;
    config.crash_handler = false;
    Diag::new(config).expect("diag")
}

fn bundles(diag: &Diag, kind: &str) -> Vec<std::path::PathBuf> {
    let mut found: Vec<_> = fs::read_dir(diag.diagnostics_dir())
        .expect("diagnostics dir")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.to_string_lossy().ends_with(&format!("-{kind}")))
        .collect();
    found.sort();
    found
}

fn node(id: u64, role: Role, name: &str, bounds: Rect, children: Vec<SemanticNode>) -> SemanticNode {
    SemanticNode {
        id: NodeId(id),
        role,
        kind: "view".into(),
        name: name.into(),
        value: None,
        test_id: None,
        placeholder: None,
        enabled: true,
        focused: false,
        checked: None,
        selected: None,
        expanded: None,
        bounds: Some(bounds),
        visible: true,
        children,
    }
}

fn window(button_x: f32, with_label: bool) -> SemanticWindow {
    let mut children = vec![node(2, Role::Button, "Save", Rect::new(button_x, 10., 80., 24.), vec![])];
    if with_label {
        children.push(node(3, Role::Text, "Saved", Rect::new(10., 40., 50., 16.), vec![]));
    }
    SemanticWindow {
        id: WindowId(1),
        title: "Main".into(),
        key: true,
        main: true,
        visible: true,
        frame: None,
        tab_group: None,
        root: node(1, Role::Window, "", Rect::new(0., 0., 400., 300.), children),
    }
}

fn frame(n: u64) -> FrameRecord {
    FrameRecord { frame: n, target: n as f64 / 60., interval_ms: 16.7, work_ms: n as f64, missed: false }
}

fn lines(path: &Path) -> Vec<Value> {
    fs::read_to_string(path).expect("read").lines().map(|l| serde_json::from_str(l).expect("ndjson")).collect()
}

#[test]
fn report_bundle_carries_tree_inputs_profile_and_layout_diffs() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let diag = diag(tmp.path(), Duration::from_secs(5));
    diag.record_input(InputRecord {
        t: 1.,
        window: 7,
        kind: "key_down".into(),
        text: Some("ab".into()),
        ..Default::default()
    });
    diag.record_frame(1, frame(1), Some(&window(10., false)));
    diag.record_frame(1, frame(2), Some(&window(30., true)));
    diag.log(serde_json::json!({ "level": "INFO", "message": "hello" }));
    let dir = diag.report_issue("the save button moved").expect("bundle");

    assert_eq!(fs::read_to_string(dir.join("report.txt")).unwrap(), "the save button moved");
    let tree: Value = serde_json::from_slice(&fs::read(dir.join("tree.json")).unwrap()).unwrap();
    let button = &tree["windows"][0]["root"]["children"][0];
    assert_eq!(button["role"], "button");
    assert_eq!(button["ref"], 2);
    assert_eq!(button["bounds"]["x"], 30.0);

    let inputs = lines(&dir.join("input.ndjson"));
    assert_eq!(inputs.len(), 1);
    assert_eq!(inputs[0]["text"], if cfg!(debug_assertions) { "ab" } else { "\u{2022}\u{2022}" });

    let layout = lines(&dir.join("frames/layout.ndjson"));
    assert_eq!(layout.len(), 2);
    assert_eq!(layout[0]["added"].as_array().unwrap().len(), 2);
    assert_eq!(layout[1]["added"][0]["id"], 3);
    assert_eq!(layout[1]["changed"][0]["id"], 2);
    assert_eq!(layout[1]["changed"][0]["to"][0], 30.0);

    let profile: Value = serde_json::from_slice(&fs::read(dir.join("profile.json")).unwrap()).unwrap();
    assert_eq!(profile["windows"]["1"]["summary"]["frames"], 2);
    assert_eq!(profile["windows"]["1"]["summary"]["work_ms_max"], 2.0);

    let meta: Value = serde_json::from_slice(&fs::read(dir.join("meta.json")).unwrap()).unwrap();
    assert_eq!(meta["app"], "diag-test");
    assert_eq!(meta["kind"], "report");
    assert!(lines(&dir.join("log.ndjson")).iter().any(|l| l["message"] == "hello"));
}

#[test]
fn input_ring_keeps_the_last_200() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let diag = diag(tmp.path(), Duration::from_secs(5));
    for i in 0..250 {
        diag.record_input(InputRecord { t: i as f64, kind: "scroll".into(), ..Default::default() });
    }
    assert_eq!(diag.input_count(), 200);
    assert_eq!(diag.snapshot().inputs[0].t, 50.);
}

/// A panic on a worker thread ends the process with status 101 and runs no exit handlers: those
/// tear down libraries, the GPU driver among them, that the main thread is still drawing with.
#[cfg(unix)]
#[test]
fn worker_panic_exits_101_without_exit_handlers() {
    const CHILD: &str = "ND_DIAG_PANIC_CHILD";
    if let Some(dir) = std::env::var_os(CHILD) {
        extern "C" fn handler() {
            eprintln!("exit handler ran");
        }
        let diag: &'static Diag = Box::leak(Box::new(diag(Path::new(&dir), Duration::from_secs(5))));
        diag.install_panic_hook();
        assert_eq!(unsafe { libc::atexit(handler) }, 0);
        std::thread::spawn(|| panic!("worker"));
        std::thread::sleep(Duration::from_secs(10));
        return;
    }
    let tmp = tempfile::tempdir().expect("tempdir");
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "worker_panic_exits_101_without_exit_handlers", "--nocapture", "--test-threads=1"])
        .env(CHILD, tmp.path())
        .output()
        .expect("child runs");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(101), "{stderr}");
    assert!(!stderr.contains("exit handler ran"), "{stderr}");
    assert_eq!(bundles(&diag(tmp.path(), Duration::from_secs(5)), "panic").len(), 1);
}

mod gate_m1 {
    use super::*;

    #[test]
    fn retention_deletes_bundle_21() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let diag = diag(tmp.path(), Duration::from_secs(5));
        let mut written = Vec::new();
        for i in 0..21 {
            written.push(diag.write_incident(Kind::Report, "report.txt", format!("{i}").as_bytes()).expect("bundle"));
        }
        let left = bundles(&diag, "report");
        assert_eq!(left.len(), 20);
        assert!(!written[0].exists(), "the oldest bundle went");
        assert_eq!(left, written[1..]);
    }

    #[test]
    fn retention_drops_old_and_oversized_bundles() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let diag = diag(tmp.path(), Duration::from_secs(5));
        let old = diag.report_issue("old").expect("bundle");
        let a = diag.write_incident(Kind::Report, "blob", &vec![0; 4000]).expect("bundle");
        let b = diag.write_incident(Kind::Report, "blob", &vec![0; 4000]).expect("bundle");
        let fifteen_days = SystemTime::now() - Duration::from_secs(15 * 24 * 3600);
        open_dir(&old).set_modified(fifteen_days).unwrap();
        let removed = enforce_retention(diag.diagnostics_dir(), Retention { max_bytes: 6000, ..Retention::default() })
            .expect("retention");
        assert!(!old.exists(), "older than 14 days");
        assert!(!a.exists(), "oldest goes first over the size cap");
        assert!(b.exists());
        assert_eq!(removed.len(), 2);
    }

    /// A directory handle that can change the directory's times.
    fn open_dir(dir: &std::path::Path) -> fs::File {
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            // FILE_FLAG_BACKUP_SEMANTICS: the only way to open a directory on Windows.
            fs::OpenOptions::new().write(true).custom_flags(0x0200_0000).open(dir).unwrap()
        }
        #[cfg(not(windows))]
        fs::File::open(dir).unwrap()
    }

    /// A stand-in main thread: runs posted closures in order, or blocks when told to.
    type Job = Box<dyn FnOnce() + Send>;

    fn fake_main() -> (mpsc::Sender<Job>, std::thread::JoinHandle<()>) {
        let (tx, rx) = mpsc::channel::<Job>();
        let thread = std::thread::spawn(move || {
            while let Ok(job) = rx.recv() {
                job();
            }
        });
        (tx, thread)
    }

    #[test]
    fn hang_watchdog_fires_on_a_blocked_main_thread() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let diag = diag(tmp.path(), Duration::from_millis(150));
        let (main, thread) = fake_main();
        let post = main.clone();
        let watchdog = diag.start_watchdog(Box::new(move |f| {
            let _ = post.send(f);
        }));

        std::thread::sleep(Duration::from_millis(400));
        assert!(bundles(&diag, "hang").is_empty(), "a responsive main thread is no hang");

        main.send(Box::new(|| std::thread::sleep(Duration::from_millis(800)))).unwrap();
        let start = Instant::now();
        let dir = loop {
            if let Some(dir) = bundles(&diag, "hang").pop()
                && dir.join("meta.json").exists()
            {
                break dir;
            }
            assert!(start.elapsed() < Duration::from_secs(5), "no hang bundle");
            std::thread::sleep(Duration::from_millis(20));
        };
        let hang: Value = serde_json::from_slice(&fs::read(dir.join("hang.json")).unwrap()).unwrap();
        assert_eq!(hang["thread"], "main");
        assert!(hang["blocked_ms"].as_u64().unwrap() >= 150, "{hang}");

        std::thread::sleep(Duration::from_millis(1000));
        assert_eq!(bundles(&diag, "hang").len(), 1, "one bundle per hang");
        drop(watchdog);
        drop(main);
        thread.join().unwrap();
    }
}
