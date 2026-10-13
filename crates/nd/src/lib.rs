//! The one crate apps depend on. Day one re-exports the seam and dispatch; the app vocabulary
//! (`Application`, `prelude`, elements) arrives with the `nd-ui` core lane.

pub use nd_core as core;
pub use nd_ui::{actions, backend, dispatch, transaction};

#[cfg(target_os = "macos")]
pub use nd_backend_appkit as appkit;
