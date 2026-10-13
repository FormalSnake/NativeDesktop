//! The app vocabulary. Day one carries the backend seam (3.15) and nd-owned focus and dispatch
//! (3.12); views, the reconciler and layout land in the `nd-ui` core lane.

pub mod actions;
pub mod backend;
pub mod dispatch;
#[cfg(feature = "test-support")]
pub mod test_backend;
pub mod transaction;

pub use nd_core::{NodeId, Point, Rect, Size, WindowId};
pub use nd_gpui::gpui::{Action, KeyBinding, KeyContext, Keystroke, Modifiers};
