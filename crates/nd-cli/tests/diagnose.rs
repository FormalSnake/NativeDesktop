//! `nd diagnose` against a real bundle written by `nd-diag`, through the real binary.

use std::path::Path;
use std::process::Command;

use nd_core::semantic::{Role, SemanticNode, SemanticWindow};
use nd_core::{NodeId, WindowId};
use nd_diag::{Config, Diag, FrameRecord, InputRecord, Kind};

fn node(id: u64, role: Role, kind: &str, name: &str, children: Vec<SemanticNode>) -> SemanticNode {
    SemanticNode {
        id: NodeId(id),
        role,
        kind: kind.into(),
        name: name.into(),
        value: None,
        test_id: (role == Role::Button).then(|| "add".into()),
        placeholder: None,
        enabled: true,
        focused: false,
        checked: None,
        selected: None,
        expanded: None,
        bounds: None,
        visible: true,
        children,
    }
}

const PANIC: &str = "thread 'main' panicked at examples/hello/src/lib.rs:12:5:
forced panic

   0: std::backtrace::Backtrace::force_capture
   1: nd_diag::install_panic_hook::{{closure}}
   2: std::panicking::panic_with_hook
   3: hello::after_first_frame
             at ./examples/hello/src/lib.rs:12:5
   4: nd::runtime::run
             at ./crates/nd/src/runtime.rs:90:9
   5: std::rt::lang_start
";

fn write_bundle(data: &Path) {
    let diag = Diag::new(Config::new("demo", "0.1.0", "test")).expect("diag");
    diag.log(serde_json::json!({ "level": "INFO", "target": "demo", "message": "clicked add" }));
    diag.record_input(InputRecord { kind: "mouse_down".into(), position: Some([10., 20.]), ..Default::default() });
    diag.record_input(InputRecord { kind: "key_up".into(), text: Some("n".into()), ..Default::default() });
    let window = SemanticWindow {
        id: WindowId(1),
        title: "Demo".into(),
        key: true,
        main: true,
        visible: true,
        frame: None,
        tab_group: None,
        root: node(1, Role::Window, "root", "Demo", vec![node(2, Role::Button, "Button", "Add one", vec![])]),
    };
    diag.record_frame(
        1,
        FrameRecord { frame: 1, target: 0., interval_ms: 16.6, work_ms: 2.5, missed: false },
        Some(&window),
    );
    diag.write_incident(Kind::Panic, "panic.txt", PANIC.as_bytes()).expect("bundle");
    assert!(data.join("diagnostics").is_dir());
}

fn nd(data: &Path, args: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_nd"))
        .arg("diagnose")
        .args(args)
        .args(["--app", "demo"])
        .env("ND_APP_DATA_DIR", data)
        .output()
        .expect("spawn nd");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout).expect("utf-8")
}

#[test]
fn last_summarises_the_newest_bundle() {
    let data = tempfile::tempdir().expect("tempdir");
    // SAFETY: nextest runs each test in its own process.
    unsafe { std::env::set_var("ND_APP_DATA_DIR", data.path()) };
    write_bundle(data.path());

    let text = nd(data.path(), &["--last"]);
    assert!(text.contains("panic on thread 'main' at examples/hello/src/lib.rs:12:5"), "{text}");
    assert!(text.contains("forced panic"), "{text}");
    assert!(text.contains("at hello::after_first_frame (./examples/hello/src/lib.rs:12:5)"), "{text}");
    assert!(!text.contains("force_capture"), "runtime frames are dropped:\n{text}");
    assert!(text.contains("button \"Add one\" [ref=e2] [testid=add]"), "{text}");
    assert!(text.contains("mouse_down window 0 at 10,20"), "{text}");
    assert!(text.contains("key_up window 0 \"n\""), "{text}");
    assert!(text.contains("window 1: 1 frames, 0 missed"), "{text}");
    assert!(text.contains("clicked add"), "{text}");
    assert!(text.contains(&data.path().join("diagnostics").display().to_string()), "{text}");
    assert_eq!(nd(data.path(), &[]), text, "--last is the default");

    let list = nd(data.path(), &["--list"]);
    assert!(list.trim_end().ends_with("ago") && list.contains("-panic  panic"), "{list}");
    let id = list.split_whitespace().next().expect("id");
    assert_eq!(nd(data.path(), &[id]), text, "an id shows that bundle");

    let json: serde_json::Value = serde_json::from_str(&nd(data.path(), &["--last", "--json"])).expect("json");
    assert_eq!(json["kind"], "panic");
    assert_eq!(json["incident"]["panic"]["message"], "forced panic");
    assert_eq!(json["meta"]["app"], "demo");
    assert_eq!(json["inputs"].as_array().map(Vec::len), Some(2));
}

#[test]
fn empty_data_dir_says_so() {
    let data = tempfile::tempdir().expect("tempdir");
    assert!(nd(data.path(), &["--last"]).starts_with("no incident bundles in "));
    assert_eq!(nd(data.path(), &["--last", "--json"]).trim(), "null");
}
