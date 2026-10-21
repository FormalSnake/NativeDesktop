//! Gestures on GPUI backends (ARCHITECTURE.md 6.1). GPUI hands over raw pointer, scroll, pinch and
//! pressure events; `nd-gesture` recognizes them over nd's own hit path (the node bounds the last
//! paint recorded), arbitrates nested recognizers, and runs scroll momentum and rubber banding on
//! the frame clock, since GPUI never reads `momentumPhase` and Wayland sends no inertia
//! (zed#14473). Scroll views are drawn with their content shifted by the router's offset; GPUI's
//! own div scrolling is not used, so there is one scroll model per window.

use nd_core::time::Instant;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use nd_core::{NodeId, Point, Rect, Size, WindowId};
use nd_gesture::{
    ForceClickEvent, GestureEvent, GesturePhase, GestureRouter, GestureSet, HitEntry, MomentumInference, Output,
    PinchEvent, PointerInput, PointerPhase, ScrollInput, ScrollPhase,
};
use nd_gpui::gpui::{
    self, AnyElement, App, Bounds, ElementId, GlobalElementId, InspectorElementId, IntoElement, LayoutId, Pixels,
    PressureStage, ScrollDelta, TouchPhase, point, px,
};
use nd_ui::backend::NodeKind;
use nd_ui::events::{EventQueue, emit_gesture};
use nd_ui::{DivProps, Modality, OverlayProps};

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
    /// The OS's animations-off setting: a wheel notch then jumps instead of easing.
    reduce_motion: Rc<std::cell::Cell<bool>>,
    /// The OS's own say on easing a wheel notch, over the platform default.
    smooth_wheel: Rc<std::cell::Cell<Option<bool>>>,
}

impl Default for Gestures {
    fn default() -> Self {
        Self {
            windows: Rc::default(),
            content: Rc::default(),
            origin: Instant::now(),
            reduce_motion: Rc::default(),
            smooth_wheel: Rc::default(),
        }
    }
}

/// `ND_DEBUG_GESTURES=1` prints where each press and scroll is routed (the nodes under the
/// pointer that take gestures, innermost first), to read a routing bug off a user's machine.
fn debug() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("ND_DEBUG_GESTURES").is_some_and(|v| v == "1"))
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

/// A `visibility: Hidden` view keeps its box but shows nothing; nothing in it is hit.
fn hidden(node: &Node) -> bool {
    node.props.downcast_ref::<DivProps>().is_some_and(|d| d.style.visibility == Some(gpui::Visibility::Hidden))
}

fn in_hidden(nodes: &HashMap<NodeId, Node>, mut at: NodeId) -> bool {
    while let Some(node) = nodes.get(&at) {
        if hidden(node) {
            return true;
        }
        let Some(parent) = node.parent else { return false };
        at = parent;
    }
    false
}

fn contains(r: &Rect, p: Point) -> bool {
    p.x >= r.origin.x && p.y >= r.origin.y && p.x < r.origin.x + r.size.width && p.y < r.origin.y + r.size.height
}

/// Where a hit test starts. Overlays draw above all content (`crate::overlay`): a point inside
/// the topmost one starts there, and a point on an input-taking overlay's backdrop hits nothing,
/// so no gesture under an overlay ever fires. Scrolls pass a light-dismiss backdrop, as GPUI's
/// own scroll handlers do.
fn hit_start(
    nodes: &HashMap<NodeId, Node>,
    bounds: &HashMap<NodeId, Rect>,
    root: NodeId,
    at: Point,
    scroll: bool,
) -> Option<NodeId> {
    let mut overlays: Vec<(NodeId, Modality)> = nodes
        .iter()
        .filter_map(|(id, n)| n.props.downcast_ref::<OverlayProps>().map(|p| (*id, p.modality)))
        .filter(|(id, m)| *m != Modality::Passive && !in_hidden(nodes, *id))
        .collect();
    // Node ids grow with creation, and a nested overlay is created after the one holding it.
    overlays.sort_by_key(|(id, _)| std::cmp::Reverse(*id));
    if let Some((inside, _)) = overlays.iter().find(|(o, _)| bounds.get(o).is_some_and(|b| contains(b, at))) {
        return Some(*inside);
    }
    let blocks = |m: &Modality| !scroll || *m == Modality::Modal;
    if overlays.iter().any(|(_, m)| blocks(m)) { None } else { Some(root) }
}

