//! The one crate apps depend on: `use nd::prelude::*;`.
//!
//! M0 state: the vocabulary, retained tree and layout are real (`nd-ui`); `scaffold`,
//! `toolbar` and the theme are composed stand-ins until `nd-components` and `nd-theme` land,
//! and `Application::run` has no platform host yet.

#[cfg(feature = "build")]
pub mod build;
#[cfg(feature = "runtime")]
mod chrome;
#[cfg(feature = "runtime")]
mod controls;
#[cfg(feature = "runtime")]
pub mod native;
#[cfg(feature = "runtime")]
mod runtime;
#[cfg(feature = "runtime")]
pub mod theme;
#[cfg(feature = "runtime")]
pub use runtime::*;

#[cfg(feature = "runtime")]
pub use nd_core as core;
#[cfg(feature = "runtime")]
pub use nd_ui::{actions, backend, dispatch, events, transaction};

#[cfg(all(feature = "runtime", target_os = "macos"))]
pub use nd_backend_appkit as appkit;
