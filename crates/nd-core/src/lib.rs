//! GPUI-free basics shared by every nd crate.

pub mod clock;
pub mod geometry;
mod ids;
pub mod paths;
pub mod platform;
pub mod semantic;
pub mod time;

pub use clock::FrameTime;
pub use geometry::{Point, Rect, Size};
pub use ids::{NodeId, WindowId};