/// Nodes under `at`, innermost first: from the hit start, into the topmost child (the last
/// painted) whose bounds contain the point.
fn hit_path(
    nodes: &HashMap<NodeId, Node>,
    bounds: &HashMap<NodeId, Rect>,
    root: NodeId,
    at: Point,
    scroll: bool,
) -> Vec<NodeId> {
    let Some(root) = hit_start(nodes, bounds, root, at, scroll) else { return Vec::new() };
    let mut path = descend(nodes, bounds, root, at);
    path.reverse();
    path
}

/// The path from `node` to the innermost node under `at`, outermost first. Of overlapping
/// children the topmost (last painted) wins, unless nothing in it takes a gesture and an earlier
/// one does: a layer drawn over the content that is empty where the pointer is (the scaffold's
/// overlay pane, a full-size toast host) lets the gesture through to what is under it, as GPUI
/// lets a click through an element with no hitbox.
fn descend(nodes: &HashMap<NodeId, Node>, bounds: &HashMap<NodeId, Rect>, node: NodeId, at: Point) -> Vec<NodeId> {
    let mut first: Option<Vec<NodeId>> = None;
    if let Some(n) = nodes.get(&node) {
        for c in n.children.iter().rev() {
            if nodes.get(c).is_some_and(hidden) || !bounds.get(c).is_some_and(|b| contains(b, at)) {
                continue;
            }
            let path = descend(nodes, bounds, *c, at);
            if path.iter().any(|p| nodes.get(p).and_then(gestures_of).is_some()) {
                return std::iter::once(node).chain(path).collect();
            }
            first.get_or_insert(path);
        }
    }
    std::iter::once(node).chain(first.unwrap_or_default()).collect()
}

/// A window's retained tree as the hit test reads it: nodes, painted bounds, root.
pub(crate) type Tree<'a> = (&'a HashMap<NodeId, Node>, &'a HashMap<NodeId, Rect>, NodeId);

impl Gestures {
    pub(crate) fn set_reduce_motion(&self, reduce: bool) {
        self.reduce_motion.set(reduce);
    }

    pub(crate) fn set_smooth_wheel(&self, smooth: Option<bool>) {
        self.smooth_wheel.set(smooth);
    }

