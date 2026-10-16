//! Element animation on the window's frame clock (ARCHITECTURE.md 3.7). A `div` with
//! `.animate(..)` moves to the opacity, offset, scale and size each render gives it instead of
//! jumping there; `.enter_from(..)` gives a freshly mounted one a pose to come in from. The
//! retained tree samples every motion at the frame's target time and sends the presented values
//! to the backend as ordinary props, so both backends animate the same way and a clean window
//! draws nothing. A new target mid-flight starts from the presented value and, for springs, the
//! velocity, so motion is interruptible. With reduce motion on, movement (offset, scale, size)
//! jumps and only opacity still fades.

use std::time::Duration;

use nd_gesture::{Spring, SpringMotion};
use nd_gpui::gpui::{AbsoluteLength, DefiniteLength, Length, Pixels, StyleRefinement, px};

/// An easing curve for a timed animation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Curve {
    Linear,
    EaseIn,
    EaseOut,
    EaseInOut,
    /// A CSS `cubic-bezier(x1, y1, x2, y2)`.
    Bezier(f32, f32, f32, f32),
}

impl Curve {
    /// Progress at `t` in 0..=1.
    pub fn at(self, t: f32) -> f32 {
        let t = t.clamp(0., 1.);
        let (x1, y1, x2, y2) = match self {
            Curve::Linear => return t,
            Curve::EaseIn => (0.42, 0., 1., 1.),
            Curve::EaseOut => (0., 0., 0.58, 1.),
            Curve::EaseInOut => (0.42, 0., 0.58, 1.),
            Curve::Bezier(a, b, c, d) => (a, b, c, d),
        };
        let bezier = |a: f32, b: f32, s: f32| {
            let u = 1. - s;
            3. * u * u * s * a + 3. * u * s * s * b + s * s * s
        };
        // Solves x(s) = t by bisection: monotonic on 0..=1 for x1, x2 in 0..=1.
        let (mut lo, mut hi) = (0f32, 1f32);
        for _ in 0..24 {
            let mid = (lo + hi) / 2.;
            if bezier(x1, x2, mid) < t {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        bezier(y1, y2, (lo + hi) / 2.)
    }
}

/// How an animated value moves to a new target.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Animation {
    /// Over `duration`, along `curve`.
    Timed { duration: Duration, curve: Curve },
    /// A spring in Apple's terms (`damping` 1 settles without overshoot, `response` is roughly
    /// the seconds it takes). Carries velocity through a retarget.
    Spring { response: f64, damping: f64 },
}

impl Animation {
    pub fn timed(duration: Duration, curve: Curve) -> Self {
        Animation::Timed { duration, curve }
    }

    pub fn linear(duration: Duration) -> Self {
        Self::timed(duration, Curve::Linear)
    }

    pub fn ease_out(duration: Duration) -> Self {
        Self::timed(duration, Curve::EaseOut)
    }

    pub fn ease_in_out(duration: Duration) -> Self {
        Self::timed(duration, Curve::EaseInOut)
    }

    pub fn spring(response: f64, damping: f64) -> Self {
        Animation::Spring { response, damping }
    }
}

/// Where a freshly mounted element starts before moving to its own values; unset fields start
/// at them.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pose {
    pub opacity: Option<f32>,
    pub offset: Option<(f32, f32)>,
    pub scale: Option<f32>,
}

impl Pose {
    pub fn opacity(mut self, opacity: f32) -> Self {
        self.opacity = Some(opacity);
        self
    }

    pub fn offset(mut self, x: Pixels, y: Pixels) -> Self {
        self.offset = Some((x.as_f32(), y.as_f32()));
        self
    }

    pub fn scale(mut self, scale: f32) -> Self {
        self.scale = Some(scale);
        self
    }
}

/// A visual move and scale of a `div`, about its centre. Layout never sees it: siblings stay
/// where they are.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    pub offset: (f32, f32),
    pub scale: f32,
}

impl Default for Transform {
    fn default() -> Self {
        Self { offset: (0., 0.), scale: 1. }
    }
}

impl Transform {
    pub fn is_identity(&self) -> bool {
        *self == Transform::default()
    }
}

/// Reported to a node's `.on_animation_end` handler when every value it animates has settled.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AnimationEnded;

/// What `.animate` and `.enter_from` asked of an element.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct MotionSpec {
    pub animation: Animation,
    pub enter: Option<Pose>,
}

const CHANNELS: usize = 6;
const OPACITY: usize = 0;
const OFFSET_X: usize = 1;
const OFFSET_Y: usize = 2;
const SCALE: usize = 3;
const WIDTH: usize = 4;
const HEIGHT: usize = 5;

/// The animatable values of one element: `None` where the element has none to animate (a
/// size that is not a length in points).
pub(crate) type Values = [Option<f64>; CHANNELS];

fn px_of(length: Option<Length>) -> Option<f64> {
    match length? {
        Length::Definite(DefiniteLength::Absolute(AbsoluteLength::Pixels(p))) => Some(f64::from(p.as_f32())),
        _ => None,
    }
}

