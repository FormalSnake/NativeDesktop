use crate::events::{GesturePhase, RowEdge, RowSwipeEvent, SwipeDirection, SwipePageEvent};
use crate::haptic::Haptic;
use crate::scroll::{project, rubber_band};
use crate::set::RowSwipeConfig;
use crate::spring::{Spring, SpringMotion};
use crate::velocity::VelocityTracker;

/// A two-finger horizontal page swipe where the OS has no swipe tracking of its own (every
/// backend but AppKit, which uses `trackSwipeEvent`). Progress follows the fingers over `width`;
/// release commits when the projected end passes half the width.
#[derive(Clone, Debug)]
pub struct PageSwipe {
    width: f32,
    /// Finger travel, positive to the right.
    travel: f32,
    tracker: VelocityTracker,
    began: bool,
}

impl PageSwipe {
    pub fn new(width: f32) -> Self {
        Self { width: width.max(1.), travel: 0., tracker: VelocityTracker::default(), began: false }
    }

    fn direction(&self) -> SwipeDirection {
        if self.travel >= 0. { SwipeDirection::Back } else { SwipeDirection::Forward }
    }

    fn progress(&self, travel: f32) -> f32 {
        (travel.abs() / self.width).min(1.)
    }

    /// `finger_dx` is the fingers' movement, positive to the right.
    pub fn update(&mut self, finger_dx: f32, time: f64) -> SwipePageEvent {
        let first = !std::mem::replace(&mut self.began, true);
        self.travel += finger_dx;
        self.tracker.add(time, self.travel);
        SwipePageEvent {
            direction: self.direction(),
            progress: self.progress(self.travel),
            phase: if first { GesturePhase::Began } else { GesturePhase::Changed },
        }
    }

    pub fn end(&mut self, time: f64) -> SwipePageEvent {
        let projected = self.travel + project(self.tracker.velocity(time));
        let committed = projected.signum() == self.travel.signum() && projected.abs() > self.width * 0.5;
        SwipePageEvent {
            direction: self.direction(),
            progress: if committed { 1. } else { 0. },
            phase: if committed { GesturePhase::Ended } else { GesturePhase::Cancelled },
        }
    }
}

/// A list row's swipe-to-reveal (`list(..).row_actions`) where the platform has no native row
/// actions. The row tracks the fingers, resists past its actions' width (unless that edge allows a
/// full swipe), and on release settles open, closed or fully swiped by projected position, on a
/// spring that carries the release velocity.
#[derive(Clone, Debug)]
pub struct RowSwipe {
    config: RowSwipeConfig,
    width: f32,
    raw: f32,
    offset: f32,
    tracker: VelocityTracker,
    spring: Option<SpringMotion>,
    open: Option<RowEdge>,
    full: bool,
    /// Past the full-swipe threshold while tracking, for the haptic on crossing it.
    armed: bool,
}

impl RowSwipe {
    pub fn new(config: RowSwipeConfig, row_width: f32) -> Self {
        Self {
            config,
            width: row_width.max(1.),
            raw: 0.,
            offset: 0.,
            tracker: VelocityTracker::default(),
            spring: None,
            open: None,
            full: false,
            armed: false,
        }
    }

    pub fn offset(&self) -> f32 {
        self.offset
    }

    pub fn open_edge(&self) -> Option<RowEdge> {
        self.open
    }

    fn reveal(&self, edge: RowEdge) -> f32 {
        match edge {
            RowEdge::Leading => self.config.leading,
            RowEdge::Trailing => self.config.trailing,
        }
    }

    fn edge_of(offset: f32) -> RowEdge {
        if offset >= 0. { RowEdge::Leading } else { RowEdge::Trailing }
    }

    fn full_threshold(&self) -> f32 {
        self.width * 0.5
    }

    fn shown(&self, raw: f32) -> f32 {
        let edge = Self::edge_of(raw);
        let limit = self.reveal(edge);
        if limit == 0. {
            return rubber_band(raw, self.width) * 0.25;
        }
        if self.config.full_swipe == Some(edge) || raw.abs() <= limit {
            return raw;
        }
        raw.signum() * (limit + rubber_band(raw.abs() - limit, self.width))
    }

    pub fn begin(&mut self, time: f64) {
        self.spring = None;
        self.raw = self.offset;
        self.tracker.reset();
        self.tracker.add(time, self.raw);
        self.armed = false;
    }

    /// `finger_dx` is the fingers' movement, positive to the right. Returns the event and a
    /// haptic when the row crosses its full-swipe threshold.
    pub fn update(&mut self, finger_dx: f32, time: f64) -> (RowSwipeEvent, Option<Haptic>) {
        self.raw += finger_dx;
        self.tracker.add(time, self.raw);
        self.offset = self.shown(self.raw);
        let edge = Self::edge_of(self.offset);
        let armed = self.config.full_swipe == Some(edge) && self.offset.abs() > self.full_threshold();
        let haptic = (armed != self.armed).then_some(Haptic::LevelChange);
        self.armed = armed;
        (self.event(GesturePhase::Changed), haptic)
    }

    pub fn end(&mut self, time: f64) -> RowSwipeEvent {
        let velocity = self.tracker.velocity(time);
        let projected = self.offset + project(velocity);
        let edge = Self::edge_of(projected);
        let reveal = self.reveal(edge);
        let full = self.config.full_swipe == Some(edge) && (self.armed || projected.abs() > self.width);
        let target = if full {
            projected.signum() * self.width
        } else if reveal > 0. && projected.abs() > reveal * 0.5 && projected.signum() == self.offset.signum() {
            projected.signum() * reveal
        } else {
            0.
        };
        self.open = (target != 0.).then_some(edge);
        self.full = full;
        let spring = if velocity.abs() > 300. { Spring::FLING } else { Spring::SETTLE };
        self.spring = Some(spring.start(f64::from(self.offset), f64::from(target), f64::from(velocity), time));
        self.event(GesturePhase::Ended)
    }

    /// Closes an open row (another row opened, an action ran).
    pub fn close(&mut self, time: f64) {
        self.open = None;
        self.full = false;
        self.spring = Some(Spring::SETTLE.start(f64::from(self.offset), 0., 0., time));
    }

    /// Advances the settle spring; returns the event when the offset moved.
    pub fn step(&mut self, time: f64) -> Option<RowSwipeEvent> {
        let spring = self.spring?;
        if spring.settled(time) {
            self.offset = spring.target() as f32;
            self.spring = None;
        } else {
            self.offset = spring.sample(time).0 as f32;
        }
        Some(self.event(GesturePhase::Changed))
    }

    pub fn is_animating(&self) -> bool {
        self.spring.is_some()
    }

    fn event(&self, phase: GesturePhase) -> RowSwipeEvent {
        RowSwipeEvent { offset: self.offset, phase, open: self.open, full: self.full }
    }
}
