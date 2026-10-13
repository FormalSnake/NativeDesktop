use nd_core::Point;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerPhase {
    Down,
    Move,
    Up,
    /// The OS took the pointer (a window drag began, the app lost focus mid-press).
    Cancel,
}

/// One primary-button pointer event. Secondary buttons never enter the arena.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointerInput {
    pub phase: PointerPhase,
    pub position: Point,
    /// Seconds on the frame clock.
    pub time: f64,
}

impl PointerInput {
    pub fn new(phase: PointerPhase, position: Point, time: f64) -> Self {
        Self { phase, position, time }
    }
}

/// The finger phase of a precise (trackpad) scroll.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollPhase {
    Began,
    Changed,
    Ended,
    Cancelled,
    /// A notched mouse wheel: no fingers, no momentum, no rubber band.
    Wheel,
}

/// The OS momentum phase that follows a scroll's finger phase, where the OS sends one (macOS).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MomentumPhase {
    #[default]
    None,
    Began,
    Changed,
    Ended,
}

/// One scroll event. `delta` is in offset units: positive moves the content's offset forward
/// (towards the end), which is the opposite of the finger's direction under natural scrolling.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollInput {
    pub position: Point,
    pub delta: Point,
    pub phase: ScrollPhase,
    pub momentum: MomentumPhase,
    pub time: f64,
}

/// Turns a stream that carries only finger phases (GPUI's `ScrollWheelEvent.touch_phase`, which
/// never reads `momentumPhase`) into one that marks OS momentum: deltas arriving after the
/// fingers lifted, before the next touch, are the OS's inertia. Where none arrive (Wayland) the
/// framework's own momentum runs.
#[derive(Clone, Copy, Debug, Default)]
pub struct MomentumInference {
    lifted: bool,
    momentum: bool,
}

impl MomentumInference {
    /// `phase` is the finger phase of a precise event, `None` for a wheel. During OS momentum the
    /// returned finger phase is `Changed` and the momentum phase carries the meaning.
    pub fn classify(&mut self, phase: Option<ScrollPhase>) -> (ScrollPhase, MomentumPhase) {
        match phase {
            None => (ScrollPhase::Wheel, MomentumPhase::None),
            Some(ScrollPhase::Began) => {
                self.lifted = false;
                self.momentum = false;
                (ScrollPhase::Began, MomentumPhase::None)
            }
            Some(ScrollPhase::Ended) | Some(ScrollPhase::Cancelled) if !self.lifted => {
                self.lifted = true;
                (phase.unwrap_or(ScrollPhase::Ended), MomentumPhase::None)
            }
            Some(ScrollPhase::Ended) | Some(ScrollPhase::Cancelled) => {
                self.momentum = false;
                (ScrollPhase::Changed, MomentumPhase::Ended)
            }
            Some(_) if self.lifted => {
                let phase = if self.momentum { MomentumPhase::Changed } else { MomentumPhase::Began };
                self.momentum = true;
                (ScrollPhase::Changed, phase)
            }
            Some(p) => (p, MomentumPhase::None),
        }
    }
}
