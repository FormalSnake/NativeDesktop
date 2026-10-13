use nd_core::{Point, Size};

use crate::set::{Axes, Axis};
use crate::spring::{Spring, SpringMotion};
use crate::velocity::VelocityTracker;

/// UIScrollView's normal deceleration: velocity keeps this fraction per millisecond.
pub const DECELERATION_RATE: f64 = 0.998;
/// Apple's rubber band constant.
const RUBBER_BAND: f32 = 0.55;
/// Below this a fling stops, in points per second.
const MIN_VELOCITY: f64 = 8.;

/// How far past an edge content shows when pulled `overshoot` points past it, in a viewport of
/// `dimension` points. Grows ever slower and never reaches `dimension`.
pub fn rubber_band(overshoot: f32, dimension: f32) -> f32 {
    if dimension <= 0. {
        return 0.;
    }
    let sign = overshoot.signum();
    let x = overshoot.abs();
    sign * (x * dimension * RUBBER_BAND) / (dimension + RUBBER_BAND * x)
}

/// Inverse of [`rubber_band`]: the pull that shows `shown` points of overshoot.
fn rubber_band_inverse(shown: f32, dimension: f32) -> f32 {
    if dimension <= 0. {
        return 0.;
    }
    let sign = shown.signum();
    let y = shown.abs().min(dimension * 0.999);
    sign * (y * dimension) / (RUBBER_BAND * (dimension - y))
}

/// Distance a fling at `velocity` (units per second) travels before it stops, at
/// [`DECELERATION_RATE`]. Snap targets are chosen from `position + project(velocity)`.
pub fn project(velocity: f32) -> f32 {
    (-f64::from(velocity) / (1000. * DECELERATION_RATE.ln())) as f32
}

#[derive(Clone, Copy, Debug)]
enum Motion {
    Idle,
    /// Fingers on the trackpad.
    Tracking,
    /// The framework's fling: exponential decay from `x0` at `v0`, started at `t0`.
    Decay {
        x0: f64,
        v0: f64,
        t0: f64,
    },
    /// The OS sends momentum deltas itself; the framework applies them.
    OsMomentum,
    Spring(SpringMotion),
}

/// One axis of a scroller.
#[derive(Clone, Debug)]
struct AxisState {
    /// Where the fingers have pulled the content, unclamped. The shown offset is this with the
    /// part past an edge rubber banded.
    raw: f32,
    offset: f32,
    max: f32,
    viewport: f32,
    motion: Motion,
    tracker: VelocityTracker,
    last_time: f64,
}

impl AxisState {
    fn new() -> Self {
        Self {
            raw: 0.,
            offset: 0.,
            max: 0.,
            viewport: 0.,
            motion: Motion::Idle,
            tracker: VelocityTracker::default(),
            last_time: 0.,
        }
    }

    fn excess(&self, value: f32) -> f32 {
        if value < 0. {
            value
        } else if value > self.max {
            value - self.max
        } else {
            0.
        }
    }

    fn edge(&self, value: f32) -> f32 {
        value.clamp(0., self.max)
    }

    fn show_raw(&mut self) {
        let excess = self.excess(self.raw);
        self.offset = self.edge(self.raw) + rubber_band(excess, self.viewport);
    }

    fn begin(&mut self, time: f64) {
        // Grabbing mid-flight continues from what is on screen.
        let excess = self.excess(self.offset);
        self.raw = self.edge(self.offset) + rubber_band_inverse(excess, self.viewport);
        self.motion = Motion::Tracking;
        self.tracker.reset();
        self.tracker.add(time, self.raw);
        self.last_time = time;
    }

    fn track(&mut self, delta: f32, time: f64) {
        if !matches!(self.motion, Motion::Tracking) {
            self.begin(time);
        }
        self.raw += delta;
        self.tracker.add(time, self.raw);
        self.last_time = time;
        self.show_raw();
    }

    fn end(&mut self, time: f64) {
        let velocity = f64::from(self.tracker.velocity(time));
        self.release(velocity, time);
    }

