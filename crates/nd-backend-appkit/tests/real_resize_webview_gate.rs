//! M4 resize gate with a native surface (ARCHITECTURE.md 5.2, 8, 16), tier 3: the M0 real-drag
//! gate on `examples/overlays` showing only a WKWebView page filling the window
//! (`OVERLAYS_RESIZE=1`). In every presented frame of a live resize the window background never
//! shows: the web view's frame moves in the window's transaction. How often WebKit's page content
//! kept up to the edge is reported, not gated (it paints on its own timeline). Prints
//! `ND_M4_RESIZE_WEBVIEW_GATE_OK`. Needs `overlays` built (`ND_OVERLAYS_BIN` overrides).

mod harness;

fn main() {
    #[cfg(target_os = "macos")]
    harness::main_real("gate_m4_overlays::real_resize_with_webview", || {
        use std::path::PathBuf;
        let bin = std::env::var_os("ND_OVERLAYS_BIN")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/overlays"));
        harness::resize::run(harness::resize::Target {
            name: "gate_m4_overlays::real_resize_with_webview",
            marker: "ND_M4_RESIZE_WEBVIEW_GATE",
            bin,
            env: vec![("OVERLAYS_RESIZE", "1")],
            lights: false,
            page: true,
        })
    });
    #[cfg(not(target_os = "macos"))]
    harness::main_real("gate_m4_overlays::real_resize_with_webview", || {});
}
