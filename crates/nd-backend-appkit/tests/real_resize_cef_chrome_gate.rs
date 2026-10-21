//! The CEF resize gate (`real_resize_cef_gate.rs`) with the page in Chrome style
//! (`ND_CEF_STYLE=chrome`): Chromium's own views, lifted into the page's view, resize in the
//! window's transaction too. Prints `ND_M9_RESIZE_CEF_CHROME_GATE_OK`. Needs `browser` built with
//! the `cef` feature (`ND_BROWSER_BIN` overrides).

mod harness;

fn main() {
    #[cfg(target_os = "macos")]
    harness::main_real("gate_m8_webview::real_resize_cef_chrome", || {
        use std::path::PathBuf;
        let bin = std::env::var_os("ND_BROWSER_BIN")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/browser"));
        harness::resize::run(harness::resize::Target {
            name: "gate_m8_webview::real_resize_cef_chrome",
            marker: "ND_M9_RESIZE_CEF_CHROME_GATE",
            bin,
            env: vec![("BROWSER_SCENE", "resize"), ("ND_WEBVIEW_ENGINE", "cef"), ("ND_CEF_STYLE", "chrome")],
            lights: false,
            page: true,
        })
    });
    #[cfg(not(target_os = "macos"))]
    harness::main_real("gate_m8_webview::real_resize_cef_chrome", || {});
}
