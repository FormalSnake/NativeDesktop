use nd_core::Point;

use crate::events::{DragEvent, GestureEvent, GesturePhase, LongPressEvent, TapEvent};
use crate::input::{PointerInput, PointerPhase};
use crate::set::{Axis, GestureConfig};
use crate::velocity::VelocityTracker;

/// What a recognizer says after each input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Still could be this gesture.
    Possible,
    /// This is the gesture: the arena makes it the winner unless a member already won.
    Accept,
    /// Not this gesture; the arena drops it.
    Reject,
}

/// Which recognizer a member is, so a member held across presses (a double tap waiting for its
/// second press) can be matched to the fresh member of the next press.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GestureKind {
    Tap,
    LongPress,
    Drag,
}

/// A pointer recognizer. It runs its own state machine on every input and writes events to `out`;
/// the arena decides whether they reach the app (winner), wait (undecided) or are dropped.
pub trait Recognizer {
    fn kind(&self) -> GestureKind;
    fn input(&mut self, input: &PointerInput, out: &mut Vec<GestureEvent>) -> Verdict;
    /// The time this recognizer next needs [`Self::tick`], if any.
    fn deadline(&self) -> Option<f64> {
        None
    }
    fn tick(&mut self, _now: f64, _out: &mut Vec<GestureEvent>) -> Verdict {
        Verdict::Possible
    }
    /// Another member won. Write a cancel for anything already delivered.
    fn lose(&mut self, _out: &mut Vec<GestureEvent>) {}
    /// Nothing more will come from this recognizer once the pointer is up.
    fn finished(&self) -> bool;
}

fn distance(a: Point, b: Point) -> f32 {
    ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt()
}

/// Tap and double tap. With `double` set, a first tap is held for the double-tap interval, so a
/// node with both handlers never sees the single tap of a double tap. Without it a tap fires on
/// release, with no delay.
pub struct Tap {
    config: GestureConfig,
    double: bool,
    down: Option<Point>,
    count: u32,
    /// After the first release, while a second press may still come.
    waiting: Option<(f64, Point)>,
    done: bool,
}

impl Tap {
    pub fn new(config: GestureConfig, double: bool) -> Self {
        Self { config, double, down: None, count: 0, waiting: None, done: false }
    }
}

impl Recognizer for Tap {
    fn kind(&self) -> GestureKind {
        GestureKind::Tap
    }

    fn input(&mut self, input: &PointerInput, out: &mut Vec<GestureEvent>) -> Verdict {
        match input.phase {
            PointerPhase::Down => {
                if let Some((_, first)) = self.waiting.take()
                    && distance(first, input.position) > self.config.slop * 2.
                {
                    // A second press elsewhere is a new tap.
                    self.count = 0;
                }
                self.down = Some(input.position);
                Verdict::Possible
            }
            PointerPhase::Move => match self.down {
                Some(down) if distance(down, input.position) > self.config.slop => {
                    self.done = true;
                    Verdict::Reject
                }
                _ => Verdict::Possible,
            },
            PointerPhase::Up => {
                if self.down.take().is_none() {
                    return Verdict::Reject;
                }
                self.count += 1;
                if self.double && self.count == 1 {
                    self.waiting = Some((input.time + self.config.double_tap, input.position));
                    return Verdict::Possible;
                }
                out.push(GestureEvent::Tap(TapEvent { position: input.position, count: self.count }));
                self.done = true;
                Verdict::Accept
            }
            PointerPhase::Cancel => {
                self.done = true;
                Verdict::Reject
            }
        }
    }

    fn deadline(&self) -> Option<f64> {
        self.waiting.map(|(t, _)| t)
    }

    fn tick(&mut self, now: f64, out: &mut Vec<GestureEvent>) -> Verdict {
        match self.waiting {
            Some((deadline, position)) if now >= deadline => {
                self.waiting = None;
                self.done = true;
                out.push(GestureEvent::Tap(TapEvent { position, count: 1 }));
                Verdict::Accept
            }
            _ => Verdict::Possible,
        }
    }

    fn finished(&self) -> bool {
        self.done || (self.down.is_none() && self.waiting.is_none())
    }
}

/// A press held in place for [`GestureConfig::long_press`]. After it begins, moves report
/// `Changed` and the release `Ended`.
pub struct LongPress {
    config: GestureConfig,
    down: Option<(Point, f64)>,
    began: bool,
    done: bool,
}

impl LongPress {
    pub fn new(config: GestureConfig) -> Self {
        Self { config, down: None, began: false, done: false }
    }
}

impl Recognizer for LongPress {
    fn kind(&self) -> GestureKind {
        GestureKind::LongPress
    }

