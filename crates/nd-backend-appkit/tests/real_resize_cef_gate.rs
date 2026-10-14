//! Resize gate with a live page (ARCHITECTURE.md 5.2, 8, 16), tier 3: the M0 real-drag gate on
//! `examples/browser` showing one flat page filling the window (`BROWSER_SCENE=resize`) on
//! CEF (`ND_WEBVIEW_ENGINE=cef`). In every presented frame of a live resize the window
//! background never shows: the page's view moves in the window's transaction. How often the page
//! content kept up to the edge is reported, not gated. Prints `ND_M9_RESIZE_CEF_GATE_OK`. Needs
//! `browser` built with the `cef` feature (`ND_BROWSER_BIN` overrides).

mod harness;

fn main() {
    #[cfg(target_os = "macos")]
    harness::main_real("gate_m8_webview::real_resize_cef", || {
        use std::path::PathBuf;
        let bin = std::env::var_os("ND_BROWSER_BIN")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/browser"));
        harness::resize::run(harness::resize::Target {
            name: "gate_m8_webview::real_resize_cef",
            marker: "ND_M9_RESIZE_CEF_GATE",
            bin,
            env: vec![("BROWSER_SCENE", "resize"), ("ND_WEBVIEW_ENGINE", "cef")],
            lights: false,
            page: true,
        })
    });
    #[cfg(not(target_os = "macos"))]
    harness::main_real("gate_m8_webview::real_resize_cef", || {});
}
