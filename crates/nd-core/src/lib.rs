//! GPUI-free basics shared by every nd crate.

pub mod clock;
pub mod geometry;
mod ids;
pub mod paths;
pub mod semantic;

pub use clock::FrameTime;
pub use geometry::{Point, Rect, Size};
pub use ids::{NodeId, WindowId};
