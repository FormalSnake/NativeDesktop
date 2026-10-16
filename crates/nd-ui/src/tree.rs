//! The retained tree and the reconciler (ARCHITECTURE.md 3.7, 3.14).
//!
//! One `Tree` per window. A frame re-renders only the views notified since the last one, top
//! down, and reconciles each view's output against its own retained subtree: children match by
//! identity, props compare with `PartialEq`, handlers swap without a backend call. A view's
//! output is spliced into its parent's native children, so views add no native node or layout
//! box of their own. Ops go into one transaction per window per frame, with layout inside it.

use std::any::TypeId;
use std::collections::{HashMap, HashSet};
use std::mem;
use std::rc::Rc;

use nd_core::{NodeId, Rect, Size, WindowId};
use nd_gpui::gpui::{KeyBinding, StyleRefinement};
use nd_gpui::taffy::{self, TaffyTree, prelude::TaffyMaxContent};

use crate::animation::{self, MotionSpec, MotionState, Transform, Values};
use crate::app::{AnyView, App, EntityId};
use crate::backend::{AvailableSpace, Backend, BoundaryInsets, Constraint, NodeKind, Props, PropsPatch};
use crate::canvas::{self, CanvasProps, PaintFn};
use crate::dispatch::DispatchTree;
use crate::element::{AnyElement, DispatchProps, DivProps, Element, Handler, Key, NodeElement, RootProps};
use crate::layout::{to_taffy, with_titlebar_insets};
use crate::overlay::{Modality, OverlayProps, is_overlay};
use crate::transaction::{MeasureCache, Op, Transaction};
use crate::window::{TitlebarInsets, Window};
use nd_gpui::layout::MinMax;

mod semantic_walk;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum SlotKind {
    Node(NodeKind),
    View(TypeId),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Ident {
    kind: SlotKind,
    key: Key,
    occurrence: u32,
}

/// A child position in the element tree: a native node, or a view whose output sits there.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Slot {
    Node(NodeId),
    View(EntityId),
}

struct RNode {
    ident: Ident,
    props: Props,
    layout: StyleRefinement,
    measured: bool,
    /// Element children, views included.
    slots: Vec<Slot>,
    /// Native children as the backend has them: `slots` with views flattened to their roots.
    native: Vec<NodeId>,
    handlers: Vec<(TypeId, Handler)>,
    dispatch: DispatchProps,
    taffy: taffy::NodeId,
    frame: Option<Rect>,
    /// Created this frame; its own children are not inserted yet.
    fresh: bool,
    /// The box a native layout container gives this node (3.15 layout boundaries); its own
    /// style's position and size are replaced by it.
    boundary: Option<Boundary>,
    /// A `div` that animates: its motions and what its element last asked for. `props` and
    /// `layout` above are what is presented.
    motion: Option<Animated>,
    /// A canvas's paint closure; its props hold what it painted, for the size they name.
    paint: Option<PaintFn>,
}

/// The size a canvas lays out at before its first frame: its style's, when given in points.
fn styled_size(layout: &StyleRefinement) -> (f32, f32) {
    let v = animation::values(layout, &Transform::default());
    (v[4].unwrap_or(0.) as f32, v[5].unwrap_or(0.) as f32)
}

/// See [`RNode::motion`].
struct Animated {
    state: MotionState,
    props: Props,
    layout: StyleRefinement,
}

/// A div's props and layout with `values` presented in place of its own.
fn present_div(props: &Props, layout: &StyleRefinement, values: &Values) -> (Props, StyleRefinement) {
    let mut div = props.downcast_ref::<DivProps>().expect("only divs animate").clone();
    animation::present(&mut div.style, &mut div.transform, values);
    let mut layout = layout.clone();
    animation::present(&mut layout, &mut Transform::default(), values);
    (Rc::new(div), layout)
}

struct ViewSlot {
    view: AnyView,
    ident: Ident,
    root: Option<Slot>,
    /// The native node the view's output is inserted into.
    parent: NodeId,
    /// Views above it; dirty views render shallowest first.
    depth: u32,
}

/// A mounted view's retained subtree, lifted out of one window's tree for another to adopt.
pub(crate) struct Lifted {
    top: EntityId,
    /// Its root is still attached in the window it left.
    attached: bool,
    /// The lifted view first, then the views under it.
    views: Vec<(EntityId, ViewSlot)>,
    nodes: Vec<(NodeId, RNode)>,
    overlays: Vec<NodeId>,
}

pub(crate) struct Tree {
    window: WindowId,
    root: NodeId,
    root_view: AnyView,
    nodes: HashMap<NodeId, RNode>,
    views: HashMap<EntityId, ViewSlot>,
    taffy: TaffyTree<NodeId>,
    tx: Transaction,
    needs_layout: bool,
    insets: TitlebarInsets,
    /// Views placed in this frame's output so far; a second placement is a render bug.
    placed: HashSet<EntityId>,
    /// Views whose old position is gone this frame. They keep their nodes until the frame ends,
    /// so a view that moved to a parent reconciled later is adopted there, not rebuilt.
    orphans: Vec<EntityId>,
    /// Orphans kept, detached, until every window drew this frame, so another window can adopt
    /// them with their native nodes (a view moved between windows never rebuilds, so a web view
    /// in it never reloads). Only while the app has more than one window.
    parked: Vec<EntityId>,
    /// The app frame this tree last drew in.
    drawn_in: u64,
    /// Views rendered in this frame, mounted or re-rendered. One that notifies itself while
    /// rendering stays dirty for the next frame instead of rendering again in this one.
    rendered: HashSet<EntityId>,
    /// Mirrors the native nodes as the backend has them, with each node's key context and
    /// action listeners.
    /// Nodes that emitted an event since the last frame (see [`Op::Reaffirm`]).
    pub(crate) reaffirm: HashSet<NodeId>,
    pub(crate) dispatch: DispatchTree<App>,
    /// Open overlays, oldest first. Their focus (save, trap, restore, Escape) is the dispatch
    /// tree's focus scopes; this list serves app deactivation.
    overlays: Vec<NodeId>,
    /// Nodes with a motion in flight; the window needs frames while there are any.
    animating: HashSet<NodeId>,
    /// The motion [`Tree::animate`] built for the node being mounted or reconciled.
    pending_motion: Option<Animated>,
}