    fn release(&mut self, velocity: f64, time: f64) {
        if self.excess(self.offset) != 0. {
            let to = self.edge(self.offset);
            self.motion = Motion::Spring(Spring::SETTLE.start(f64::from(self.offset), f64::from(to), velocity, time));
        } else if velocity.abs() > MIN_VELOCITY {
            self.motion = Motion::Decay { x0: f64::from(self.offset), v0: velocity, t0: time };
        } else {
            self.motion = Motion::Idle;
        }
    }

    fn os_momentum(&mut self, delta: f32, time: f64) {
        match self.motion {
            Motion::Spring(_) => return,
            Motion::OsMomentum => {}
            _ => self.motion = Motion::OsMomentum,
        }
        let dt = (time - self.last_time).max(1. / 240.);
        let velocity = f64::from(delta) / dt;
        self.last_time = time;
        let next = self.offset + delta;
        if self.excess(next) != 0. {
            let edge = self.edge(next);
            self.offset = edge;
            self.motion = Motion::Spring(Spring::SETTLE.start(f64::from(edge), f64::from(edge), velocity, time));
        } else {
            self.offset = next;
        }
    }

    fn os_momentum_end(&mut self, time: f64) {
        if matches!(self.motion, Motion::OsMomentum) {
            self.release(0., time);
        }
    }

    fn wheel(&mut self, delta: f32) {
        self.motion = Motion::Idle;
        self.offset = self.edge(self.offset + delta);
    }

    fn step(&mut self, time: f64) {
        match self.motion {
            Motion::Decay { x0, v0, t0 } => {
                let t = (time - t0).max(0.);
                let k = 1000. * DECELERATION_RATE.ln();
                let decay = (k * t).exp();
                let x = x0 + v0 * (decay - 1.) / k;
                let v = v0 * decay;
                if x < 0. || x > f64::from(self.max) {
                    // The fling hit an edge: the content carries past it on the spring and comes
                    // back, the bounce.
                    let edge = self.edge(x as f32);
                    let hit = crossing_time(x0, v0, k, f64::from(edge)).map_or(time, |dt| t0 + dt);
                    let at_edge_velocity = v0 * (k * (hit - t0)).exp();
                    let spring = Spring::SETTLE.start(f64::from(edge), f64::from(edge), at_edge_velocity, hit);
                    self.motion = Motion::Spring(spring);
                    self.offset = spring.sample(time).0 as f32;
                } else if v.abs() < MIN_VELOCITY {
                    self.offset = x as f32;
                    self.motion = Motion::Idle;
                } else {
                    self.offset = x as f32;
                }
            }
            Motion::Spring(spring) => {
                if spring.settled(time) {
                    self.offset = spring.target() as f32;
                    self.motion = Motion::Idle;
                } else {
                    self.offset = spring.sample(time).0 as f32;
                }
            }
            Motion::Idle | Motion::Tracking | Motion::OsMomentum => {}
        }
    }

    fn animating(&self) -> bool {
        matches!(self.motion, Motion::Decay { .. } | Motion::Spring(_))
    }

    fn set_extent(&mut self, viewport: f32, content: f32) {
        self.viewport = viewport;
        self.max = (content - viewport).max(0.);
        if matches!(self.motion, Motion::Idle) {
            self.offset = self.edge(self.offset);
        }
    }
}

/// When the decay from `x0` at `v0` reaches `edge`, if it does.
fn crossing_time(x0: f64, v0: f64, k: f64, edge: f64) -> Option<f64> {
    // x0 + v0 (e^{kt} - 1) / k = edge  =>  e^{kt} = 1 + (edge - x0) k / v0
    let arg = 1. + (edge - x0) * k / v0;
    (arg > 0.).then(|| arg.ln() / k)
}

/// Momentum and rubber band for one scroll view, on the frame clock. Offsets are points from the
/// content's top-left; `max` is content minus viewport.
#[derive(Clone, Debug)]
pub struct Scroller {
    axes: Axes,
    x: AxisState,
    y: AxisState,
}

impl Scroller {
    pub fn new(axes: Axes) -> Self {
        Self { axes, x: AxisState::new(), y: AxisState::new() }
    }