    fn input(&mut self, input: &PointerInput, out: &mut Vec<GestureEvent>) -> Verdict {
        let event = |phase| GestureEvent::LongPress(LongPressEvent { position: input.position, phase });
        match input.phase {
            PointerPhase::Down => {
                self.down = Some((input.position, input.time));
                Verdict::Possible
            }
            PointerPhase::Move if self.began => {
                out.push(event(GesturePhase::Changed));
                Verdict::Possible
            }
            PointerPhase::Move => match self.down {
                Some((at, _)) if distance(at, input.position) > self.config.slop => {
                    self.done = true;
                    Verdict::Reject
                }
                _ => Verdict::Possible,
            },
            PointerPhase::Up | PointerPhase::Cancel if self.began => {
                self.done = true;
                let phase = if input.phase == PointerPhase::Up { GesturePhase::Ended } else { GesturePhase::Cancelled };
                out.push(event(phase));
                Verdict::Possible
            }
            PointerPhase::Up | PointerPhase::Cancel => {
                self.done = true;
                Verdict::Reject
            }
        }
    }

    fn deadline(&self) -> Option<f64> {
        if self.began || self.done { None } else { self.down.map(|(_, t)| t + self.config.long_press) }
    }

    fn tick(&mut self, now: f64, out: &mut Vec<GestureEvent>) -> Verdict {
        match (self.deadline(), self.down) {
            (Some(deadline), Some((position, _))) if now >= deadline => {
                self.began = true;
                out.push(GestureEvent::LongPress(LongPressEvent { position, phase: GesturePhase::Began }));
                Verdict::Accept
            }
            _ => Verdict::Possible,
        }
    }

    fn lose(&mut self, out: &mut Vec<GestureEvent>) {
        if self.began && !self.done {
            out.push(GestureEvent::LongPress(LongPressEvent {
                position: self.down.map_or(Point::default(), |d| d.0),
                phase: GesturePhase::Cancelled,
            }));
        }
        self.done = true;
    }

    fn finished(&self) -> bool {
        self.done || self.down.is_none()
    }
}

/// A pointer drag. Claims once the pointer moves past the slop (along `axis` when set; movement
/// mostly across the axis rejects, leaving it to a drag on the other axis or a scroller).
pub struct Pan {
    config: GestureConfig,
    axis: Option<Axis>,
    start: Option<Point>,
    began: bool,
    done: bool,
    vx: VelocityTracker,
    vy: VelocityTracker,
}

impl Pan {
    pub fn new(config: GestureConfig, axis: Option<Axis>) -> Self {
        Self {
            config,
            axis,
            start: None,
            began: false,
            done: false,
            vx: VelocityTracker::default(),
            vy: VelocityTracker::default(),
        }
    }

    fn event(&self, phase: GesturePhase, position: Point, velocity: Point) -> GestureEvent {
        let start = self.start.unwrap_or(position);
        let mut translation = Point { x: position.x - start.x, y: position.y - start.y };
        match self.axis {
            Some(Axis::Horizontal) => translation.y = 0.,
            Some(Axis::Vertical) => translation.x = 0.,
            None => {}
        }
        GestureEvent::Drag(DragEvent { phase, start, position, translation, velocity })
    }
}

impl Recognizer for Pan {
    fn kind(&self) -> GestureKind {
        GestureKind::Drag
    }

    fn input(&mut self, input: &PointerInput, out: &mut Vec<GestureEvent>) -> Verdict {
        match input.phase {
            PointerPhase::Down => {
                self.start = Some(input.position);
                self.vx.reset();
                self.vy.reset();
                self.vx.add(input.time, input.position.x);
                self.vy.add(input.time, input.position.y);
                Verdict::Possible
            }
            PointerPhase::Move => {
                let Some(start) = self.start else { return Verdict::Reject };
                self.vx.add(input.time, input.position.x);
                self.vy.add(input.time, input.position.y);
                if self.began {
                    out.push(self.event(GesturePhase::Changed, input.position, Point::default()));
                    return Verdict::Possible;
                }
                let (dx, dy) = ((input.position.x - start.x).abs(), (input.position.y - start.y).abs());
                let (along, across) = match self.axis {
                    Some(Axis::Horizontal) => (dx, dy),
                    Some(Axis::Vertical) => (dy, dx),
                    None => (distance(start, input.position), 0.),
                };
                if across > self.config.slop && across > along {
                    self.done = true;
                    return Verdict::Reject;
                }
                if along > self.config.slop {
                    self.began = true;
                    out.push(self.event(GesturePhase::Began, input.position, Point::default()));
                    out.push(self.event(GesturePhase::Changed, input.position, Point::default()));
                    return Verdict::Accept;
                }
                Verdict::Possible
            }
            PointerPhase::Up | PointerPhase::Cancel => {
                self.done = true;
                if !self.began {
                    return Verdict::Reject;
                }
                let (phase, velocity) = if input.phase == PointerPhase::Up {
                    (GesturePhase::Ended, Point { x: self.vx.velocity(input.time), y: self.vy.velocity(input.time) })
                } else {
                    (GesturePhase::Cancelled, Point::default())
                };
                out.push(self.event(phase, input.position, velocity));
                Verdict::Possible
            }
        }
    }

    fn lose(&mut self, out: &mut Vec<GestureEvent>) {
        if self.began && !self.done {
            let at = self.start.unwrap_or_default();
            out.push(self.event(GesturePhase::Cancelled, at, Point::default()));
        }
        self.done = true;
    }

    fn finished(&self) -> bool {
        self.done || self.start.is_none()
    }
}
