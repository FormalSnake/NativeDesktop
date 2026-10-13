//! A window's frame clock (ARCHITECTURE.md 5.3). Times are seconds on the host's monotonic clock
//! (`CACurrentMediaTime` on macOS); the test clock starts at zero and moves only when a test
//! advances it.

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FrameTime {
    /// Frames the clock has produced, counting from 1.
    pub frame: u64,
    /// When the previous frame was presented.
    pub timestamp: f64,
    /// When the frame being prepared will be presented. Animations sample here.
    pub target: f64,
}

impl FrameTime {
    /// The display interval this frame was paced at.
    pub fn interval(&self) -> f64 {
        self.target - self.timestamp
    }

    /// The next tick of a clock running at `interval`, as the test clock produces it.
    pub fn advance(self, interval: f64) -> Self {
        Self { frame: self.frame + 1, timestamp: self.target, target: self.target + interval }
    }
}
