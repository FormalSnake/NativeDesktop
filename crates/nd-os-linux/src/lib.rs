//! Linux OS services for the GPUI backend (ARCHITECTURE.md 2, 5.4). M2: window chrome. The
//! decoration mode and `wm_capabilities` decide whether the app draws window controls at all
//! ([`chrome`]); [`window`] reads them from a GPUI window; [`LinuxHooks`] plugs both into the
//! backend's `OsHooks` seam along with `wp_presentation` frame times.

pub mod chrome;
mod hooks;
pub mod window;

pub use hooks::{LinuxHooks, TITLEBAR_HEIGHT};