/// The values a div's props and layout style give it.
pub(crate) fn values(style: &StyleRefinement, transform: &Transform) -> Values {
    [
        Some(f64::from(style.opacity.unwrap_or(1.))),
        Some(f64::from(transform.offset.0)),
        Some(f64::from(transform.offset.1)),
        Some(f64::from(transform.scale)),
        px_of(style.size.width),
        px_of(style.size.height),
    ]
}

/// `style` and `transform` with `values` in place of their own.
pub(crate) fn present(style: &mut StyleRefinement, transform: &mut Transform, values: &Values) {
    if let Some(o) = values[OPACITY] {
        style.opacity = Some(o.clamp(0., 1.) as f32);
    }
    if let Some(x) = values[OFFSET_X] {
        transform.offset.0 = x as f32;
    }
    if let Some(y) = values[OFFSET_Y] {
        transform.offset.1 = y as f32;
    }
    if let Some(s) = values[SCALE] {
        transform.scale = s.max(0.) as f32;
    }
    if let Some(w) = values[WIDTH] {
        style.size.width = Some(px(w.max(0.) as f32).into());
    }
    if let Some(h) = values[HEIGHT] {
        style.size.height = Some(px(h.max(0.) as f32).into());
    }
}

#[derive(Clone, Copy, Debug)]
enum Motion {
    Timed { from: f64, to: f64, start: f64, duration: f64, curve: Curve },
    Spring(SpringMotion),
}

impl Motion {
    fn start(animation: Animation, from: f64, to: f64, velocity: f64, now: f64) -> Self {
        match animation {
            Animation::Timed { duration, curve } => {
                Motion::Timed { from, to, start: now, duration: duration.as_secs_f64(), curve }
            }
            Animation::Spring { response, damping } => {
                Motion::Spring(Spring { response: response.max(0.01), damping }.start(from, to, velocity, now))
            }
        }
    }

    /// Value and velocity at `now`.
    fn sample(&self, now: f64) -> (f64, f64) {
        match *self {
            Motion::Timed { from, to, start, duration, curve } => {
                if duration <= 0. {
                    return (to, 0.);
                }
                let at = |t: f64| {
                    let p = (t - start) / duration;
                    match p {
                        ..=0. => from,
                        1.0.. => to,
                        p => from + (to - from) * f64::from(curve.at(p as f32)),
                    }
                };
                let dt = 1. / 240.;
                (at(now), (at(now + dt) - at(now)) / dt)
            }
            Motion::Spring(s) => s.sample(now),
        }
    }

    #[cfg(test)]
    fn target(&self) -> f64 {
        match *self {
            Motion::Timed { to, .. } => to,
            Motion::Spring(s) => s.target(),
        }
    }

    /// At rest by `now`, to within `epsilon` of the target.
    fn settled(&self, now: f64, epsilon: f64) -> bool {
        match *self {
            Motion::Timed { start, duration, .. } => now >= start + duration,
            Motion::Spring(s) => {
                let (x, v) = s.sample(now);
                (x - s.target()).abs() < epsilon && v.abs() < epsilon * 10.
            }
        }
    }
}

/// How close a channel must come to rest: a hundredth of opacity, a tenth of a point.
fn epsilon(channel: usize) -> f64 {
    match channel {
        OPACITY => 0.002,
        SCALE => 0.001,
        _ => 0.1,
    }
}

fn moves(channel: usize) -> bool {
    channel != OPACITY
}

/// One element's motions in flight.
#[derive(Clone, Debug)]
pub(crate) struct MotionState {
    animation: Animation,
    /// What the element asked for last.
    targets: Values,
    motions: [Option<Motion>; CHANNELS],
}

impl MotionState {
    /// A state at rest on `targets`, or coming in from `enter`.
    pub fn new(spec: MotionSpec, targets: Values, now: f64, reduce: bool) -> Self {
        let mut state = MotionState { animation: spec.animation, targets, motions: [None; CHANNELS] };
        if let Some(pose) = spec.enter {
            let from: Values = [
                pose.opacity.map(f64::from),
                pose.offset.map(|o| f64::from(o.0)),
                pose.offset.map(|o| f64::from(o.1)),
                pose.scale.map(f64::from),
                None,
                None,
            ];
            for (c, from) in from.into_iter().enumerate() {
                if let (Some(from), Some(to)) = (from, targets[c])
                    && !(reduce && moves(c))
                    && from != to
                {
                    state.motions[c] = Some(Motion::start(spec.animation, from, to, 0., now));
                }
            }
        }
        state
    }

    /// The element rendered again with `targets`: channels whose target moved start towards
    /// it from where they are now.
    pub fn retarget(&mut self, animation: Animation, targets: Values, now: f64, reduce: bool) {
        self.animation = animation;
        for c in 0..CHANNELS {
            if targets[c] == self.targets[c] {
                continue;
            }
            let current = self.motions[c].map(|m| m.sample(now)).or(self.targets[c].map(|v| (v, 0.)));
            self.motions[c] = match (current, targets[c]) {
                (Some((from, velocity)), Some(to)) if !(reduce && moves(c)) && from != to => {
                    Some(Motion::start(animation, from, to, velocity, now))
                }
                _ => None,
            };
        }
        self.targets = targets;
    }