    pub fn axes(&self) -> Axes {
        self.axes
    }

    pub fn set_axes(&mut self, axes: Axes) {
        self.axes = axes;
    }

    /// The viewport and content sizes, as laid out this frame.
    pub fn set_extent(&mut self, viewport: Size, content: Size) {
        self.x.set_extent(viewport.width, content.width);
        self.y.set_extent(viewport.height, content.height);
    }

    pub fn offset(&self) -> Point {
        Point { x: self.x.offset, y: self.y.offset }
    }

    pub fn max_offset(&self) -> Point {
        Point { x: self.x.max, y: self.y.max }
    }

    /// Whether the content can move on `axis` in the direction of `delta`'s sign, before rubber
    /// banding. A scroller without room in that direction yields the gesture to an outer one.
    pub fn has_room(&self, axis: Axis, delta: f32) -> bool {
        let a = self.axis(axis);
        if !self.axes.has(axis) || a.max <= 0. {
            return false;
        }
        if delta < 0. { a.offset > 0. } else { a.offset < a.max }
    }

    /// Scrollable on `axis` at all (content larger than the viewport).
    pub fn scrolls(&self, axis: Axis) -> bool {
        self.axes.has(axis) && self.axis(axis).max > 0.
    }

    fn axis(&self, axis: Axis) -> &AxisState {
        match axis {
            Axis::Horizontal => &self.x,
            Axis::Vertical => &self.y,
        }
    }

    /// Fingers touched: any fling or bounce stops where it is.
    pub fn begin(&mut self, time: f64) {
        self.x.begin(time);
        self.y.begin(time);
    }

    /// Fingers moved by `delta` in offset units, on the axes this scroller tracks. `locked`
    /// restricts it to one axis after the direction lock.
    pub fn track(&mut self, delta: Point, locked: Option<Axis>, time: f64) {
        if self.axes.horizontal && locked != Some(Axis::Vertical) {
            self.x.track(delta.x, time);
        }
        if self.axes.vertical && locked != Some(Axis::Horizontal) {
            self.y.track(delta.y, time);
        }
    }

    /// Fingers lifted: fling with the release velocity, or spring back from an overscroll.
    pub fn end(&mut self, time: f64) {
        self.x.end(time);
        self.y.end(time);
    }

    /// A momentum delta from the OS. Replaces the framework's own fling; a delta that crosses an
    /// edge becomes a bounce.
    pub fn os_momentum(&mut self, delta: Point, time: f64) {
        if self.axes.horizontal {
            self.x.os_momentum(delta.x, time);
        }
        if self.axes.vertical {
            self.y.os_momentum(delta.y, time);
        }
    }

    pub fn os_momentum_end(&mut self, time: f64) {
        self.x.os_momentum_end(time);
        self.y.os_momentum_end(time);
    }

    /// A notched wheel: moves by `delta`, clamped, and stops any animation.
    pub fn wheel(&mut self, delta: Point) {
        if self.axes.horizontal {
            self.x.wheel(delta.x);
        }
        if self.axes.vertical {
            self.y.wheel(delta.y);
        }
    }

    /// Sets the offset (a programmatic scroll), clamped, stopping any animation.
    pub fn scroll_to(&mut self, offset: Point) {
        self.x.motion = Motion::Idle;
        self.y.motion = Motion::Idle;
        self.x.offset = self.x.edge(offset.x);
        self.y.offset = self.y.edge(offset.y);
    }

    /// Advances flings and springs to `time`, the frame being prepared. Returns whether the offset
    /// changed.
    pub fn step(&mut self, time: f64) -> bool {
        let before = self.offset();
        self.x.step(time);
        self.y.step(time);
        self.offset() != before
    }

    /// A fling or a spring is in flight: the backend keeps requesting frames.
    pub fn is_animating(&self) -> bool {
        self.x.animating() || self.y.animating()
    }

    pub fn is_tracking(&self) -> bool {
        matches!(self.x.motion, Motion::Tracking) || matches!(self.y.motion, Motion::Tracking)
    }
}
