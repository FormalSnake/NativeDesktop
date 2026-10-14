//! OS services for nd apps (ARCHITECTURE.md 2): dialogs, clipboard, notifications, recent
//! documents, credentials, shell helpers, app directories, app events, tray, media session and
//! audio playback with a spectrum. Ports the zig branch's `system.ts`, `shell.ts`, `paths.ts`
//! and audio, without the ACL: app code is trusted Rust.
//!
//! Everything goes through a [`System`]: [`System::native`] talks to the OS, [`System::fake`]
//! to a [`fake::FakeBackend`] that records calls and answers from a script, for tier 1 tests.
//! Calls that wait on the user or the OS return futures that are plain oneshot receivers, so
//! they run under GPUI's `cx.spawn`, tokio or `block_on` alike. Native work that must happen on
//! the main thread (AppKit) is posted there; a future for such a call must not be blocked on
//! from the main thread itself.
//!
//! On a target that cannot do something the call fails with [`Error::Unsupported`], and
//! [`nd_core::platform::has`] answers ahead of time.

mod backend;
mod bus;
pub mod dirs;
pub mod fake;
mod native;
mod script;
pub mod spectrum;
mod system;
mod types;

#[cfg(not(target_arch = "wasm32"))]
mod xbel;

pub use backend::{AudioControl, Backend, Emitter, Reply};
pub use bus::Events;
#[doc(hidden)]
#[cfg(target_os = "macos")]
pub use native::probe;
pub use nd_core::WindowId;
pub use nd_core::platform::{Feature, Target, Unsupported};
pub use script::DialogScript;
pub use system::*;
pub use types::*;

#[derive(Debug, Clone, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Unsupported(#[from] Unsupported),
    /// The arguments cannot be expressed on this platform (say, more alert buttons than it shows).
    #[error("invalid request: {0}")]
    Invalid(String),
    /// The clipboard, a credential or a tray item had nothing to return.
    #[error("not found")]
    NotFound,
    /// The user or the OS refused (notification permission, a locked keychain).
    #[error("denied: {0}")]
    Denied(String),
    /// The OS service answered with an error.
    #[error("{0}")]
    Platform(String),
    /// The backend dropped the request without answering (it shut down).
    #[error("the system backend dropped the request")]
    Dropped,
}

impl Error {
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub(crate) fn platform(e: impl std::fmt::Display) -> Self {
        Self::Platform(e.to_string())
    }

    pub(crate) fn unsupported(feature: Feature) -> Self {
        Self::Unsupported(Unsupported::here(feature))
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
