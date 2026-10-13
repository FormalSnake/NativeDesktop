/// A spring in Apple's two designer parameters: `damping` (1 is critically damped, below 1
/// overshoots) and `response` (seconds to settle, roughly; not a duration). Evaluated in closed
/// form, so stepping it at any frame rate lands on the same curve.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spring {
    pub damping: f64,
    pub response: f64,
}

impl Spring {
    /// Critically damped, for positions settling back: scroll bounce, a row closing.
    pub const SETTLE: Spring = Spring { damping: 1.0, response: 0.4 };
    /// A little overshoot, only after a gesture threw something (a row swipe flung open).
    pub const FLING: Spring = Spring { damping: 0.8, response: 0.35 };

    /// Starts a motion from `from` towards `to` at `velocity` (units per second), at `time`.
    pub fn start(self, from: f64, to: f64, velocity: f64, time: f64) -> SpringMotion {
        SpringMotion { spring: self, x0: from - to, v0: velocity, target: to, start: time }
    }
}

/// One spring motion in flight. Re-target by starting a new one from [`Self::sample`], which
/// carries the velocity through, so a reversal never hits a wall.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpringMotion {
    spring: Spring,
    /// Displacement from the target at `start`.
    x0: f64,
    v0: f64,
    target: f64,
    start: f64,
}

impl SpringMotion {
    pub fn target(&self) -> f64 {
        self.target
    }

    /// Position and velocity at `time`.
    pub fn sample(&self, time: f64) -> (f64, f64) {
        let t = (time - self.start).max(0.);
        let w = std::f64::consts::TAU / self.spring.response;
        let z = self.spring.damping;
        let (x, v) = if (z - 1.).abs() < 1e-6 {
            let c = self.v0 + w * self.x0;
            let e = (-w * t).exp();
            ((self.x0 + c * t) * e, (self.v0 - w * t * c) * e)
        } else if z < 1. {
            let wd = w * (1. - z * z).sqrt();
            let a = self.x0;
            let b = (self.v0 + z * w * self.x0) / wd;
            let e = (-z * w * t).exp();
            let (s, c) = (wd * t).sin_cos();
            let x = e * (a * c + b * s);
            let v = e * ((b * wd - z * w * a) * c - (a * wd + z * w * b) * s);
            (x, v)
        } else {
            let root = (z * z - 1.).sqrt();
            let (r1, r2) = (-w * (z - root), -w * (z + root));
            let c2 = (self.v0 - r1 * self.x0) / (r2 - r1);
            let c1 = self.x0 - c2;
            ((c1 * (r1 * t).exp() + c2 * (r2 * t).exp()), (c1 * r1 * (r1 * t).exp() + c2 * r2 * (r2 * t).exp()))
        };
        (self.target + x, v)
    }

    /// Within half a point and 5 points per second of rest at `time`.
    pub fn settled(&self, time: f64) -> bool {
        let (x, v) = self.sample(time);
        (x - self.target).abs() < 0.5 && v.abs() < 5.
    }
}
