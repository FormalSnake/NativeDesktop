//! The backend [`crate::System::native`] uses on each target.

use crate::{AppIdentity, Backend};

#[cfg(not(target_arch = "wasm32"))]
mod desktop;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod mac;
#[cfg(any(target_os = "linux", windows))]
mod notify_rust;
#[cfg(target_arch = "wasm32")]
mod web;
#[cfg(windows)]
mod windows;

/// Native state read back for tier 2 tests.
#[doc(hidden)]
#[cfg(target_os = "macos")]
pub use mac::probe;

#[cfg(target_os = "macos")]
pub fn backend(app: &AppIdentity) -> impl Backend {
    mac::MacBackend::new(app)
}

#[cfg(target_os = "linux")]
pub fn backend(app: &AppIdentity) -> impl Backend {
    linux::LinuxBackend::new(app)
}

#[cfg(windows)]
pub fn backend(app: &AppIdentity) -> impl Backend {
    windows::WindowsBackend::new(app)
}

#[cfg(target_arch = "wasm32")]
pub fn backend(_app: &AppIdentity) -> impl Backend {
    web::WebBackend
}