fn idents(elements: &[AnyElement]) -> Vec<Ident> {
    let mut seen: HashMap<(SlotKind, Key), u32> = HashMap::new();
    elements
        .iter()
        .map(|el| {
            let (kind, key) = match &el.0 {
                Element::Node(n) => (SlotKind::Node(n.kind), n.key.clone()),
                Element::View(v) => (SlotKind::View(v.view_type()), Key::Entity(v.entity_id())),
                Element::Deferred(_) => unreachable!("deferred elements are built before matching"),
            };
            let count = seen.entry((kind, key.clone())).or_default();
            let occurrence = *count;
            *count += 1;
            Ident { kind, key, occurrence }
        })
        .collect()
}

/// What a native layout container gives a child node.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Boundary {
    /// A fixed box, with edges kept clear inside it.
    Box(Size, BoundaryInsets),
    /// A fixed width; the height is the content's (a row a native list hosts).
    Width(f32),
}

/// The node's own style, or, inside a native layout container, that style pinned to the box
/// the container gives it.
fn pin_to_boundary(mut style: taffy::Style, boundary: Option<Boundary>) -> taffy::Style {
    if let Some(Boundary::Width(width)) = boundary {
        style.position = taffy::Position::Absolute;
        style.inset = taffy::Rect {
            left: taffy::LengthPercentageAuto::length(0.),
            top: taffy::LengthPercentageAuto::length(0.),
            right: taffy::LengthPercentageAuto::auto(),
            bottom: taffy::LengthPercentageAuto::auto(),
        };
        style.margin = taffy::Rect::zero();
        style.size = taffy::Size { width: taffy::Dimension::length(width), height: taffy::Dimension::auto() };
        style.min_size.width = MinMax::auto();
        style.max_size.width = MinMax::auto();
        return style;
    }
    if let Some(Boundary::Box(size, insets)) = boundary {
        let add = |p: taffy::LengthPercentage, by: f32| match p.into_raw().tag() {
            taffy::CompactLength::LENGTH_TAG => taffy::LengthPercentage::length(p.into_raw().value() + by),
            _ => taffy::LengthPercentage::length(by),
        };
        style.padding = taffy::Rect {
            top: add(style.padding.top, insets.top),
            right: add(style.padding.right, insets.right),
            bottom: add(style.padding.bottom, insets.bottom),
            left: add(style.padding.left, insets.left),
        };
        style.position = taffy::Position::Absolute;
        style.inset = taffy::Rect {
            left: taffy::LengthPercentageAuto::length(0.),
            top: taffy::LengthPercentageAuto::length(0.),
            right: taffy::LengthPercentageAuto::auto(),
            bottom: taffy::LengthPercentageAuto::auto(),
        };
        style.margin = taffy::Rect::zero();
        style.size =
            taffy::Size { width: taffy::Dimension::length(size.width), height: taffy::Dimension::length(size.height) };
        style.min_size = taffy::Size::auto();
        style.max_size = taffy::Size::auto();
    }
    style
}

fn pads(props: &Props) -> bool {
    props.downcast_ref::<DivProps>().is_some_and(|p| p.titlebar_insets)
}

fn root_style(size: Size) -> taffy::Style {
    taffy::Style {
        display: taffy::Display::Flex,
        flex_direction: taffy::FlexDirection::Column,
        size: taffy::Size {
            width: taffy::Dimension::length(size.width),
            height: taffy::Dimension::length(size.height),
        },
        ..Default::default()
    }
}

impl Tree {
    pub(crate) fn new(
        window: WindowId,
        root: NodeId,
        size: Size,
        root_view: AnyView,
        bindings: Vec<KeyBinding>,
    ) -> Self {
        let mut taffy = TaffyTree::new();
        taffy.disable_rounding();
        let root_taffy = taffy.new_leaf(root_style(size)).expect("taffy root");
        let mut tx = Transaction::new(window);
        let props: Props = Rc::new(RootProps);
        tx.push(Op::Create { node: root, kind: NodeKind::Root, props: props.clone() });
        let mut nodes = HashMap::new();
        nodes.insert(
            root,
            RNode {
                ident: Ident { kind: SlotKind::Node(NodeKind::Root), key: Key::Id("root".into()), occurrence: 0 },
                props,
                layout: StyleRefinement::default(),
                measured: false,
                slots: Vec::new(),
                native: Vec::new(),
                handlers: Vec::new(),
                dispatch: DispatchProps::default(),
                taffy: root_taffy,
                frame: None,
                fresh: false,
                boundary: None,
                motion: None,
                paint: None,
            },
        );
        let mut dispatch = DispatchTree::new(bindings);
        dispatch.set_root(root);
        Self {
            window,
            root,
            root_view,
            nodes,
            views: HashMap::new(),
            taffy,
            tx,
            needs_layout: true,
            insets: TitlebarInsets::default(),
            placed: HashSet::new(),
            orphans: Vec::new(),
            parked: Vec::new(),
            drawn_in: 0,
            reaffirm: HashSet::new(),
            rendered: HashSet::new(),
            dispatch,
            overlays: Vec::new(),
            animating: HashSet::new(),
            pending_motion: None,
        }
    }

    pub(crate) fn contains(&self, node: NodeId) -> bool {
        self.nodes.contains_key(&node)
    }

    pub(crate) fn handler(&self, node: NodeId, event: TypeId) -> Option<Handler> {
        self.nodes.get(&node)?.handlers.iter().find(|(t, _)| *t == event).map(|(_, h)| h.clone())
    }

