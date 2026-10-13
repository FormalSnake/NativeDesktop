//! Gestures on GPUI backends (ARCHITECTURE.md 6.1). GPUI hands over raw pointer, scroll, pinch and
//! pressure events; `nd-gesture` recognizes them over nd's own hit path (the node bounds the last
//! paint recorded), arbitrates nested recognizers, and runs scroll momentum and rubber banding on
//! the frame clock, since GPUI never reads `momentumPhase` and Wayland sends no inertia
//! (zed#14473). Scroll views are drawn with their content shifted by the router's offset; GPUI's
//! own div scrolling is not used, so there is one scroll model per window.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Instant;

use nd_core::{NodeId, Point, Rect, Size, WindowId};
use nd_gesture::{
    ForceClickEvent, GestureEvent, GesturePhase, GestureRouter, GestureSet, HitEntry, MomentumInference, Output,
    PinchEvent, PointerInput, PointerPhase, ScrollInput, ScrollPhase,
};
use nd_gpui::gpui::{
    self, AnyElement, App, Bounds, ElementId, GlobalElementId, InspectorElementId, IntoElement, LayoutId, Pixels,
    PressureStage, ScrollDelta, TouchPhase, point, px,
};
use nd_ui::DivProps;
use nd_ui::backend::NodeKind;
use nd_ui::events::{EventQueue, emit_gesture};

use crate::backend::Node;

#[derive(Default)]
struct WindowGestures {
    router: GestureRouter,
    momentum: MomentumInference,
    /// Last force-click stage, so a deep press fires once.
    force: bool,
}

/// Every window's gesture state, kept apart from the backend's seam state so input handlers and
/// paints never contend with a seam op.
#[derive(Clone)]
pub(crate) struct Gestures {
    windows: Rc<RefCell<HashMap<WindowId, WindowGestures>>>,
    /// Each scroll view's content extent as of the last paint: the union of its children's
    /// unshifted bounds.
    content: Rc<RefCell<HashMap<NodeId, Rect>>>,
    origin: Instant,
}

impl Default for Gestures {
    fn default() -> Self {
        Self { windows: Rc::default(), content: Rc::default(), origin: Instant::now() }
    }
}

fn gestures_of(node: &Node) -> Option<&GestureSet> {
    if let NodeKind::Widget(w) = node.kind
        && w.type_id == std::any::TypeId::of::<nd_virtual::ListWidget>()
    {
        return node.props.downcast_ref::<nd_virtual::ListProps>().map(|l| &l.gestures);
    }
    if node.kind != NodeKind::View {
        return None;
    }
    node.props.downcast_ref::<DivProps>().map(|d| &d.gestures).filter(|g| !g.is_empty())
}

fn contains(r: &Rect, p: Point) -> bool {
    p.x >= r.origin.x && p.y >= r.origin.y && p.x < r.origin.x + r.size.width && p.y < r.origin.y + r.size.height
}

/// Nodes under `at`, innermost first: from the root, into the topmost child (the last painted)
/// whose bounds contain the point.
fn hit_path(nodes: &HashMap<NodeId, Node>, bounds: &HashMap<NodeId, Rect>, root: NodeId, at: Point) -> Vec<NodeId> {
    let mut path = vec![root];
    let mut cursor = root;
    while let Some(node) = nodes.get(&cursor) {
        let next = node.children.iter().rev().find(|c| bounds.get(c).is_some_and(|b| contains(b, at)));
        match next {
            Some(c) => {
                path.push(*c);
                cursor = *c;
            }
            None => break,
        }
    }
    path.reverse();
    path
}

/// A window's retained tree as the hit test reads it: nodes, painted bounds, root.
pub(crate) type Tree<'a> = (&'a HashMap<NodeId, Node>, &'a HashMap<NodeId, Rect>, NodeId);

impl Gestures {
    pub fn needs_frame(&self, window: WindowId) -> bool {
        self.windows.borrow().get(&window).is_some_and(|w| w.router.needs_frame())
    }

    pub fn now(&self) -> f64 {
        self.origin.elapsed().as_secs_f64()
    }

