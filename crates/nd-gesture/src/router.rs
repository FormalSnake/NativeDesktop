use std::collections::HashMap;

use nd_core::{NodeId, Point, Rect, Size};

use crate::arena::{Arena, ArenaMember};
use crate::events::{GestureEvent, RowSwipeEvent};
use crate::haptic::Haptic;
use crate::input::{MomentumPhase, PointerInput, PointerPhase, ScrollInput, ScrollPhase};
use crate::recognizer::{LongPress, Pan, Tap};
use crate::scroll::Scroller;
use crate::set::{Axis, GestureConfig, GestureSet};
use crate::swipe::{PageSwipe, RowSwipe};

/// One node under the pointer, as the backend's hit test found it. A hit path lists them
/// innermost first.
#[derive(Clone, Copy, Debug)]
pub struct HitEntry<'a> {
    pub node: NodeId,
    pub gestures: &'a GestureSet,
    /// The node's bounds in window coordinates: a row's width, a page's width.
    pub bounds: Rect,
}

/// What the backend does with the router's result.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Output {
    /// Queue `event` for `node`'s handler.
    Event(NodeId, GestureEvent),
    /// Show the scroller's content at `offset`.
    Scroll {
        node: NodeId,
        offset: Point,
    },
    Haptic(Haptic),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Candidate {
    Scroller(NodeId),
    Row(NodeId, f32),
    Page(NodeId, f32),
}

enum Route {
    Scroller(NodeId),
    Row(NodeId),
    Page(NodeId, PageSwipe),
}

struct ScrollSession {
    candidates: Vec<Candidate>,
    accumulated: Point,
    lock: Option<Axis>,
    route: Option<Route>,
    /// Scrollers whose fling the touch caught, released again if they do not win.
    caught: Vec<NodeId>,
}

/// A window's gesture state: the pointer arena, the scroll arbiter, every scroller's physics and
/// every row's swipe state. The backend feeds it raw input with the hit path and applies its
/// [`Output`]s; it calls [`Self::step`] once per frame while [`Self::needs_frame`].
#[derive(Default)]
pub struct GestureRouter {
    config: GestureConfig,
    arena: Arena,
    scrollers: HashMap<NodeId, Scroller>,
    rows: HashMap<NodeId, RowSwipe>,
    session: Option<ScrollSession>,
}

/// Picks the axis and the winner of a two-finger scroll, as in [`GestureRouter`]: the innermost
/// candidate that takes the locked axis in the gesture's direction wins; a scroller with no room
/// that way yields to an outer candidate, and only rubber bands when nothing outer takes it.
pub struct ScrollArbiter;

impl ScrollArbiter {
    /// The locked axis once `accumulated` passes `lock_distance`.
    pub fn lock(accumulated: Point, lock_distance: f32) -> Option<Axis> {
        let (ax, ay) = (accumulated.x.abs(), accumulated.y.abs());
        if ax.max(ay) < lock_distance {
            None
        } else if ax > ay {
            Some(Axis::Horizontal)
        } else {
            Some(Axis::Vertical)
        }
    }
}

impl GestureRouter {
    pub fn new(config: GestureConfig) -> Self {
        Self { config, ..Self::default() }
    }

    pub fn config(&self) -> GestureConfig {
        self.config
    }

    pub fn set_config(&mut self, config: GestureConfig) {
        self.config = config;
    }

    // Pointer gestures.

