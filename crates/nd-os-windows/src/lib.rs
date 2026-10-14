//! Windows services for the GPUI backend (ARCHITECTURE.md 2, 5.1, 5.4): system backdrops, the
//! caption hit-testing that gives app-drawn caption buttons Snap Layouts, drag regions,
//! per-monitor DPI v2 and the `WM_POINTER` pen and touch path. It works on an HWND the backend
//! created, through `raw-window-handle`, and names no GPUI type. It also re-exports the backend
//! for `nd`, which reaches it only through this crate so that an app's build script (`nd` with
//! `build` alone) compiles no GPUI.
//!
//! `caption` and `pointer` are plain data and logic, built and tested on every OS.

pub mod caption;
pub mod pointer;

#[cfg(windows)]
pub mod backdrop;
#[cfg(windows)]
pub mod chrome;
#[cfg(windows)]
pub mod colors;
#[cfg(windows)]
pub mod dpi;
#[cfg(windows)]
mod hooks;

#[cfg(windows)]
pub use hooks::WindowsHooks;

#[cfg(windows)]
pub use nd_backend_gpui;
#[cfg(windows)]
pub use windows::Win32::Foundation::HWND;
