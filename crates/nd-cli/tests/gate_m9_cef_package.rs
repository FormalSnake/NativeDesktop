//! M9 packaging gate (ARCHITECTURE.md 8, 11.2): `examples/browser` packaged twice by
//! `nd package mac`. With the system engine the bundle holds no Chromium byte (no framework, no
//! helper, no CEF symbol in any binary); with `ND_WEBVIEW_ENGINE=cef` it carries the framework
//! and the five helper apps, passes `codesign --verify --deep --strict`, and the packaged app
//! runs the web view parity scene on Chromium.
//!
//! Release builds and Velopack: minutes, so only under nextest or with `ND_REAL=1`.

#[cfg(target_os = "macos")]
mod gate_m9_cef_package {
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::time::Duration;

    use nd_cli::package::cef::{FRAMEWORK, HELPERS, chromium_findings};

    fn allowed(name: &str) -> bool {
        let allowed = std::env::var_os("NEXTEST").is_some() || std::env::var_os("ND_REAL").is_some_and(|v| v == "1");
        if !allowed {
            println!("gate_m9_cef_package::{name}: skipped, it builds release packages");
        }
        allowed
    }

    fn browser_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/browser")
    }

    /// `nd package mac` of the browser with `engine`; the packed `.app`.
    fn package(engine: &str) -> PathBuf {
        let dir = browser_dir();
        let output = Command::new(env!("CARGO_BIN_EXE_nd"))
            .args(["package", "mac", "--dir"])
            .arg(&dir)
            .env("ND_WEBVIEW_ENGINE", engine)
            .output()
            .expect("spawn nd");
        assert!(
            output.status.success(),
            "nd package mac ({engine}) failed\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        let app = stdout.lines().find_map(|l| l.strip_prefix("app: ")).expect("nd package prints the app");
        PathBuf::from(app)
    }

    #[test]
    fn real_package_without_cef_has_no_chromium() {
        if !allowed("real_package_without_cef_has_no_chromium") {
            return;
        }
        let app = package("system");
        let findings = chromium_findings(&app);
        assert!(findings.is_empty(), "Chromium in a system-engine package:\n{}", findings.join("\n"));
        assert!(!app.join("Contents/Frameworks").join(FRAMEWORK).exists());
        println!("ND_M9_NO_CHROMIUM_OK");
    }

    #[test]
    fn real_package_with_cef_runs_chromium() {
        if !allowed("real_package_with_cef_runs_chromium") {
            return;
        }
        let app = package("cef");
        let frameworks = app.join("Contents/Frameworks");
        assert!(frameworks.join(FRAMEWORK).join("Chromium Embedded Framework").exists(), "framework missing");
        for (suffix, _) in HELPERS {
            let name = format!("Browser Helper{suffix}");
            assert!(frameworks.join(format!("{name}.app/Contents/MacOS/{name}")).exists(), "{name} missing");
        }
        let mut options = nd_test::LaunchOptions::new(app.join("Contents/MacOS/browser"))
            .env("BROWSER_SCENE", "parity")
            .env("ND_WEBVIEW_ENGINE", "cef");
        options.ready_timeout = Duration::from_secs(60);
        let handle = nd_test::launch_app(options).expect("launch the packaged browser");
        let line = handle.wait_for_stderr("ND_WEBVIEW_PARITY_", Duration::from_secs(240));
        let tail = handle.stderr_tail(40).join("\n");
        match line {
            Ok(l) if l.contains("ND_WEBVIEW_PARITY_OK engine=cef") => println!("ND_M9_CEF_PACKAGE_OK"),
            Ok(l) => panic!("{l}\n{tail}"),
            Err(e) => panic!("no parity result: {e}\n{tail}"),
        }
    }
}