    /// A primary-button event. `hit` is read on a press only; the press's members stay until
    /// release.
    pub fn pointer(&mut self, input: &PointerInput, hit: &[HitEntry<'_>]) -> Vec<Output> {
        let events = if input.phase == PointerPhase::Down {
            let members = self.members(hit);
            if members.is_empty() && !self.arena.is_active() {
                return Vec::new();
            }
            self.arena.down(members, input)
        } else {
            self.arena.pointer(input)
        };
        events.into_iter().map(|(n, e)| Output::Event(n, e)).collect()
    }

    fn members(&self, hit: &[HitEntry<'_>]) -> Vec<ArenaMember> {
        let mut members = Vec::new();
        for entry in hit {
            let set = entry.gestures;
            if let Some(axis) = set.drag {
                members.push(ArenaMember::new(entry.node, Pan::new(self.config, axis)));
            }
            if set.long_press {
                members.push(ArenaMember::new(entry.node, LongPress::new(self.config)));
            }
            if set.tap || set.double_tap {
                members.push(ArenaMember::new(entry.node, Tap::new(self.config, set.double_tap)));
            }
        }
        members
    }

    /// A press is being arbitrated, or a member still waits on a timer after release.
    pub fn pointer_active(&self) -> bool {
        self.arena.is_active()
    }

    /// The node whose pointer recognizer won the current press.
    pub fn pointer_winner(&self) -> Option<NodeId> {
        self.arena.winner()
    }

    // Discrete gestures the OS recognizes (pinch, rotate, force click, smart zoom).

    /// Routes an OS-recognized gesture to the innermost node listening for it.
    pub fn route(&self, event: GestureEvent, hit: &[HitEntry<'_>]) -> Option<Output> {
        let wants = |set: &GestureSet| match event {
            GestureEvent::Pinch(_) => set.pinch,
            GestureEvent::Rotate(_) => set.rotate,
            GestureEvent::ForceClick(_) => set.force_click,
            GestureEvent::SmartMagnify(_) => set.smart_magnify,
            GestureEvent::SwipePage(_) => set.swipe_page,
            GestureEvent::Tap(_) => set.tap || set.double_tap,
            GestureEvent::LongPress(_) => set.long_press,
            GestureEvent::Drag(_) => set.drag.is_some(),
            GestureEvent::RowSwipe(_) => set.row_swipe.is_some(),
        };
        hit.iter().find(|e| wants(e.gestures)).map(|e| Output::Event(e.node, event))
    }

    // Scrolling.

    /// Registers a scroll view and its laid-out extent. Called every frame for each scroller the
    /// backend shows.
    pub fn set_scroll_extent(&mut self, node: NodeId, set: &GestureSet, viewport: Size, content: Size) {
        let scroller = self.scrollers.entry(node).or_insert_with(|| Scroller::new(set.scroll));
        scroller.set_axes(set.scroll);
        scroller.set_extent(viewport, content);
    }

    pub fn scroll_offset(&self, node: NodeId) -> Point {
        self.scrollers.get(&node).map_or(Point::default(), Scroller::offset)
    }

    pub fn scroller(&self, node: NodeId) -> Option<&Scroller> {
        self.scrollers.get(&node)
    }

    pub fn scroller_mut(&mut self, node: NodeId) -> Option<&mut Scroller> {
        self.scrollers.get_mut(&node)
    }

    pub fn row(&self, node: NodeId) -> Option<&RowSwipe> {
        self.rows.get(&node)
    }

    /// Moves the node's scroll and swipe state to `to`, for a view that moved to another
    /// window's router.
    pub fn hand_over(&mut self, node: NodeId, to: &mut GestureRouter) {
        if let Some(scroller) = self.scrollers.remove(&node) {
            to.scrollers.insert(node, scroller);
        }
        if let Some(row) = self.rows.remove(&node) {
            to.rows.insert(node, row);
        }
    }

    /// The node was released: its physics go with it.
    pub fn forget(&mut self, node: NodeId) {
        self.scrollers.remove(&node);
        self.rows.remove(&node);
    }

    /// One scroll event with the hit path under it, read when a gesture begins and for wheels.
    pub fn scroll(&mut self, input: &ScrollInput, hit: &[HitEntry<'_>]) -> Vec<Output> {
        let before = self.offsets();
        let mut out = Vec::new();
        match (input.phase, input.momentum) {
            (ScrollPhase::Wheel, _) => self.wheel(input, hit),
            (ScrollPhase::Began, MomentumPhase::None) => {
                self.begin_session(hit, input.time);
                self.track(input, &mut out);
            }
            (ScrollPhase::Changed, MomentumPhase::None) => {
                if self.session.is_none() {
                    // A trackpad that sent no Began (some Linux drivers): this delta starts one.
                    self.begin_session(hit, input.time);
                }
                self.track(input, &mut out);
            }
            (ScrollPhase::Ended | ScrollPhase::Cancelled, MomentumPhase::None) => {
                self.end_fingers(input.time, &mut out)
            }
            (_, MomentumPhase::Began | MomentumPhase::Changed) => {
                if let Some(ScrollSession { route: Some(Route::Scroller(node)), .. }) = &self.session
                    && let Some(s) = self.scrollers.get_mut(node)
                {
                    s.os_momentum(input.delta, input.time);
                }
            }
            (_, MomentumPhase::Ended) => {
                if let Some(ScrollSession { route: Some(Route::Scroller(node)), .. }) = self.session.take()
                    && let Some(s) = self.scrollers.get_mut(&node)
                {
                    s.os_momentum_end(input.time);
                }
            }
        }
        self.push_moved(&before, &mut out);
        out
    }

    fn wheel(&mut self, input: &ScrollInput, hit: &[HitEntry<'_>]) {
        let axis = if input.delta.y.abs() >= input.delta.x.abs() { Axis::Vertical } else { Axis::Horizontal };
        let along = match axis {
            Axis::Horizontal => input.delta.x,
            Axis::Vertical => input.delta.y,
        };
        let target = hit
            .iter()
            .filter(|e| e.gestures.scroll.any())
            .find(|e| self.scrollers.get(&e.node).is_some_and(|s| s.has_room(axis, along)))
            .map(|e| e.node);
        if let Some(s) = target.and_then(|n| self.scrollers.get_mut(&n)) {
            s.wheel(input.delta);
        }
    }

    fn begin_session(&mut self, hit: &[HitEntry<'_>], time: f64) {
        let mut candidates = Vec::new();
        let mut caught = Vec::new();
        for entry in hit {
            let set = entry.gestures;
            if set.scroll.any() && self.scrollers.contains_key(&entry.node) {
                candidates.push(Candidate::Scroller(entry.node));
                let s = self.scrollers.get_mut(&entry.node).expect("scroller");
                if s.is_animating() {
                    s.begin(time);
                    caught.push(entry.node);
                }
            }
            if set.row_swipe.is_some() {
                candidates.push(Candidate::Row(entry.node, entry.bounds.size.width));
            }
            if set.swipe_page {
                candidates.push(Candidate::Page(entry.node, entry.bounds.size.width));
            }
        }
        self.session =
            Some(ScrollSession { candidates, accumulated: Point::default(), lock: None, route: None, caught });
    }

    fn track(&mut self, input: &ScrollInput, out: &mut Vec<Output>) {
        let Some(mut session) = self.session.take() else { return };
        let mut delta = input.delta;
        if session.route.is_none() {
            session.accumulated.x += delta.x;
            session.accumulated.y += delta.y;
            if let Some(axis) = ScrollArbiter::lock(session.accumulated, self.config.lock_distance) {
                session.lock = Some(axis);
                session.route = self.pick(&session.candidates, axis, session.accumulated, input.time, out);
                // The motion spent reaching the lock is applied, so nothing lags the fingers.
                delta = session.accumulated;
                for node in std::mem::take(&mut session.caught) {
                    if !matches!(session.route, Some(Route::Scroller(n)) if n == node)
                        && let Some(s) = self.scrollers.get_mut(&node)
                    {
                        s.end(input.time);
                    }
                }
            }
        }
        match &mut session.route {
            Some(Route::Scroller(node)) => {
                if let Some(s) = self.scrollers.get_mut(node) {
                    s.track(delta, session.lock, input.time);
                }
            }
            Some(Route::Row(node)) => {
                if let Some(row) = self.rows.get_mut(node) {
                    let (event, haptic) = row.update(-delta.x, input.time);
                    out.push(Output::Event(*node, GestureEvent::RowSwipe(event)));
                    out.extend(haptic.map(Output::Haptic));
                }
            }
            Some(Route::Page(node, page)) => {
                let event = page.update(-delta.x, input.time);
                out.push(Output::Event(*node, GestureEvent::SwipePage(event)));
            }
            None => {}
        }
        self.session = Some(session);
    }

    fn pick(
        &mut self,
        candidates: &[Candidate],
        axis: Axis,
        accumulated: Point,
        time: f64,
        out: &mut Vec<Output>,
    ) -> Option<Route> {
        let along = match axis {
            Axis::Horizontal => accumulated.x,
            Axis::Vertical => accumulated.y,
        };
        let mut fallback = None;
        for candidate in candidates {
            match *candidate {
                Candidate::Scroller(node) => {
                    let s = &self.scrollers[&node];
                    if s.has_room(axis, along) {
                        self.scrollers.get_mut(&node).expect("scroller").begin(time);
                        return Some(Route::Scroller(node));
                    }
                    if fallback.is_none() && s.scrolls(axis) {
                        fallback = Some(node);
                    }
                }
                Candidate::Row(node, width) if axis == Axis::Horizontal => {
                    self.open_row(node, width, time, out);
                    return Some(Route::Row(node));
                }
                Candidate::Page(node, width) if axis == Axis::Horizontal => {
                    return Some(Route::Page(node, PageSwipe::new(width)));
                }
                Candidate::Row(..) | Candidate::Page(..) => {}
            }
        }
        let node = fallback?;
        self.scrollers.get_mut(&node).expect("scroller").begin(time);
        Some(Route::Scroller(node))
    }

    /// Starts tracking a row, closing any other open row first.
    fn open_row(&mut self, node: NodeId, width: f32, time: f64, out: &mut Vec<Output>) {
        for (other, row) in self.rows.iter_mut() {
            if *other != node && row.open_edge().is_some() {
                row.close(time);
                out.push(Output::Event(*other, GestureEvent::RowSwipe(close_event(row))));
            }
        }
        // A row the backend never configured through `set_row_config` has no actions: it resists.
        self.rows.entry(node).or_insert_with(|| RowSwipe::new(Default::default(), width)).begin(time);
    }

    /// The row's actions and width, from its props and layout. Called each frame like
    /// [`Self::set_scroll_extent`].
    pub fn set_row_config(&mut self, node: NodeId, set: &GestureSet, width: f32) {
        let Some(config) = set.row_swipe else {
            self.rows.remove(&node);
            return;
        };
        let replace = match self.rows.get(&node) {
            Some(row) => !row.is_animating() && row.open_edge().is_none() && row.offset() == 0.,
            None => true,
        };
        if replace {
            self.rows.insert(node, RowSwipe::new(config, width));
        }
    }

    fn end_fingers(&mut self, time: f64, out: &mut Vec<Output>) {
        let Some(session) = self.session.as_mut() else { return };
        for node in std::mem::take(&mut session.caught) {
            if let Some(s) = self.scrollers.get_mut(&node) {
                s.end(time);
            }
        }
        match session.route.take() {
            Some(Route::Scroller(node)) => {
                if let Some(s) = self.scrollers.get_mut(&node) {
                    s.end(time);
                }
                // Kept for the OS momentum that may follow.
                session.route = Some(Route::Scroller(node));
            }
            Some(Route::Row(node)) => {
                if let Some(row) = self.rows.get_mut(&node) {
                    let event = row.end(time);
                    out.push(Output::Event(node, GestureEvent::RowSwipe(event)));
                }
                self.session = None;
            }
            Some(Route::Page(node, mut page)) => {
                out.push(Output::Event(node, GestureEvent::SwipePage(page.end(time))));
                self.session = None;
            }
            None => self.session = None,
        }
    }

    // The frame clock.

    /// Advances timers, flings, springs and row settles to `now`, the frame being prepared.
    pub fn step(&mut self, now: f64) -> Vec<Output> {
        let mut out: Vec<Output> = self.arena.tick(now).into_iter().map(|(n, e)| Output::Event(n, e)).collect();
        let before = self.offsets();
        for s in self.scrollers.values_mut() {
            s.step(now);
        }
        self.push_moved(&before, &mut out);
        for (node, row) in self.rows.iter_mut() {
            if let Some(event) = row.step(now) {
                out.push(Output::Event(*node, GestureEvent::RowSwipe(event)));
            }
        }
        out
    }

    /// A fling, spring or timer needs the next frame.
    pub fn needs_frame(&self) -> bool {
        self.scrollers.values().any(Scroller::is_animating)
            || self.rows.values().any(RowSwipe::is_animating)
            || self.arena.next_deadline().is_some()
    }

    /// The next timer (long press, double-tap wait), for a backend that sleeps between frames.
    pub fn next_deadline(&self) -> Option<f64> {
        self.arena.next_deadline()
    }

    fn offsets(&self) -> Vec<(NodeId, Point)> {
        self.scrollers.iter().map(|(n, s)| (*n, s.offset())).collect()
    }

    fn push_moved(&self, before: &[(NodeId, Point)], out: &mut Vec<Output>) {
        for (node, old) in before {
            if let Some(s) = self.scrollers.get(node)
                && s.offset() != *old
            {
                out.push(Output::Scroll { node: *node, offset: s.offset() });
            }
        }
    }
}

fn close_event(row: &RowSwipe) -> RowSwipeEvent {
    RowSwipeEvent { offset: row.offset(), phase: crate::events::GesturePhase::Changed, open: None, full: false }
}
