//! The one crate apps depend on: `use nd::prelude::*;`.
//!
//! The vocabulary, retained tree and layout are `nd-ui`'s, on AppKit (macOS) and GPUI
//! (elsewhere, and on macOS with the `gpui` feature and `ND_BACKEND=gpui`); the theme is
//! `nd-theme`'s; the chrome intents, `glass()`, menus and dialogs are `nd-components`'.

#[cfg(feature = "build")]
pub mod build;
#[cfg(feature = "runtime")]
pub mod chrome;
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
pub use nd_components as components;
#[cfg(feature = "runtime")]
pub use nd_core as core;
#[cfg(feature = "runtime")]
pub use nd_ui::{actions, backend, dispatch, events, transaction};
#[cfg(feature = "runtime")]
pub use nd_update as update;

#[cfg(all(feature = "runtime", target_os = "macos"))]
pub use nd_backend_appkit as appkit;

/// Tier 1 vocabulary for app tests: `TestApp` over the recording `TestBackend`.
#[cfg(feature = "test")]
pub mod test {
    pub use nd_ui::test_app::TestApp;
    pub use nd_ui::test_backend::TestBackend;
    pub use nd_ui::{ButtonProps, NodeId, Size, TextProps};
}