    fn with_hit<R>(
        &self,
        nodes: &HashMap<NodeId, Node>,
        bounds: &HashMap<NodeId, Rect>,
        root: NodeId,
        at: Point,
        f: impl FnOnce(&[HitEntry<'_>]) -> R,
    ) -> R {
        let path = hit_path(nodes, bounds, root, at);
        let hit: Vec<HitEntry<'_>> = path
            .iter()
            .filter_map(|n| {
                let gestures = gestures_of(nodes.get(n)?)?;
                Some(HitEntry { node: *n, gestures, bounds: bounds.get(n).copied().unwrap_or_default() })
            })
            .collect();
        f(&hit)
    }

    pub fn pointer(&self, window: WindowId, phase: PointerPhase, at: Point, tree: Tree<'_>) -> Vec<Output> {
        let input = PointerInput::new(phase, at, self.now());
        let mut windows = self.windows.borrow_mut();
        let w = windows.entry(window).or_default();
        if phase == PointerPhase::Down {
            self.with_hit(tree.0, tree.1, tree.2, at, |hit| w.router.pointer(&input, hit))
        } else if w.router.pointer_active() {
            w.router.pointer(&input, &[])
        } else {
            Vec::new()
        }
    }

    pub fn scroll(&self, window: WindowId, event: &gpui::ScrollWheelEvent, tree: Tree<'_>) -> Vec<Output> {
        let at = Point { x: event.position.x.as_f32(), y: event.position.y.as_f32() };
        let (delta, finger) = match event.delta {
            ScrollDelta::Pixels(d) => {
                let phase = match event.touch_phase {
                    TouchPhase::Started => ScrollPhase::Began,
                    TouchPhase::Moved => ScrollPhase::Changed,
                    TouchPhase::Ended => ScrollPhase::Ended,
                    TouchPhase::Cancelled => ScrollPhase::Cancelled,
                };
                (Point { x: -d.x.as_f32(), y: -d.y.as_f32() }, Some(phase))
            }
            // A notched wheel: GPUI's default line height, as its own scrolling uses.
            ScrollDelta::Lines(d) => (Point { x: -d.x * 20., y: -d.y * 20. }, None),
        };
        let mut windows = self.windows.borrow_mut();
        let w = windows.entry(window).or_default();
        let (phase, momentum) = w.momentum.classify(finger);
        let input = ScrollInput { position: at, delta, phase, momentum, time: self.now() };
        self.with_hit(tree.0, tree.1, tree.2, at, |hit| w.router.scroll(&input, hit))
    }

    pub fn os_gesture(&self, window: WindowId, event: GestureEvent, at: Point, tree: Tree<'_>) -> Option<Output> {
        let windows = self.windows.borrow();
        let router = windows.get(&window).map(|w| &w.router);
        let fallback = GestureRouter::default();
        let router = router.unwrap_or(&fallback);
        self.with_hit(tree.0, tree.1, tree.2, at, |hit| router.route(event, hit))
    }

    /// A pressure event: the step into the force stage is a force click.
    pub fn pressure(&self, window: WindowId, stage: PressureStage) -> bool {
        let mut windows = self.windows.borrow_mut();
        let w = windows.entry(window).or_default();
        let force = stage == PressureStage::Force;
        let fired = force && !w.force;
        w.force = force;
        fired
    }

    /// Advances flings, springs and timers to now. Returns what changed and whether another
    /// frame is needed.
    pub fn step(&self, window: WindowId) -> (Vec<Output>, bool) {
        let mut windows = self.windows.borrow_mut();
        let Some(w) = windows.get_mut(&window) else { return (Vec::new(), false) };
        let out = w.router.step(self.now());
        (out, w.router.needs_frame())
    }

    /// The scroll view's offset, after registering its extent from the last paint.
    pub fn scroll_offset(&self, window: WindowId, node: NodeId, set: &GestureSet, viewport: Option<Rect>) -> Point {
        let content = self.content.borrow().get(&node).copied();
        let mut windows = self.windows.borrow_mut();
        let w = windows.entry(window).or_default();
        if let (Some(viewport), Some(content)) = (viewport, content) {
            let extent = Size {
                width: content.origin.x + content.size.width - viewport.origin.x,
                height: content.origin.y + content.size.height - viewport.origin.y,
            };
            w.router.set_scroll_extent(node, set, viewport.size, extent);
        }
        w.router.scroll_offset(node)
    }

    /// A virtualized list's offset in the router as input and flings left it, `None` before the
    /// list is registered.
    pub fn list_offset(&self, window: WindowId, node: NodeId) -> Option<f32> {
        let windows = self.windows.borrow();
        windows.get(&window)?.router.scroller(node).map(|s| s.offset().y)
    }

    /// Puts the list's scroller on the model: registers the scrollable `range` the model reports,
    /// then moves the scroller's coordinates so its in-range offset is the model's `position`,
    /// keeping a fling or bounce running. Returns the offset after, rubber band included.
    pub fn sync_list(
        &self,
        window: WindowId,
        node: NodeId,
        set: &GestureSet,
        viewport: Size,
        range: f32,
        position: f32,
    ) -> f32 {
        let mut windows = self.windows.borrow_mut();
        let w = windows.entry(window).or_default();
        let content = Size { width: viewport.width, height: viewport.height + range.max(0.) };
        w.router.set_scroll_extent(node, set, viewport, content);
        let Some(s) = w.router.scroller_mut(node) else { return position };
        let offset = s.offset().y;
        let edge = offset.clamp(0., range.max(0.));
        if (edge - position).abs() > 0.25 {
            s.shift(Point { x: 0., y: position - edge });
        }
        s.offset().y
    }

    pub fn row_config(&self, window: WindowId, node: NodeId, set: &GestureSet, width: f32) {
        self.windows.borrow_mut().entry(window).or_default().router.set_row_config(node, set, width);
    }

    pub fn forget_window(&self, window: WindowId) {
        self.windows.borrow_mut().remove(&window);
    }

    pub fn forget(&self, node: NodeId) {
        self.content.borrow_mut().remove(&node);
        for w in self.windows.borrow_mut().values_mut() {
            w.router.forget(node);
        }
    }

    /// Forgets the scroll view's content extent before its children prepaint again.
    pub fn clear_content(&self, node: NodeId) {
        self.content.borrow_mut().remove(&node);
    }

    /// Wraps a scroll view's child so it lays out in place and paints shifted by the offset.
    pub fn shifted(&self, scroller: NodeId, offset: Point, child: AnyElement) -> AnyElement {
        Shifted { scroller, offset, inner: child, content: self.content.clone() }.into_any_element()
    }
}

/// Applies the router's outputs: events go onto the queue for the app, scroll offsets need a
/// redraw (the caller's), haptics are a no-op off macOS.
pub(crate) fn deliver(events: &EventQueue, outputs: Vec<Output>) -> (bool, bool) {
    let (mut queued, mut scrolled) = (false, false);
    for output in outputs {
        match output {
            Output::Event(node, event) => {
                emit_gesture(&events.emitter(node), event);
                queued = true;
            }
            Output::Scroll { .. } => scrolled = true,
            Output::Haptic(_) => {}
        }
    }
    (queued, scrolled)
}

pub(crate) fn pinch_event(event: &gpui::PinchEvent) -> GestureEvent {
    let phase = match event.phase {
        TouchPhase::Started => GesturePhase::Began,
        TouchPhase::Moved => GesturePhase::Changed,
        TouchPhase::Ended => GesturePhase::Ended,
        TouchPhase::Cancelled => GesturePhase::Cancelled,
    };
    GestureEvent::Pinch(PinchEvent {
        position: Point { x: event.position.x.as_f32(), y: event.position.y.as_f32() },
        scale: 1. + event.delta,
        phase,
    })
}

pub(crate) fn force_click(at: Point) -> GestureEvent {
    GestureEvent::ForceClick(ForceClickEvent { position: at })
}

/// A scroll view's child, laid out where Taffy put it and painted shifted by the scroll offset.
/// Its unshifted bounds feed the scroll view's content extent.
struct Shifted {
    scroller: NodeId,
    offset: Point,
    inner: AnyElement,
    content: Rc<RefCell<HashMap<NodeId, Rect>>>,
}

impl IntoElement for Shifted {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl gpui::Element for Shifted {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut gpui::Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        (self.inner.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut gpui::Window,
        cx: &mut App,
    ) {
        let rect = Rect::new(
            bounds.origin.x.as_f32(),
            bounds.origin.y.as_f32(),
            bounds.size.width.as_f32(),
            bounds.size.height.as_f32(),
        );
        {
            let mut content = self.content.borrow_mut();
            let union = content.entry(self.scroller).or_insert(rect);
            let right = (union.origin.x + union.size.width).max(rect.origin.x + rect.size.width);
            let bottom = (union.origin.y + union.size.height).max(rect.origin.y + rect.size.height);
            union.origin.x = union.origin.x.min(rect.origin.x);
            union.origin.y = union.origin.y.min(rect.origin.y);
            union.size.width = right - union.origin.x;
            union.size.height = bottom - union.origin.y;
        }
        let shift = point(px(-self.offset.x), px(-self.offset.y));
        window.with_element_offset(shift, |window| self.inner.prepaint(window, cx));
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut gpui::Window,
        cx: &mut App,
    ) {
        self.inner.paint(window, cx);
    }
}