    /// Open overlays, oldest first.
    pub(crate) fn open_overlays(&self) -> impl Iterator<Item = (NodeId, Modality)> + '_ {
        self.overlays.iter().filter_map(|n| Some((*n, self.overlay_modality(*n)?)))
    }

    fn overlay_modality(&self, node: NodeId) -> Option<Modality> {
        self.nodes.get(&node)?.props.downcast_ref::<OverlayProps>().map(|p| p.modality)
    }

    /// The node `window.focus(id)` lands on: the first node keyed `id` in tree order that is
    /// focusable or has a focusable descendant, which is then the one focused.
    pub(crate) fn focus_target(&self, id: &str) -> Option<NodeId> {
        fn first_focusable(tree: &Tree, node: NodeId) -> Option<NodeId> {
            if tree.dispatch.is_focusable(node) {
                return Some(node);
            }
            tree.nodes.get(&node)?.native.iter().find_map(|c| first_focusable(tree, *c))
        }
        let mut stack = vec![self.root];
        while let Some(n) = stack.pop() {
            let node = &self.nodes[&n];
            if matches!(&node.ident.key, Key::Id(k) if k.as_ref() == id)
                && let Some(target) = first_focusable(self, n)
            {
                return Some(target);
            }
            stack.extend(node.native.iter().rev().copied());
        }
        None
    }

    pub(crate) fn needs_frame(&self) -> bool {
        self.needs_layout || !self.tx.is_empty() || !self.reaffirm.is_empty() || !self.animating.is_empty()
    }

    /// `el` as presented at `now`: a div that animates gets its motions started or retargeted
    /// and the sampled values in place of its own. `node` is its retained node, if it has one.
    fn animate(
        &mut self,
        node: Option<NodeId>,
        id: NodeId,
        mut el: NodeElement,
        now: f64,
        reduce: bool,
    ) -> NodeElement {
        let previous = node.and_then(|n| self.nodes.get_mut(&n)).and_then(|n| n.motion.take());
        let (Some(spec), Some(div)) = (el.motion, el.props.downcast_ref::<DivProps>()) else {
            self.animating.remove(&id);
            return el;
        };
        let targets = animation::values(&el.layout, &div.transform);
        let mut state = match previous {
            Some(mut previous) => {
                previous.state.retarget(spec.animation, targets, now, reduce);
                previous.state
            }
            // Only a node mounting now comes in from its enter pose.
            None if node.is_some() => MotionState::new(MotionSpec { enter: None, ..spec }, targets, now, reduce),
            None => MotionState::new(spec, targets, now, reduce),
        };
        state.settle(now);
        let base = (el.props.clone(), el.layout.clone());
        if state.is_moving() {
            (el.props, el.layout) = present_div(&el.props, &el.layout, &state.sample(now));
            self.animating.insert(id);
        } else {
            self.animating.remove(&id);
        }
        self.pending_motion = Some(Animated { state, props: base.0, layout: base.1 });
        el
    }

    /// Advances every motion in flight to `now`: presented values reach the backend as
    /// updates. Returns the nodes whose motions all came to rest.
    pub(crate) fn step_motion(&mut self, now: f64) -> Vec<NodeId> {
        let mut ended = Vec::new();
        let mut ids: Vec<NodeId> = self.animating.iter().copied().collect();
        ids.sort();
        for id in ids {
            let Some(node) = self.nodes.get_mut(&id) else {
                self.animating.remove(&id);
                continue;
            };
            let Some(anim) = node.motion.as_mut() else {
                self.animating.remove(&id);
                continue;
            };
            if anim.state.settle(now) {
                ended.push(id);
            }
            let moving = anim.state.is_moving();
            let (props, layout) = present_div(&anim.props, &anim.layout, &anim.state.sample(now));
            if !node.props.props_eq(&*props) {
                let patch = PropsPatch { old: mem::replace(&mut node.props, props), new: node.props.clone() };
                self.tx.push(Op::Update { node: id, patch });
            }
            if node.layout != layout {
                node.layout = layout;
                let node = &self.nodes[&id];
                let style = self.taffy_style(&node.layout, &node.props, node.boundary);
                self.taffy.set_style(node.taffy, style).expect("taffy node");
                self.needs_layout = true;
            }
            if !moving {
                self.animating.remove(&id);
            }
        }
        ended
    }

    /// Paints canvas `n` again for `width` x `height`, when that is not the size it was painted
    /// for.
    pub(crate) fn repaint_canvas(&mut self, n: NodeId, width: f32, height: f32, app: &mut App) {
        let Some(node) = self.nodes.get(&n) else { return };
        let Some(paint) = node.paint.clone() else { return };
        if node.props.downcast_ref::<CanvasProps>().is_some_and(|p| (p.width, p.height) == (width, height)) {
            return;
        }
        let props: Props = Rc::new(canvas::record(&paint, node.layout.clone(), width, height, app));
        let node = self.nodes.get_mut(&n).expect("node");
        let patch = PropsPatch { old: mem::replace(&mut node.props, props), new: node.props.clone() };
        self.tx.push(Op::Update { node: n, patch });
    }

    /// Canvases laid out at a size they were not painted for, with that size.
    fn stale_canvases(&self) -> Vec<(NodeId, f32, f32)> {
        let mut out: Vec<(NodeId, f32, f32)> = self
            .nodes
            .iter()
            .filter(|(_, n)| n.paint.is_some())
            .filter_map(|(id, n)| {
                let f = n.frame?;
                let p = n.props.downcast_ref::<CanvasProps>()?;
                ((p.width, p.height) != (f.size.width, f.size.height)).then_some((*id, f.size.width, f.size.height))
            })
            .collect();
        out.sort_by_key(|c| c.0);
        out
    }

    /// Marks every measured node for measuring again.
    pub(crate) fn remeasure(&mut self) {
        for node in self.nodes.values().filter(|n| n.measured) {
            let _ = self.taffy.mark_dirty(node.taffy);
        }
        self.needs_layout = true;
    }

    pub(crate) fn view_node(&self, view: EntityId) -> Option<NodeId> {
        self.native_of(self.views.get(&view)?.root?)
    }

    pub(crate) fn resize(&mut self, size: Size) {
        let root = self.nodes[&self.root].taffy;
        self.taffy.set_style(root, root_style(size)).expect("taffy root");
        self.needs_layout = true;
    }

    pub(crate) fn root_view_id(&self) -> EntityId {
        self.root_view.entity_id()
    }

    /// Re-lays out the elements padded by the titlebar insets.
    pub(crate) fn set_titlebar_insets(&mut self, insets: TitlebarInsets) {
        if self.insets == insets {
            return;
        }
        self.insets = insets;
        let padded: Vec<NodeId> = self.nodes.iter().filter(|(_, n)| pads(&n.props)).map(|(id, _)| *id).collect();
        for id in padded {
            let node = &self.nodes[&id];
            let style = self.taffy_style(&node.layout, &node.props, node.boundary);
            self.taffy.set_style(self.nodes[&id].taffy, style).expect("taffy node");
        }
        self.needs_layout = true;
    }

    /// The node's Taffy style: padded by the titlebar insets when it asks for them, pinned to
    /// the box a native layout container gives it.
    fn taffy_style(&self, layout: &StyleRefinement, props: &Props, boundary: Option<Boundary>) -> taffy::Style {
        let style = if pads(props) { to_taffy(&with_titlebar_insets(layout, self.insets)) } else { to_taffy(layout) };
        pin_to_boundary(style, boundary)
    }

    /// Releases every node in one last transaction.
    pub(crate) fn close(mut self, app: &mut App) {
        let root = self.root;
        let slots = mem::take(&mut self.nodes.get_mut(&root).expect("root").slots);
        for slot in slots {
            self.unmount(slot);
        }
        self.orphans.append(&mut self.parked);
        self.release_orphans(app);
        self.sync_children(root);
        self.tx.push(Op::Release { node: root });
        let tx = mem::replace(&mut self.tx, Transaction::new(self.window));
        tx.commit(&mut *app.backend, &mut app.measure_cache);
    }

    /// Queues nd's latest focus change for the backend; the transaction applies it last.
    pub(crate) fn push_focus_change(&mut self) {
        if let Some(node) = self.dispatch.take_focus_change() {
            self.tx.push(Op::Focus { node });
        }
    }

    fn native_of(&self, slot: Slot) -> Option<NodeId> {
        match slot {
            Slot::Node(n) => Some(n),
            Slot::View(e) => self.native_of(self.views.get(&e)?.root?),
        }
    }

    fn ident_of(&self, slot: Slot) -> &Ident {
        match slot {
            Slot::Node(n) => &self.nodes[&n].ident,
            Slot::View(e) => &self.views[&e].ident,
        }
    }

    fn render_view(&mut self, view: &AnyView, window: &mut Window, app: &mut App) -> AnyElement {
        let id = view.entity_id();
        app.dirty.remove(&id);
        self.rendered.insert(id);
        *app.render_counts.entry(id).or_default() += 1;
        view.render(window, app).resolve(window, app)
    }

    fn mount(
        &mut self,
        el: AnyElement,
        ident: Ident,
        parent: NodeId,
        depth: u32,
        window: &mut Window,
        app: &mut App,
    ) -> Slot {
        match el.0 {
            Element::View(view) => self.mount_view(view, ident, parent, depth, window, app),
            Element::Node(node) => Slot::Node(self.mount_node(*node, ident, depth, window, app)),
            Element::Deferred(_) => unreachable!("deferred elements are built before mounting"),
        }
    }

    fn mount_node(&mut self, el: NodeElement, ident: Ident, depth: u32, window: &mut Window, app: &mut App) -> NodeId {
        let id = app.alloc_node();
        let mut el = self.animate(None, id, el, window.now(), app.reduce_motion);
        let motion = self.pending_motion.take();
        if let Some(paint) = &el.paint {
            let (w, h) = styled_size(&el.layout);
            el.props = Rc::new(canvas::record(paint, el.layout.clone(), w, h, app));
        }
        if is_overlay(el.kind) {
            self.overlays.push(id);
        }
        self.tx.push(Op::Create { node: id, kind: el.kind, props: el.props.clone() });
        let style = self.taffy_style(&el.layout, &el.props, None);
        let taffy = if el.measured { self.taffy.new_leaf_with_context(style, id) } else { self.taffy.new_leaf(style) };
        self.nodes.insert(
            id,
            RNode {
                ident,
                props: el.props,
                layout: el.layout,
                measured: el.measured,
                slots: Vec::new(),
                native: Vec::new(),
                handlers: el.handlers,
                dispatch: el.dispatch,
                taffy: taffy.expect("taffy node"),
                frame: None,
                fresh: true,
                boundary: None,
                motion,
                paint: el.paint,
            },
        );
        let children: Vec<AnyElement> = el.children.into_iter().map(|c| c.resolve(window, app)).collect();
        let idents = idents(&children);
        let slots = children.into_iter().zip(idents).map(|(c, i)| self.mount(c, i, id, depth, window, app)).collect();
        self.nodes.get_mut(&id).expect("node").slots = slots;
        id
    }

    fn mount_view(
        &mut self,
        view: AnyView,
        ident: Ident,
        parent: NodeId,
        depth: u32,
        window: &mut Window,
        app: &mut App,
    ) -> Slot {
        let id = view.entity_id();
        assert!(self.placed.insert(id), "{id:?} is rendered in two places");
        if self.views.contains_key(&id) {
            self.adopt(id, ident, parent, depth);
            return Slot::View(id);
        }
        if let Some(&from) = app.view_windows.get(&id)
            && from != self.window
            && let Some(lifted) = app.lift_view(from, id)
        {
            self.implant(lifted, ident, parent, depth, app);
            return Slot::View(id);
        }
        app.view_windows.insert(id, self.window);
        self.views.insert(id, ViewSlot { view: view.clone(), ident, root: None, parent, depth });
        // Deferred elements in the view's subtree build while it mounts, so they see the view.
        app.rendering.push(id);
        let el = self.render_view(&view, window, app);
        let root_ident = idents(std::slice::from_ref(&el)).remove(0);
        let root = self.mount(el, root_ident, parent, depth + 1, window, app);
        app.rendering.pop();
        self.views.get_mut(&id).expect("view").root = Some(root);
        Slot::View(id)
    }

    /// Takes a mounted view to `parent`, keeping its native nodes: an orphan from earlier in
    /// this frame, or one still under a parent that is reconciled later in it, which is
    /// detached from there now.
    fn adopt(&mut self, id: EntityId, ident: Ident, parent: NodeId, depth: u32) {
        if let Some(i) = self.orphans.iter().position(|o| *o == id) {
            self.orphans.swap_remove(i);
        } else {
            let old = self.views[&id].parent;
            let slot = Slot::View(id);
            let held = self.nodes.get(&old).is_some_and(|n| n.slots.contains(&slot));
            // Not in its parent's slots: that parent is mid-reconcile and kept it this frame.
            assert!(held, "{id:?} is rendered in two places");
            self.nodes.get_mut(&old).expect("node").slots.retain(|s| *s != slot);
            self.sync_children(old);
        }
        let view = self.views.get_mut(&id).expect("view");
        let (old_depth, root) = (view.depth, view.root);
        view.ident = ident;
        view.parent = parent;
        view.depth = depth;
        if depth != old_depth
            && let Some(root) = root
        {
            self.shift_depth(root, depth as i64 - old_depth as i64);
        }
    }

    /// Moves the views under `slot` by `delta` levels, after the view above them moved.
    fn shift_depth(&mut self, slot: Slot, delta: i64) {
        match slot {
            Slot::Node(n) => {
                for s in self.nodes[&n].slots.clone() {
                    self.shift_depth(s, delta);
                }
            }
            Slot::View(e) => {
                let view = self.views.get_mut(&e).expect("view");
                view.depth = (view.depth as i64 + delta) as u32;
                if let Some(root) = view.root {
                    self.shift_depth(root, delta);
                }
            }
        }
    }

    /// Forgets a subtree. The backend gets a release per node; the remove of its top node comes
    /// from the parent's `sync_children`. Views inside it become orphans until the frame ends.
    fn unmount(&mut self, slot: Slot) {
        match slot {
            Slot::Node(n) => {
                let node = self.nodes.remove(&n).expect("node");
                let _ = self.taffy.remove(node.taffy);
                self.overlays.retain(|o| *o != n);
                self.animating.remove(&n);
                self.tx.push(Op::Release { node: n });
                for s in node.slots {
                    if let Slot::View(e) = s {
                        // Its parent is going, so its root is detached here rather than by a
                        // parent's sync.
                        if let Some(root) = self.native_of(s) {
                            self.tx.push(Op::Remove { node: root });
                            self.dispatch.detach(root);
                        }
                        self.orphans.push(e);
                    } else {
                        self.unmount(s);
                    }
                }
                if self.dispatch.contains(n) {
                    self.dispatch.remove(n);
                }
            }
            Slot::View(e) => self.orphans.push(e),
        }
    }

    /// Releases the views no parent adopted this frame, with everything under them.
    fn release_orphans(&mut self, app: &mut App) {
        while let Some(e) = self.orphans.pop() {
            let view = self.views.remove(&e).expect("view");
            app.view_windows.remove(&e);
            if let Some(root) = view.root {
                self.unmount(root);
            }
        }
    }

    fn flatten(&self, slots: &[Slot], out: &mut Vec<NodeId>) {
        for slot in slots {
            if let Some(n) = self.native_of(*slot) {
                out.push(n);
            }
        }
    }

    /// Brings the backend's children of `parent` in line with its slots: removes, then inserts
    /// and moves from the end so `before` is always in place. An unchanged list sends nothing.
    fn sync_children(&mut self, parent: NodeId) {
        let mut desired = Vec::new();
        self.flatten(&self.nodes[&parent].slots, &mut desired);
        let node = self.nodes.get_mut(&parent).expect("node");
        if node.native == desired {
            return;
        }
        let keep: HashSet<NodeId> = desired.iter().copied().collect();
        let mut list = mem::take(&mut node.native);
        for n in &list {
            if !keep.contains(n) {
                self.tx.push(Op::Remove { node: *n });
                self.dispatch.detach(*n);
            }
        }
        list.retain(|n| keep.contains(n));
        let mut before: Option<NodeId> = None;
        for &n in desired.iter().rev() {
            let at = |list: &Vec<NodeId>| before.and_then(|b| list.iter().position(|x| *x == b)).unwrap_or(list.len());
            match list.iter().position(|x| *x == n) {
                None => {
                    list.insert(at(&list), n);
                    self.tx.push(Op::Insert { parent, node: n, before });
                    let fresh = !self.dispatch.contains(n);
                    self.dispatch.place(parent, n, before);
                    if fresh {
                        self.bind_dispatch_subtree(n);
                    }
                }
                Some(i) if list.get(i + 1).copied() != before => {
                    list.remove(i);
                    list.insert(at(&list), n);
                    self.tx.push(Op::Move { node: n, parent, before });
                    self.dispatch.place(parent, n, before);
                }
                Some(_) => {}
            }
            before = Some(n);
        }
        let taffy_children: Vec<taffy::NodeId> = desired.iter().map(|n| self.nodes[n].taffy).collect();
        let node = self.nodes.get_mut(&parent).expect("node");
        self.taffy.set_children(node.taffy, &taffy_children).expect("taffy children");
        node.native = desired;
        self.needs_layout = true;
    }

    /// Inserts the children of nodes created this frame, top down, after their own insert.
    fn sync_fresh(&mut self, slot: Slot) {
        match slot {
            Slot::Node(n) => {
                let node = self.nodes.get_mut(&n).expect("node");
                if !node.fresh {
                    return;
                }
                node.fresh = false;
                let slots = node.slots.clone();
                self.sync_children(n);
                for s in slots {
                    self.sync_fresh(s);
                }
            }
            Slot::View(e) => {
                if let Some(root) = self.views[&e].root {
                    self.sync_fresh(root);
                }
            }
        }
    }

    fn reconcile_node(&mut self, n: NodeId, el: NodeElement, depth: u32, window: &mut Window, app: &mut App) {
        let mut el = self.animate(Some(n), n, el, window.now(), app.reduce_motion);
        let motion = self.pending_motion.take();
        if let Some(paint) = &el.paint {
            let node = &self.nodes[&n];
            let (w, h) = match node.props.downcast_ref::<CanvasProps>() {
                Some(p) if p.width > 0. || p.height > 0. => (p.width, p.height),
                _ => styled_size(&el.layout),
            };
            el.props = Rc::new(canvas::record(paint, el.layout.clone(), w, h, app));
        }
        let node = self.nodes.get_mut(&n).expect("node");
        node.motion = motion;
        node.paint = el.paint.clone();
        let mut restyle = false;
        if !node.props.props_eq(&*el.props) {
            restyle = pads(&node.props) || pads(&el.props);
            let patch = PropsPatch { old: mem::replace(&mut node.props, el.props), new: node.props.clone() };
            self.tx.push(Op::Update { node: n, patch });
            if node.measured {
                self.taffy.mark_dirty(node.taffy).expect("taffy node");
                self.needs_layout = true;
            }
        }
        if node.layout != el.layout || restyle {
            node.layout = el.layout;
            let node = &self.nodes[&n];
            let style = self.taffy_style(&node.layout, &node.props, node.boundary);
            self.taffy.set_style(self.nodes[&n].taffy, style).expect("taffy node");
            self.needs_layout = true;
        }
        let node = self.nodes.get_mut(&n).expect("node");
        node.handlers = el.handlers;
        node.dispatch = el.dispatch;
        self.bind_dispatch(n);
        self.reconcile_children(n, el.children, depth, window, app);
    }

    /// Binds `n` and the native nodes under it that the dispatch tree does not hold yet: a
    /// subtree adopted from another window arrives with its children in place.
    fn bind_dispatch_subtree(&mut self, n: NodeId) {
        self.bind_dispatch(n);
        for child in self.nodes[&n].native.clone() {
            if !self.dispatch.contains(child) {
                self.dispatch.place(n, child, None);
                self.bind_dispatch_subtree(child);
            }
        }
    }

    /// Hands the node's key context and action listeners to the dispatch tree, replacing the
    /// previous render's, without a backend call.
    fn bind_dispatch(&mut self, n: NodeId) {
        if !self.dispatch.contains(n) {
            return;
        }
        let props = &self.nodes[&n].dispatch;
        self.dispatch.set_key_context(n, props.key_context.clone());
        self.dispatch.set_focusable(n, props.focusable);
        self.dispatch.set_key_roles(n, props.roles);
        self.dispatch.set_focus_scope(n, props.focus_scope);
        self.dispatch.clear_listeners(n);
        for register in props.actions.clone() {
            register(&mut self.dispatch, n, self.window);
        }
    }

    fn reconcile_children(
        &mut self,
        parent: NodeId,
        children: Vec<AnyElement>,
        depth: u32,
        window: &mut Window,
        app: &mut App,
    ) {
        let children: Vec<AnyElement> = children.into_iter().map(|c| c.resolve(window, app)).collect();
        let old = mem::take(&mut self.nodes.get_mut(&parent).expect("node").slots);
        if old.is_empty() && children.is_empty() {
            return;
        }
        let mut by_ident: HashMap<Ident, Slot> = old.iter().map(|s| (self.ident_of(*s).clone(), *s)).collect();
        let idents = idents(&children);
        let mut matched = Vec::with_capacity(children.len());
        for (el, ident) in children.into_iter().zip(idents) {
            matched.push(match (by_ident.remove(&ident), el.0) {
                (Some(slot @ Slot::Node(_)), Element::Node(desc)) => (Ok(slot), Some(desc)),
                (Some(slot @ Slot::View(e)), Element::View(_)) => {
                    assert!(self.placed.insert(e), "{e:?} is rendered in two places");
                    (Ok(slot), None)
                }
                (_, el) => (Err((AnyElement(el), ident)), None),
            });
        }
        // Unmount before mounting, so a view that moved between keys is adopted, not rebuilt.
        let gone: HashSet<Slot> = by_ident.into_values().collect();
        for slot in old {
            if gone.contains(&slot) {
                self.unmount(slot);
            }
        }
        let mut slots = Vec::with_capacity(matched.len());
        let mut reused = Vec::with_capacity(matched.len());
        for (slot, desc) in matched {
            slots.push(match slot {
                Ok(slot) => slot,
                Err((el, ident)) => self.mount(el, ident, parent, depth, window, app),
            });
            reused.push(desc);
        }
        self.nodes.get_mut(&parent).expect("node").slots = slots.clone();
        self.sync_children(parent);
        for (slot, desc) in slots.into_iter().zip(reused) {
            match (slot, desc) {
                (Slot::Node(n), Some(desc)) => self.reconcile_node(n, *desc, depth, window, app),
                (slot, _) => self.sync_fresh(slot),
            }
        }
    }

    fn rerender_view(&mut self, id: EntityId, window: &mut Window, app: &mut App) {
        app.rendering.push(id);
        self.rerender_view_in(id, window, app);
        app.rendering.pop();
    }

    fn rerender_view_in(&mut self, id: EntityId, window: &mut Window, app: &mut App) {
        let (view, parent, depth, old) = {
            let v = &self.views[&id];
            (v.view.clone(), v.parent, v.depth, v.root)
        };
        let el = self.render_view(&view, window, app);
        let ident = idents(std::slice::from_ref(&el)).remove(0);
        if let Some(old) = old
            && *self.ident_of(old) == ident
        {
            if let (Slot::Node(n), Element::Node(desc)) = (old, el.0) {
                self.reconcile_node(n, *desc, depth + 1, window, app);
            }
            return;
        }
        let new = self.mount(el, ident, parent, depth + 1, window, app);
        self.views.get_mut(&id).expect("view").root = Some(new);
        if let Some(old) = old {
            self.unmount(old);
        }
        self.sync_children(parent);
        self.sync_fresh(new);
    }

    /// Takes a mounted view out of this window with everything under it, for another window
    /// to adopt. Its native nodes stay alive; a root still attached here is detached by the
    /// adopting window's transaction, so the backend never sees it in two places. `None` for the
    /// window's root view.
    pub(crate) fn lift(&mut self, id: EntityId, frame: u64) -> Option<Lifted> {
        if id == self.root_view.entity_id() {
            return None;
        }
        assert!(!(self.drawn_in == frame && self.placed.contains(&id)), "{id:?} is rendered in two windows");
        let view = self.views.get(&id)?;
        let (parent, root) = (view.parent, view.root.and_then(|r| self.native_of(r)));
        let mut attached = false;
        if let Some(i) = self.parked.iter().position(|e| *e == id) {
            self.parked.swap_remove(i);
        } else if let Some(i) = self.orphans.iter().position(|e| *e == id) {
            self.orphans.swap_remove(i);
        } else if self.nodes.get(&parent).is_some_and(|n| n.slots.contains(&Slot::View(id))) {
            self.nodes.get_mut(&parent).expect("node").slots.retain(|s| *s != Slot::View(id));
            self.sync_children(parent);
            attached = root.is_some_and(|root| self.tx.take_remove(root));
        }
        if let Some(root) = root
            && self.dispatch.contains(root)
        {
            self.dispatch.remove(root);
        }
        let mut lifted = Lifted { top: id, attached, views: Vec::new(), nodes: Vec::new(), overlays: Vec::new() };
        self.lift_slot(Slot::View(id), &mut lifted);
        self.needs_layout = true;
        Some(lifted)
    }

    fn lift_slot(&mut self, slot: Slot, out: &mut Lifted) {
        match slot {
            Slot::View(e) => {
                let view = self.views.remove(&e).expect("view");
                let root = view.root;
                out.views.push((e, view));
                if let Some(root) = root {
                    self.lift_slot(root, out);
                }
            }
            Slot::Node(n) => {
                let node = self.nodes.remove(&n).expect("node");
                let _ = self.taffy.remove(node.taffy);
                if let Some(i) = self.overlays.iter().position(|o| *o == n) {
                    self.overlays.remove(i);
                    out.overlays.push(n);
                }
                self.reaffirm.remove(&n);
                self.animating.remove(&n);
                let slots = node.slots.clone();
                out.nodes.push((n, node));
                for s in slots {
                    self.lift_slot(s, out);
                }
            }
        }
    }

    /// Mounts a view lifted from another window at `parent`, keeping its native nodes.
    fn implant(&mut self, lifted: Lifted, ident: Ident, parent: NodeId, depth: u32, app: &mut App) {
        let Lifted { top, attached, views, nodes, overlays } = lifted;
        let moved: Vec<NodeId> = nodes.iter().map(|(n, _)| *n).collect();
        for (n, mut node) in nodes {
            node.frame = None;
            node.boundary = None;
            node.fresh = false;
            let style = self.taffy_style(&node.layout, &node.props, None);
            let taffy =
                if node.measured { self.taffy.new_leaf_with_context(style, n) } else { self.taffy.new_leaf(style) };
            node.taffy = taffy.expect("taffy node");
            if node.motion.as_ref().is_some_and(|m| m.state.is_moving()) {
                self.animating.insert(n);
            }
            self.nodes.insert(n, node);
        }
        for n in moved {
            let children: Vec<taffy::NodeId> = self.nodes[&n].native.iter().map(|c| self.nodes[c].taffy).collect();
            self.taffy.set_children(self.nodes[&n].taffy, &children).expect("taffy children");
        }
        let delta = views.first().map_or(0, |(_, v)| depth as i64 - v.depth as i64);
        for (e, mut view) in views {
            if e == top {
                view.ident = ident.clone();
                view.parent = parent;
            }
            view.depth = (view.depth as i64 + delta) as u32;
            app.view_windows.insert(e, self.window);
            self.views.insert(e, view);
        }
        self.overlays.extend(overlays);
        if attached && let Some(root) = self.view_node(top) {
            self.tx.push(Op::Remove { node: root });
        }
        self.needs_layout = true;
    }

    /// Releases the views no window adopted this frame, in a transaction of their own.
    pub(crate) fn release_parked(&mut self, app: &mut App) {
        if self.parked.is_empty() {
            return;
        }
        self.orphans.append(&mut self.parked);
        self.release_orphans(app);
        let tx = mem::replace(&mut self.tx, Transaction::new(self.window));
        tx.commit(&mut *app.backend, &mut app.measure_cache);
    }

    fn layout(&mut self, backend: &mut dyn Backend, cache: &mut MeasureCache) -> Vec<Op> {
        if !self.needs_layout {
            return Vec::new();
        }
        self.needs_layout = false;
        // A backend that lays out itself (GPUI) would drop the frames; measuring for them is
        // wasted work every resize.
        if !backend.retains_frames() {
            return Vec::new();
        }
        // Children of native layout containers (split panes, toolbar items) take the size the
        // container gives them; Taffy lays out inside that box.
        let ids: Vec<NodeId> = self.nodes.keys().copied().collect();
        for id in ids {
            let boundary = match backend.boundary_size(id) {
                Some(size) => Some(Boundary::Box(size, backend.boundary_insets(id))),
                None => backend.boundary_width(id).map(Boundary::Width),
            };
            if boundary != self.nodes[&id].boundary {
                self.nodes.get_mut(&id).expect("node").boundary = boundary;
                let node = &self.nodes[&id];
                let style = self.taffy_style(&node.layout, &node.props, boundary);
                self.taffy.set_style(node.taffy, style).expect("taffy node");
            }
        }
        let root = self.nodes[&self.root].taffy;
        let space = |s: taffy::AvailableSpace| match s {
            taffy::AvailableSpace::Definite(v) => AvailableSpace::Definite(v),
            taffy::AvailableSpace::MinContent => AvailableSpace::MinContent,
            taffy::AvailableSpace::MaxContent => AvailableSpace::MaxContent,
        };
        nd_gpui::layout::compute_with_measure(
            &mut self.taffy,
            root,
            taffy::Size::MAX_CONTENT,
            |known, available, node| {
                let Some(node) = node else { return taffy::Size::ZERO };
                let constraint = Constraint {
                    width: known.width.map_or(space(available.width), AvailableSpace::Definite),
                    height: known.height.map_or(space(available.height), AvailableSpace::Definite),
                };
                let size = cache.measure(backend, *node, constraint);
                taffy::Size { width: known.width.unwrap_or(size.width), height: known.height.unwrap_or(size.height) }
            },
        )
        .expect("taffy layout");
        let mut ops = Vec::new();
        let mut stack = self.nodes[&self.root].native.clone();
        while let Some(n) = stack.pop() {
            let node = self.nodes.get_mut(&n).expect("node");
            let l = self.taffy.layout(node.taffy).expect("taffy layout");
            let frame = Rect::new(l.location.x, l.location.y, l.size.width, l.size.height);
            if node.frame != Some(frame) {
                node.frame = Some(frame);
                ops.push(Op::SetFrame { node: n, frame });
            }
            stack.extend(node.native.iter().copied());
        }
        ops
    }
}

