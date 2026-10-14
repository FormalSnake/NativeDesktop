//! M0 resize gate (ARCHITECTURE.md 5.2 and 16), tier 3. Launches `examples/hello` with its window
//! background painted magenta (`ND_DEBUG_EXPOSE=1`), captures every frame the window server
//! presents for that window through ScreenCaptureKit, and drags the window's edges with HID
//! CGEvents (`ndshell::input`), so AppKit runs a real live resize. Every captured frame must be
//! free of magenta (content covered the window to its edge). The close button is checked at the
//! configured point in stills taken after the resize, after full screen (entered and left with
//! the menu's key equivalent) and after a second resize. A control run with
//! `ND_DEBUG_DEFER_LIVE_LAYOUT=1` must fail the magenta check, which proves the detector sees a
//! late layout.
//!
//! Needs Screen Recording and Accessibility (post events) for the terminal running it, no window
//! manager that intercepts input (`ndshell::preflight` checks), and a built `hello`
//! (`cargo build -p hello`, or `ND_HELLO_BIN`). Prints `ND_M0_RESIZE_GATE_OK`. `ND_GATE_OUT=<dir>`
//! keeps an idle, a mid-drag and an after-full-screen frame as BMP.

mod harness;

fn main() {
    #[cfg(target_os = "macos")]
    harness::main_real("gate_m0::real_resize_drag", || harness::resize::run(harness::resize::Target::hello()));
    #[cfg(not(target_os = "macos"))]
    harness::main_real("gate_m0::real_resize_drag", || {});
}
