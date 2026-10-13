/// Release velocity from recent samples: a least-squares line through the last 100 ms of one
/// coordinate. A pause before release (no sample in the last 40 ms) reads as zero, so a drag
/// that stopped before lifting does not fling.
#[derive(Clone, Debug, Default)]
pub struct VelocityTracker {
    samples: Vec<(f64, f32)>,
}

const WINDOW: f64 = 0.1;
const STALE: f64 = 0.04;
const MAX_SAMPLES: usize = 20;

impl VelocityTracker {
    pub fn reset(&mut self) {
        self.samples.clear();
    }

    /// Moves every sample by `delta`, as when the coordinate space under them shifted.
    pub fn shift(&mut self, delta: f32) {
        for (_, v) in &mut self.samples {
            *v += delta;
        }
    }

    pub fn add(&mut self, time: f64, value: f32) {
        if self.samples.len() == MAX_SAMPLES {
            self.samples.remove(0);
        }
        self.samples.push((time, value));
    }

    /// Units per second at `now`.
    pub fn velocity(&self, now: f64) -> f32 {
        let Some(&(last, _)) = self.samples.last() else { return 0. };
        if now - last > STALE {
            return 0.;
        }
        let recent: Vec<(f64, f64)> =
            self.samples.iter().filter(|(t, _)| last - t <= WINDOW).map(|(t, v)| (*t - last, f64::from(*v))).collect();
        if recent.len() < 2 {
            return 0.;
        }
        let n = recent.len() as f64;
        let mean_t = recent.iter().map(|(t, _)| t).sum::<f64>() / n;
        let mean_v = recent.iter().map(|(_, v)| v).sum::<f64>() / n;
        let (mut num, mut den) = (0., 0.);
        for (t, v) in &recent {
            num += (t - mean_t) * (v - mean_v);
            den += (t - mean_t) * (t - mean_t);
        }
        if den <= f64::EPSILON { 0. } else { (num / den) as f32 }
    }
}
