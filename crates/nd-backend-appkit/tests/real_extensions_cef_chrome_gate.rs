//! Chrome extensions on a cold start: `examples/browser`'s `extensions` scene (a popup that fills
//! a login through a native messaging host, clicked the moment the extension list arrives) on one
//! profile, once to install the test extension and then 20 times from a cold start. Every run
//! must fill. Prints `ND_EXTENSIONS_CEF_CHROME_GATE_OK`. Needs `browser` and `nd-test-host`
//! built with the `cef` feature (`ND_BROWSER_BIN` overrides).

mod harness;

fn main() {
    #[cfg(target_os = "macos")]
    harness::main_real("gate_m8_webview::real_extensions_cef_chrome", || {
        use std::path::PathBuf;
        use std::process::Command;

        const COLD_STARTS: usize = 20;
        let bin = std::env::var_os("ND_BROWSER_BIN")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/browser"));
        let profile = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/nd-extensions-gate-profile");
        let _ = std::fs::remove_dir_all(&profile);
        let run = |label: &str| -> String {
            let out = Command::new(&bin)
                .env("BROWSER_SCENE", "extensions")
                .env("ND_WEBVIEW_ENGINE", "cef")
                .env("ND_CEF_STYLE", "chrome")
                .env("ND_CEF_CACHE", &profile)
                .output()
                .unwrap_or_else(|e| panic!("{}: {e}", bin.display()));
            let err = String::from_utf8_lossy(&out.stderr);
            let line = err.lines().find(|l| l.starts_with("ND_EXTENSIONS_OK") || l.starts_with("ND_EXTENSIONS_FAILED"));
            match line {
                Some(l) if l.starts_with("ND_EXTENSIONS_OK") => l.to_owned(),
                _ => {
                    eprintln!("{err}");
                    eprintln!("ND_EXTENSIONS_CEF_CHROME_GATE_FAILED {label}: {}", line.unwrap_or("no verdict"));
                    std::process::exit(1);
                }
            }
        };
        run("install");
        let mut worst = (0u64, 0u64);
        for i in 1..=COLD_STARTS {
            let line = run(&format!("cold start {i}"));
            let field = |name: &str| {
                line.split_whitespace()
                    .find_map(|f| f.strip_prefix(name).and_then(|v| v.parse::<u64>().ok()))
                    .unwrap_or_default()
            };
            worst.0 = worst.0.max(field("popup_ms="));
            worst.1 = worst.1.max(field("fill_ms="));
        }
        println!(
            "ND_EXTENSIONS_CEF_CHROME_GATE_OK cold_starts={COLD_STARTS} worst_popup_ms={} worst_fill_ms={}",
            worst.0, worst.1
        );
        std::process::exit(0);
    });
    #[cfg(not(target_os = "macos"))]
    harness::main_real("gate_m8_webview::real_extensions_cef_chrome", || {});
}
