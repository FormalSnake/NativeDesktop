//! M1 gate: `nd new` produces an app that builds and whose generated test passes.

mod gate_m1 {
    use std::path::Path;
    use std::process::Command;

    fn run(dir: &Path, program: &str, args: &[&str]) {
        let output = Command::new(program).args(args).current_dir(dir).output().expect("spawn");
        assert!(
            output.status.success(),
            "{program} {args:?} failed\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn nd_new_builds_and_its_test_passes() {
        let nd = env!("CARGO_BIN_EXE_nd");
        let nd_crate = Path::new(env!("CARGO_MANIFEST_DIR")).join("../nd");
        let base = Path::new(env!("CARGO_TARGET_TMPDIR")).join("nd-new-gate");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).expect("temp dir");

        run(&base, nd, &["new", "demo", "--standalone", "--nd-path", nd_crate.to_str().expect("utf-8")]);
        let demo = base.join("demo");
        run(&demo, "cargo", &["test"]);
        run(&demo, nd, &["check"]);
    }
}
