//! The one crate apps depend on. Day one re-exports the seam and dispatch; the app vocabulary
//! (`Application`, `prelude`, elements) arrives with the `nd-ui` core lane.

#[cfg(feature = "build")]
pub mod build;

#[cfg(feature = "runtime")]
pub use nd_core as core;
#[cfg(feature = "runtime")]
pub use nd_ui::{actions, backend, dispatch, transaction};

#[cfg(all(feature = "runtime", target_os = "macos"))]
pub use nd_backend_appkit as appkit;
