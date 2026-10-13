//! Windows services for the GPUI backend (ARCHITECTURE.md 2, 5.1, 5.4): system backdrops, the
//! caption hit-testing that gives app-drawn caption buttons Snap Layouts, drag regions,
//! per-monitor DPI v2 and the `WM_POINTER` pen and touch path. It works on an HWND the backend
//! created, through `raw-window-handle`, and names no GPUI type.
//!
//! `caption` and `pointer` are plain data and logic, built and tested on every OS.

pub mod caption;
pub mod pointer;

#[cfg(windows)]
pub mod backdrop;
#[cfg(windows)]
pub mod chrome;
#[cfg(windows)]
pub mod dpi;

#[cfg(windows)]
pub use windows::Win32::Foundation::HWND;