    /// The values to show at `now`.
    pub fn sample(&self, now: f64) -> Values {
        let mut out = self.targets;
        for (c, motion) in self.motions.iter().enumerate() {
            if let Some(m) = motion {
                out[c] = Some(m.sample(now).0);
            }
        }
        out
    }

    /// Drops the motions that came to rest by `now`. True when the last one just did.
    pub fn settle(&mut self, now: f64) -> bool {
        let was = self.is_moving();
        for (c, motion) in self.motions.iter_mut().enumerate() {
            if motion.is_some_and(|m| m.settled(now, epsilon(c))) {
                *motion = None;
            }
        }
        was && !self.is_moving()
    }

    pub fn is_moving(&self) -> bool {
        self.motions.iter().any(Option::is_some)
    }

    #[cfg(test)]
    fn target(&self, c: usize) -> Option<f64> {
        self.motions[c].map(|m| m.target())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(animation: Animation) -> MotionSpec {
        MotionSpec { animation, enter: None }
    }

    fn opacity(v: f64) -> Values {
        [Some(v), Some(0.), Some(0.), Some(1.), None, None]
    }

    #[test]
    fn curves_start_and_end_in_place() {
        for curve in [Curve::Linear, Curve::EaseIn, Curve::EaseOut, Curve::EaseInOut, Curve::Bezier(0.2, 0.9, 0.3, 1.)] {
            assert!(curve.at(0.).abs() < 1e-3, "{curve:?}");
            assert!((curve.at(1.) - 1.).abs() < 1e-3, "{curve:?}");
        }
        assert!(Curve::EaseOut.at(0.5) > 0.5, "ease out is ahead at the midpoint");
        assert!(Curve::EaseIn.at(0.5) < 0.5);
    }

    #[test]
    fn a_timed_fade_samples_its_curve_and_settles_on_time() {
        let anim = Animation::linear(Duration::from_millis(200));
        let mut state = MotionState::new(spec(anim), opacity(1.), 0., false);
        state.retarget(anim, opacity(0.), 1., false);
        assert_eq!(state.sample(1.)[OPACITY], Some(1.));
        assert!((state.sample(1.1)[OPACITY].unwrap() - 0.5).abs() < 1e-3);
        assert!(!state.settle(1.15));
        assert!(state.settle(1.2), "settled exactly at the end");
        assert_eq!(state.sample(5.)[OPACITY], Some(0.));
    }

    #[test]
    fn a_spring_retargeted_mid_flight_keeps_its_velocity() {
        let anim = Animation::spring(0.4, 1.);
        let mut state = MotionState::new(spec(anim), opacity(0.), 0., false);
        state.retarget(anim, opacity(1.), 0., false);
        let (mid, v) = state.motions[OPACITY].unwrap().sample(0.1);
        assert!(mid > 0. && mid < 1. && v > 0.);
        state.retarget(anim, opacity(0.), 0.1, false);
        let (again, v2) = state.motions[OPACITY].unwrap().sample(0.1);
        assert!((again - mid).abs() < 1e-9, "no jump on retarget");
        assert!((v2 - v).abs() < 1e-6, "the velocity carries through");
        assert_eq!(state.target(OPACITY), Some(0.));
        let mut t = 0.1;
        while state.is_moving() {
            t += 1. / 60.;
            state.settle(t);
            assert!(t < 3., "a critically damped spring settles");
        }
    }

    #[test]
    fn reduce_motion_jumps_movement_but_still_fades() {
        let anim = Animation::ease_out(Duration::from_millis(300));
        let pose = Pose::default().opacity(0.).offset(px(0.), px(12.)).scale(0.5);
        let state = MotionState::new(MotionSpec { animation: anim, enter: Some(pose) }, opacity(1.), 0., true);
        let now = state.sample(0.);
        assert!(now[OPACITY].unwrap().abs() < 1e-9, "the fade starts at the pose");
        assert_eq!(now[OFFSET_Y], Some(0.), "the offset does not move");
        assert_eq!(now[SCALE], Some(1.));
        let moving = MotionState::new(MotionSpec { animation: anim, enter: Some(pose) }, opacity(1.), 0., false);
        assert_eq!(moving.sample(0.)[OFFSET_Y], Some(12.));
        assert_eq!(moving.sample(0.)[SCALE], Some(0.5));
    }

    #[test]
    fn sizes_animate_only_when_given_in_points() {
        let mut style = StyleRefinement::default();
        style.size.width = Some(px(100.).into());
        style.size.height = Some(nd_gpui::gpui::relative(0.5).into());
        let v = values(&style, &Transform::default());
        assert_eq!(v[WIDTH], Some(100.));
        assert_eq!(v[HEIGHT], None);
        let mut t = Transform::default();
        present(&mut style, &mut t, &[Some(0.5), Some(3.), Some(4.), Some(2.), Some(50.), None]);
        assert_eq!(style.opacity, Some(0.5));
        assert_eq!(t, Transform { offset: (3., 4.), scale: 2. });
        assert_eq!(px_of(style.size.width), Some(50.));
    }
}
