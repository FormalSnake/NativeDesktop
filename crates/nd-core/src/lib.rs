//! GPUI-free basics shared by every nd crate.

pub mod geometry;
mod ids;

pub use geometry::{Point, Rect, Size};
pub use ids::{NodeId, WindowId};