impl App {
    /// One window's frame: mount the root view on the first, render dirty views shallowest
    /// first, lay out and commit one transaction.
    pub(crate) fn draw(&mut self, id: WindowId) {
        let slot = self.windows.get_mut(&id).expect("window");
        let (Some(mut window), Some(mut tree)) = (slot.window.take(), slot.tree.take()) else {
            panic!("{id:?} drawn while leased");
        };
        let outer = self.current_window.replace(id);
        tree.placed.clear();
        tree.rendered.clear();
        tree.drawn_in = self.frames;
        for node in tree.step_motion(window.now()) {
            if tree.handler(node, std::any::TypeId::of::<crate::AnimationEnded>()).is_some() {
                self.events().emitter(node).emit(crate::AnimationEnded);
            }
        }
        let root_view = tree.root_view.clone();
        tree.placed.insert(root_view.entity_id());
        if !tree.views.contains_key(&root_view.entity_id()) {
            let root = tree.root;
            let ident = Ident {
                kind: SlotKind::View(root_view.view_type()),
                key: Key::Entity(root_view.entity_id()),
                occurrence: 0,
            };
            tree.placed.remove(&root_view.entity_id());
            let slot = tree.mount_view(root_view, ident, root, 0, &mut window, self);
            tree.nodes.get_mut(&root).expect("root").slots = vec![slot];
            tree.sync_children(root);
            tree.sync_fresh(slot);
        }
        // Each view renders at most once per frame; one that notifies itself while rendering
        // stays dirty for the next frame instead of spinning here.
        loop {
            let mut dirty: Vec<(u32, EntityId)> = self
                .dirty
                .iter()
                .filter(|e| !tree.rendered.contains(*e) && !tree.orphans.contains(e))
                .filter_map(|e| tree.views.get(e).map(|v| (v.depth, *e)))
                .collect();
            if dirty.is_empty() {
                break;
            }
            dirty.sort();
            for (_, e) in dirty {
                // A parent rendered earlier in this pass may have unmounted it or mounted it fresh.
                let live = tree.views.contains_key(&e) && !tree.orphans.contains(&e);
                if self.dirty.contains(&e) && live && !tree.rendered.contains(&e) {
                    tree.rerender_view(e, &mut window, self);
                }
            }
        }
        if self.windows.len() > 1 {
            tree.parked.append(&mut tree.orphans);
        } else {
            tree.release_orphans(self);
        }
        tree.dispatch.settle_scopes();
        if let Some(node) = window.focus_request.take().and_then(|id| tree.focus_target(&id)) {
            tree.dispatch.focus(node);
        }
        tree.push_focus_change();
        let mut reaffirm: Vec<NodeId> = tree.reaffirm.drain().filter(|n| tree.nodes.contains_key(n)).collect();
        reaffirm.sort();
        for node in reaffirm {
            tree.tx.push(Op::Reaffirm { node });
        }
        let target = tree.view_node(tree.root_view.entity_id());
        tree.dispatch.set_default_target(target);
        let tx = mem::replace(&mut tree.tx, Transaction::new(id));
        let force = tree.needs_layout;
        tx.commit_with(&mut *self.backend, &mut self.measure_cache, force, |backend, cache| {
            tree.layout(backend, cache)
        });
        // A retained backend's frames are known now; canvases laid out at a new size paint
        // again, and the next frame sends what they painted.
        for (node, width, height) in tree.stale_canvases() {
            tree.repaint_canvas(node, width, height, self);
        }
        self.current_window = outer;
        let slot = self.windows.get_mut(&id).expect("window");
        slot.window = Some(window);
        slot.tree = Some(tree);
    }
}
