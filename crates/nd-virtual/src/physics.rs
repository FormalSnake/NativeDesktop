//! The seam drawn presenters scroll through. Native scroll views (NSScrollView) bring their own
//! momentum and rubber band; GPUI presenters turn wheel and touchpad deltas into offsets here.
//! [`Direct`] applies deltas as they come, which is what the platform already smoothed on macOS
//! and Windows. Framework momentum and rubber band for Wayland touchpads come from `nd-gesture`
//! through this trait.

/// Where a wheel delta sits in a touchpad gesture.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WheelPhase {
    /// A notched wheel or a device without phases.
    #[default]
    None,
    Started,
    Moved,
    Ended,
    /// Momentum the OS generated after the fingers lifted.
    Momentum,
}

pub trait ScrollPhysics {
    /// The scroll offset change for one wheel event of `delta` points (positive towards the
    /// end), at `now` seconds on the frame clock.
    fn wheel(&mut self, delta: f64, phase: WheelPhase, now: f64) -> f64;

    /// Offset change for the frame at `now` while a fling or bounce runs; `None` when idle.
    fn tick(&mut self, _now: f64) -> Option<f64> {
        None
    }

    /// The offset has hit an end of the content; momentum stops or bounces there.
    fn hit_edge(&mut self) {}
}

/// Deltas applied unchanged.
#[derive(Clone, Copy, Debug, Default)]
pub struct Direct;

impl ScrollPhysics for Direct {
    fn wheel(&mut self, delta: f64, _phase: WheelPhase, _now: f64) -> f64 {
        delta
    }
}
