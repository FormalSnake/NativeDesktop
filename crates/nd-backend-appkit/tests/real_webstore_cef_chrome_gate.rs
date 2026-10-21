//! Chrome Web Store installs: `examples/browser`'s `webstore` scene on a fresh profile, over the
//! network. One extension installs by id, one through the store page's "Add to Chrome" with the
//! app answering, and one the app cancels; Chromium's own prompt never shows. Prints
//! `ND_WEBSTORE_CEF_CHROME_GATE_OK`. Needs `browser` built with the `cef` feature
//! (`ND_BROWSER_BIN` overrides).

mod harness;

fn main() {
    #[cfg(target_os = "macos")]
    harness::main_real("gate_m8_webview::real_webstore_cef_chrome", || {
        use std::path::PathBuf;
        use std::process::Command;

        let bin = std::env::var_os("ND_BROWSER_BIN")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/browser"));
        let profile = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/nd-webstore-gate-profile");
        let _ = std::fs::remove_dir_all(&profile);
        let out = Command::new(&bin)
            .env("BROWSER_SCENE", "webstore")
            .env("ND_WEBVIEW_ENGINE", "cef")
            .env("ND_CEF_STYLE", "chrome")
            .env("ND_CEF_CACHE", &profile)
            .output()
            .unwrap_or_else(|e| panic!("{}: {e}", bin.display()));
        let err = String::from_utf8_lossy(&out.stderr);
        match err.lines().find(|l| l.starts_with("ND_WEBSTORE_OK") || l.starts_with("ND_WEBSTORE_FAILED")) {
            Some(l) if l.starts_with("ND_WEBSTORE_OK") => {
                println!("ND_WEBSTORE_CEF_CHROME_GATE_OK {}", l.trim_start_matches("ND_WEBSTORE_OK").trim());
                std::process::exit(0);
            }
            line => {
                eprintln!("{err}");
                eprintln!("ND_WEBSTORE_CEF_CHROME_GATE_FAILED {}", line.unwrap_or("no verdict"));
                std::process::exit(1);
            }
        }
    });
    #[cfg(not(target_os = "macos"))]
    harness::main_real("gate_m8_webview::real_webstore_cef_chrome", || {});
}
