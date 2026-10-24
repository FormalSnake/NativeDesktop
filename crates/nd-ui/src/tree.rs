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
use nd_gpui::gpui::{KeyBinding, Refineable, StyleRefinement};
use nd_gpui::taffy::{self, TaffyTree, prelude::TaffyMaxContent};

use crate::animation::{self, MotionSpec, MotionState, MotionTokens, Transform, Values};
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

/// Frames a mounting hero waits for its own frame before it shows in place instead.
const HERO_WAIT_FRAMES: u32 = 4;

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
    drag: crate::drag::DragSpec,
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

/// A div's props and layout as it shows now: its group hover style while a group lights it,
/// and on its source's frame while a hero goes back.
fn shown(
    lit: &HashSet<NodeId>,
    away: Option<&Values>,
    id: NodeId,
    props: &Props,
    layout: &StyleRefinement,
) -> (Props, StyleRefinement) {
    let Some(div) = props.downcast_ref::<DivProps>() else { return (props.clone(), layout.clone()) };
    let hover = div.group_hover.as_ref().filter(|_| lit.contains(&id)).map(|(_, s)| s);
    if hover.is_none() && away.is_none() {
        return (props.clone(), layout.clone());
    }
    let mut div = div.clone();
    let mut layout = layout.clone();
    if let Some(style) = hover {
        div.style.refine(style);
        layout.refine(style);
    }
    if let Some(values) = away {
        animation::present(&mut div.style, &mut div.transform, values);
        animation::present(&mut layout, &mut Transform::default(), values);
    }
    (Rc::new(div), layout)
}

/// A div's props drawn fully transparent, its layout untouched.
fn hidden(props: &Props) -> Props {
    let mut div = props.downcast_ref::<DivProps>().expect("div").clone();
    div.style.opacity = Some(0.);
    Rc::new(div)
}