    fn window_state(&self) -> WindowGestures {
        let mut config = nd_gesture::GestureConfig::default();
        if self.reduce_motion.get() || self.smooth_wheel.get() == Some(false) {
            config.wheel_glide = 0.;
        }
        WindowGestures { router: GestureRouter::new(config), ..WindowGestures::default() }
    }

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
        scroll: bool,
        f: impl FnOnce(&[HitEntry<'_>]) -> R,
    ) -> R {
        let path = hit_path(nodes, bounds, root, at, scroll);
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
        let w = windows.entry(window).or_insert_with(|| self.window_state());
        if phase == PointerPhase::Down {
            self.with_hit(tree.0, tree.1, tree.2, at, false, |hit| {
                if debug() {
                    let path: Vec<(NodeId, Rect)> = hit.iter().map(|h| (h.node, h.bounds)).collect();
                    eprintln!("nd: press at {at:?} to {path:?}");
                }
                w.router.pointer(&input, hit)
            })
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
        let w = windows.entry(window).or_insert_with(|| self.window_state());
        let (phase, momentum) = w.momentum.classify(finger);
        let input = ScrollInput { position: at, delta, phase, momentum, time: self.now() };
        self.with_hit(tree.0, tree.1, tree.2, at, true, |hit| {
            if debug() {
                let path: Vec<(NodeId, Rect)> = hit.iter().map(|h| (h.node, h.bounds)).collect();
                eprintln!("nd: scroll at {at:?} {:?} {:?} to {path:?}", input.phase, input.delta);
            }
            w.router.scroll(&input, hit)
        })
    }

    pub fn os_gesture(&self, window: WindowId, event: GestureEvent, at: Point, tree: Tree<'_>) -> Option<Output> {
        let windows = self.windows.borrow();
        let router = windows.get(&window).map(|w| &w.router);
        let fallback = GestureRouter::default();
        let router = router.unwrap_or(&fallback);
        self.with_hit(tree.0, tree.1, tree.2, at, false, |hit| router.route(event, hit))
    }

    /// A pressure event: the step into the force stage is a force click.
    pub fn pressure(&self, window: WindowId, stage: PressureStage) -> bool {
        let mut windows = self.windows.borrow_mut();
        let w = windows.entry(window).or_insert_with(|| self.window_state());
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
        let w = windows.entry(window).or_insert_with(|| self.window_state());
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
        let w = windows.entry(window).or_insert_with(|| self.window_state());
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
        self.windows
            .borrow_mut()
            .entry(window)
            .or_insert_with(|| self.window_state())
            .router
            .set_row_config(node, set, width);
    }

    pub fn forget_window(&self, window: WindowId) {
        self.windows.borrow_mut().remove(&window);
    }

    /// Moves the physics of `nodes` to another window, which a view moved to.
    pub fn hand_over(&self, from: WindowId, to: WindowId, nodes: &[NodeId]) {
        let mut windows = self.windows.borrow_mut();
        let Some(mut source) = windows.remove(&from) else { return };
        let target = windows.entry(to).or_insert_with(|| self.window_state());
        for node in nodes {
            source.router.hand_over(*node, &mut target.router);
        }
        windows.insert(from, source);
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
/// `child` painted (and hit) `by` away from where layout put it.
pub(crate) fn offset(child: AnyElement, by: gpui::Point<Pixels>) -> AnyElement {
    Offset { by, inner: child }.into_any_element()
}

struct Offset {
    by: gpui::Point<Pixels>,
    inner: AnyElement,
}

impl IntoElement for Offset {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl gpui::Element for Offset {
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
        _bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut gpui::Window,
        cx: &mut App,
    ) {
        window.with_element_offset(self.by, |window| self.inner.prepaint(window, cx));
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

#[cfg(test)]
mod tests {
    use super::*;
    use nd_gesture::GestureSet;

    fn view(parent: Option<NodeId>, children: Vec<NodeId>, tap: bool) -> Node {
        let props = DivProps {
            style: Default::default(),
            scope: None,
            drag_region: false,
            titlebar_insets: false,
            gestures: GestureSet { tap, ..GestureSet::default() },
            role: None,
            transform: Default::default(),
            hover: None,
            tracks_hover: false,
            reports_size: false,
        };
        Node { kind: NodeKind::View, props: Rc::new(props), parent, children }
    }

    #[test]
    fn an_empty_layer_over_the_content_lets_the_gesture_through() {
        let (root, content, layer) = (NodeId(1), NodeId(2), NodeId(3));
        let nodes = HashMap::from([
            (root, view(None, vec![content, layer], false)),
            (content, view(Some(root), vec![], true)),
            (layer, view(Some(root), vec![], false)),
        ]);
        let full = Rect::new(0., 0., 100., 100.);
        let bounds = HashMap::from([(root, full), (content, full), (layer, full)]);
        let path = hit_path(&nodes, &bounds, root, Point { x: 50., y: 50. }, false);
        assert_eq!(path, vec![content, root], "the tap reaches the content under the empty layer");
    }

    #[test]
    fn the_topmost_layer_wins_when_it_takes_the_gesture() {
        let (root, under, over) = (NodeId(1), NodeId(2), NodeId(3));
        let nodes = HashMap::from([
            (root, view(None, vec![under, over], false)),
            (under, view(Some(root), vec![], true)),
            (over, view(Some(root), vec![], true)),
        ]);
        let full = Rect::new(0., 0., 100., 100.);
        let bounds = HashMap::from([(root, full), (under, full), (over, full)]);
        assert_eq!(hit_path(&nodes, &bounds, root, Point { x: 50., y: 50. }, false), vec![over, root]);
    }
}
