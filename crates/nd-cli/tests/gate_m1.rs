//! M1 gate: `nd new` produces an app that builds and whose generated test passes, generated
//! outside any cargo workspace or config of nd's, as an app's own repository is.

mod gate_m1 {
    use std::hash::{DefaultHasher, Hash, Hasher};
    use std::path::Path;
    use std::process::Command;

    fn run(dir: &Path, program: &str, args: &[&str], target: &Path) {
        let output =
            Command::new(program).args(args).current_dir(dir).env("CARGO_TARGET_DIR", target).output().expect("spawn");
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
        // One directory per checkout, kept between runs so its target builds incrementally.
        let mut hasher = DefaultHasher::new();
        env!("CARGO_MANIFEST_DIR").hash(&mut hasher);
        let base = std::env::temp_dir().join(format!("nd-new-gate-{:x}", hasher.finish()));
        let target = base.join("target");
        let demo = base.join("demo");
        let _ = std::fs::remove_dir_all(&demo);
        std::fs::create_dir_all(&base).expect("temp dir");

        run(&base, nd, &["new", "demo", "--nd-path", nd_crate.to_str().expect("utf-8")], &target);
        run(&demo, "cargo", &["test"], &target);
        run(&demo, nd, &["check"], &target);
    }
}