/// The values that put an element whose frame at rest is `home` (with `targets` there) onto
/// `source`: its size when given in points, else a scale about its centre.
fn pose_on(targets: &Values, source: Rect, home: Rect) -> Values {
    let mut out: Values = [None; 6];
    let (ox, oy) = (targets[1].unwrap_or(0.), targets[2].unwrap_or(0.));
    if targets[4].is_some() && targets[5].is_some() {
        out[1] = Some(ox + f64::from(source.origin.x - home.origin.x));
        out[2] = Some(oy + f64::from(source.origin.y - home.origin.y));
        out[4] = Some(f64::from(source.size.width));
        out[5] = Some(f64::from(source.size.height));
    } else {
        let centre = |r: Rect| (r.origin.x + r.size.width / 2., r.origin.y + r.size.height / 2.);
        let (s, h) = (centre(source), centre(home));
        out[1] = Some(ox + f64::from(s.0 - h.0));
        out[2] = Some(oy + f64::from(s.1 - h.1));
        out[3] = Some(f64::from(source.size.width / home.size.width));
    }
    out
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
    /// Hover groups the pointer is over, and the nodes under them that take their
    /// `group_hover` style.
    hovered_groups: HashSet<NodeId>,
    lit: HashSet<NodeId>,
    /// Heroes mounted this frame, hidden until their frame is known and they can start from
    /// their source's.
    heroes_pending: Vec<NodeId>,
    heroes_still_pending: Vec<NodeId>,
    /// Frames each pending hero has waited for its own frame.
    hero_waits: HashMap<NodeId, u32>,
    /// Heroes asked to go back whose destination is not worked out yet, and the values that
    /// put the ones going back on their source's frame.
    away_pending: Vec<NodeId>,
    hero_away: HashMap<NodeId, Values>,
    /// Heroes that went back with reduce motion: hidden from the frame they go, already landed.
    away_done: HashSet<NodeId>,
    /// A hero's frame at rest, for going back from mid-flight.
    hero_home: HashMap<NodeId, Rect>,
    /// Nodes that report their size, laid out at a new one this frame.
    resized: Vec<(NodeId, Size)>,
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
                drag: crate::drag::DragSpec::default(),
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
            hovered_groups: HashSet::new(),
            lit: HashSet::new(),
            heroes_pending: Vec::new(),
            heroes_still_pending: Vec::new(),
            hero_waits: HashMap::new(),
            away_pending: Vec::new(),
            hero_away: HashMap::new(),
            away_done: HashSet::new(),
            hero_home: HashMap::new(),
            resized: Vec::new(),
        }
    }

    pub(crate) fn contains(&self, node: NodeId) -> bool {
        self.nodes.contains_key(&node)
    }

    /// Nodes that take drags of `kind`, in no particular order.
    pub(crate) fn drop_targets(&self, kind: &str) -> Vec<NodeId> {
        self.nodes.iter().filter(|(_, n)| n.drag.accepts.iter().any(|k| k == kind)).map(|(id, _)| *id).collect()
    }

    pub(crate) fn drag_item(&self, node: NodeId) -> Option<crate::DragItem> {
        self.nodes.get(&node)?.drag.item.clone()
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
    /// The node keyed `id` and its laid-out frame, when it has one.
    pub(crate) fn keyed(&self, id: &str) -> Option<(NodeId, Option<nd_core::Rect>)> {
        let mut stack = vec![self.root];
        while let Some(n) = stack.pop() {
            let node = &self.nodes[&n];
            if matches!(&node.ident.key, Key::Id(k) if k.as_ref() == id) {
                return Some((n, node.frame));
            }
            stack.extend(node.native.iter().rev().copied());
        }
        None
    }

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
        self.needs_layout
            || !self.tx.is_empty()
            || !self.reaffirm.is_empty()
            || !self.animating.is_empty()
            || !self.heroes_pending.is_empty()
            || !self.away_pending.is_empty()
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
        tokens: &MotionTokens,
    ) -> NodeElement {
        let previous = node.and_then(|n| self.nodes.get_mut(&n)).and_then(|n| n.motion.take());
        let (Some(mut spec), Some(div)) = (el.motion, el.props.downcast_ref::<DivProps>()) else {
            self.animating.remove(&id);
            return el;
        };
        spec.animation = tokens.resolve(spec.animation, reduce);
        let hero = div.hero.clone();
        let away = hero.as_ref().is_some_and(|h| h.away);
        if !away {
            self.hero_away.remove(&id);
            self.away_done.remove(&id);
            self.away_pending.retain(|n| *n != id);
        } else if node.is_some()
            && !self.hero_away.contains_key(&id)
            && !self.away_done.contains(&id)
            && !self.away_pending.contains(&id)
        {
            self.away_pending.push(id);
            if reduce {
                self.away_done.insert(id);
            }
        }
        let (shown_props, shown_layout) = shown(&self.lit, self.hero_away.get(&id), id, &el.props, &el.layout);
        let transform = shown_props.downcast_ref::<DivProps>().map_or_else(Transform::default, |d| d.transform);
        let targets = animation::values(&shown_layout, &transform);
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
            (el.props, el.layout) = present_div(&shown_props, &shown_layout, &state.sample(now));
            self.animating.insert(id);
        } else {
            (el.props, el.layout) = (shown_props, shown_layout);
            self.animating.remove(&id);
        }
        // A hero mounting shows nothing until its frame is known; then it starts on its source.
        if node.is_none() && hero.is_some_and(|h| !h.away) && !reduce {
            el.props = hidden(&el.props);
            self.heroes_pending.push(id);
        }
        if self.away_done.contains(&id) {
            el.props = hidden(&el.props);
        }
        self.pending_motion = Some(Animated { state, props: base.0, layout: base.1 });
        el
    }

    /// The pointer entered or left `node`. A hover group lights or dims the nodes under it.
    pub(crate) fn hover_changed(&mut self, node: NodeId, hovered: bool, now: f64, reduce: bool) {
        let is_group =
            self.nodes.get(&node).and_then(|n| n.props.downcast_ref::<DivProps>()).is_some_and(|d| d.group.is_some());
        if !is_group {
            return;
        }
        if hovered {
            self.hovered_groups.insert(node);
        } else {
            self.hovered_groups.remove(&node);
        }
        self.relight(now, reduce);
    }

    /// Recomputes which nodes hovered groups light, and moves the ones that changed.
    fn relight(&mut self, now: f64, reduce: bool) {
        self.hovered_groups.retain(|g| self.nodes.contains_key(g));
        let mut lit = HashSet::new();
        for &group in &self.hovered_groups {
            let Some(name) = self.nodes[&group].props.downcast_ref::<DivProps>().and_then(|d| d.group.clone()) else {
                continue;
            };
            let mut stack = self.nodes[&group].native.clone();
            while let Some(n) = stack.pop() {
                let node = &self.nodes[&n];
                if node
                    .props
                    .downcast_ref::<DivProps>()
                    .and_then(|d| d.group_hover.as_ref())
                    .is_some_and(|(g, _)| *g == name)
                {
                    lit.insert(n);
                }
                stack.extend(node.native.iter().copied());
            }
        }
        let changed: Vec<NodeId> = lit.symmetric_difference(&self.lit).copied().collect();
        self.lit = lit;
        for n in changed {
            self.retarget_shown(n, now, reduce);
        }
    }

    /// Starts node `n` towards what it shows now (its group hover and hero state applied).
    fn retarget_shown(&mut self, n: NodeId, now: f64, reduce: bool) {
        let away = self.hero_away.get(&n).copied();
        let Some(node) = self.nodes.get_mut(&n) else { return };
        let Some(anim) = node.motion.as_mut() else { return };
        let (props, layout) = shown(&self.lit, away.as_ref(), n, &anim.props, &anim.layout);
        let transform = props.downcast_ref::<DivProps>().map_or_else(Transform::default, |d| d.transform);
        let animation = anim.state.animation();
        anim.state.retarget(animation, animation::values(&layout, &transform), now, reduce);
        self.animating.insert(n);
    }

    /// Works out where heroes come from and go back to, from the frames the backend drew last.
    /// Returns the heroes going back that have nowhere to go: they are already done.
    pub(crate) fn place_heroes(
        &mut self,
        backend: &dyn Backend,
        now: f64,
        reduce: bool,
        tokens: &MotionTokens,
    ) -> Vec<NodeId> {
        let mut stranded = Vec::new();
        for id in mem::take(&mut self.heroes_pending) {
            let Some(hero) = self.hero_of(id) else {
                self.hero_waits.remove(&id);
                continue;
            };
            let source = self.source_rect(backend, &hero);
            let home = backend.probe(id).bounds.filter(|r| r.size.width > 0. && r.size.height > 0.);
            // A backend that lays out as it paints (GPUI) knows the frame a frame later: the hero
            // stays hidden until then rather than appearing in place.
            if source.is_some() && home.is_none() {
                let tries = self.hero_waits.entry(id).or_insert(0);
                *tries += 1;
                if *tries <= HERO_WAIT_FRAMES {
                    self.heroes_still_pending.push(id);
                    continue;
                }
            }
            self.hero_waits.remove(&id);
            let Some(node) = self.nodes.get_mut(&id) else { continue };
            let Some(anim) = node.motion.as_mut() else { continue };
            if let (Some(source), Some(home)) = (source, home) {
                self.hero_home.insert(id, home);
                let from = pose_on(&anim.state.targets(), source, home);
                anim.state.start_from(&from, now, reduce);
            }
            // Shown again: moving from its source, or in place when there is none.
            self.animating.insert(id);
        }
        self.heroes_pending.append(&mut self.heroes_still_pending);
        for id in mem::take(&mut self.away_pending) {
            let Some(hero) = self.hero_of(id).filter(|h| h.away) else { continue };
            // Reduce motion leaves out the travel, so the view around it waits on nothing.
            if reduce {
                self.away_done.insert(id);
                stranded.push(id);
                continue;
            }
            let Some(source) = self.source_rect(backend, &hero) else {
                stranded.push(id);
                continue;
            };
            let moving = self.nodes.get(&id).and_then(|n| n.motion.as_ref()).is_some_and(|m| m.state.is_moving());
            let probed = backend.probe(id).bounds.filter(|r| r.size.width > 0. && r.size.height > 0.);
            let home = if moving { self.hero_home.get(&id).copied().or(probed) } else { probed };
            let Some(home) = home else {
                stranded.push(id);
                continue;
            };
            let Some(anim) = self.nodes.get(&id).and_then(|n| n.motion.as_ref()) else { continue };
            let mut base = anim.props.downcast_ref::<DivProps>().map_or_else(Transform::default, |d| d.transform);
            base.scale = 1.;
            let targets = animation::values(&anim.layout, &base);
            self.hero_away.insert(id, pose_on(&targets, source, home));
            // From rest it waits for the view around it to fade, then travels home.
            let at = if moving { now } else { now + tokens.hero_hold(reduce) };
            self.retarget_shown(id, at, reduce);
        }
        stranded
    }

    fn hero_of(&self, id: NodeId) -> Option<crate::element::HeroFrom> {
        self.nodes.get(&id)?.props.downcast_ref::<DivProps>()?.hero.clone()
    }

    fn source_rect(&self, backend: &dyn Backend, hero: &crate::element::HeroFrom) -> Option<Rect> {
        let (node, _) = self.keyed(&hero.id)?;
        backend.probe(node).bounds.filter(|r| r.size.width > 0. && r.size.height > 0.)
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
            let (shown_props, shown_layout) = shown(&self.lit, self.hero_away.get(&id), id, &anim.props, &anim.layout);
            let (mut props, layout) = present_div(&shown_props, &shown_layout, &anim.state.sample(now));
            if self.away_done.contains(&id) {
                props = hidden(&props);
            }
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
        let mut el = self.animate(None, id, el, window.now(), app.reduce_motion, &app.motion);
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
                drag: el.drag,
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
                self.lit.remove(&n);
                self.hovered_groups.remove(&n);
                self.hero_away.remove(&n);
                self.away_done.remove(&n);
                self.hero_home.remove(&n);
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
        let mut el = self.animate(Some(n), n, el, window.now(), app.reduce_motion, &app.motion);
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
        node.drag = el.drag;
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
                if node.frame.is_none_or(|f| f.size != frame.size)
                    && node.props.downcast_ref::<DivProps>().is_some_and(|p| p.reports_size)
                {
                    self.resized.push((n, frame.size));
                }
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
        let stranded = tree.place_heroes(&*self.backend, window.now(), self.reduce_motion, &self.motion);
        for node in stranded.into_iter().chain(tree.step_motion(window.now())) {
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
        // Nodes mounted under a hovered group this frame take its hover style too.
        if !tree.hovered_groups.is_empty() {
            tree.relight(window.now(), self.reduce_motion);
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
        for (node, size) in tree.resized.drain(..) {
            self.events().emitter(node).emit(crate::Resized { width: size.width, height: size.height });
        }
        self.current_window = outer;
        let slot = self.windows.get_mut(&id).expect("window");
        slot.window = Some(window);
        slot.tree = Some(tree);
    }
}
