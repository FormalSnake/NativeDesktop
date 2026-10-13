//! The retained tree and the reconciler (ARCHITECTURE.md 3.7, 3.14).
//!
//! One `Tree` per window. A frame re-renders only the views notified since the last one, top
//! down, and reconciles each view's output against its own retained subtree: children match by
//! identity, props compare with `PartialEq`, handlers swap without a backend call. A view's
//! output is spliced into its parent's native children, so views add no native node or layout
//! box of their own. Ops go into one transaction per window per frame, with layout inside it.

mod snapshot;

use std::any::TypeId;
use std::collections::{HashMap, HashSet};
use std::mem;
use std::rc::Rc;

use nd_core::{NodeId, Rect, Size, WindowId};
use nd_gpui::gpui::{KeyBinding, StyleRefinement};
use taffy::TaffyTree;
use taffy::prelude::TaffyMaxContent;

use crate::app::{AnyView, App, EntityId};
use crate::backend::{AvailableSpace, Backend, Constraint, NodeKind, Props, PropsPatch};
use crate::dispatch::DispatchTree;
use crate::element::{AnyElement, DispatchProps, Element, Handler, Key, NodeElement, RootProps};
use crate::layout::to_taffy;
use crate::transaction::{MeasureCache, Op, Transaction};
use crate::window::Window;

pub use snapshot::TreeNode;

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

pub(crate) struct Tree {
    window: WindowId,
    root: NodeId,
    root_view: AnyView,
    nodes: HashMap<NodeId, RNode>,
    views: HashMap<EntityId, ViewSlot>,
    taffy: TaffyTree<NodeId>,
    tx: Transaction,
    needs_layout: bool,
    /// Mirrors the native nodes as the backend has them, with each node's key context and
    /// action listeners.
    pub(crate) dispatch: DispatchTree<App>,
}

fn idents(elements: &[AnyElement]) -> Vec<Ident> {
    let mut seen: HashMap<(SlotKind, Key), u32> = HashMap::new();
    elements
        .iter()
        .map(|el| {
            let (kind, key) = match &el.0 {
                Element::Node(n) => (SlotKind::Node(n.kind), n.key.clone()),
                Element::View(v) => (SlotKind::View(v.view_type()), Key::Entity(v.entity_id())),
            };
            let count = seen.entry((kind, key.clone())).or_default();
            let occurrence = *count;
            *count += 1;
            Ident { kind, key, occurrence }
        })
        .collect()
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
            },
        );
        let mut dispatch = DispatchTree::new(bindings);
        dispatch.set_root(root);
        Self { window, root, root_view, nodes, views: HashMap::new(), taffy, tx, needs_layout: true, dispatch }
    }

    pub(crate) fn contains(&self, node: NodeId) -> bool {
        self.nodes.contains_key(&node)
    }

    pub(crate) fn handler(&self, node: NodeId, event: TypeId) -> Option<Handler> {
        self.nodes.get(&node)?.handlers.iter().find(|(t, _)| *t == event).map(|(_, h)| h.clone())
    }

    pub(crate) fn needs_frame(&self) -> bool {
        self.needs_layout || !self.tx.is_empty()
    }

    pub(crate) fn view_node(&self, view: EntityId) -> Option<NodeId> {
        self.native_of(self.views.get(&view)?.root?)
    }

    pub(crate) fn resize(&mut self, size: Size) {
        let root = self.nodes[&self.root].taffy;
        self.taffy.set_style(root, root_style(size)).expect("taffy root");
        self.needs_layout = true;
    }

    /// Releases every node in one last transaction.
    pub(crate) fn close(mut self, app: &mut App) {
        let root = self.root;
        let slots = mem::take(&mut self.nodes.get_mut(&root).expect("root").slots);
        for slot in slots {
            self.unmount(slot, app);
        }
        self.sync_children(root);
        self.tx.push(Op::Release { node: root });
        let tx = mem::replace(&mut self.tx, Transaction::new(self.window));
        tx.commit(&mut *app.backend, &mut app.measure_cache);
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
        *app.render_counts.entry(id).or_default() += 1;
        view.render(window, app)
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
        }
    }

    fn mount_node(&mut self, el: NodeElement, ident: Ident, depth: u32, window: &mut Window, app: &mut App) -> NodeId {
        let id = app.alloc_node();
        self.tx.push(Op::Create { node: id, kind: el.kind, props: el.props.clone() });
        let style = to_taffy(&el.layout);
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
            },
        );
        let idents = idents(&el.children);
        let slots =
            el.children.into_iter().zip(idents).map(|(c, i)| self.mount(c, i, id, depth, window, app)).collect();
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
        assert!(!self.views.contains_key(&id), "{id:?} is rendered in two places");
        app.view_windows.insert(id, self.window);
        self.views.insert(id, ViewSlot { view: view.clone(), ident, root: None, parent, depth });
        let el = self.render_view(&view, window, app);
        let root_ident = idents(std::slice::from_ref(&el)).remove(0);
        let root = self.mount(el, root_ident, parent, depth + 1, window, app);
        self.views.get_mut(&id).expect("view").root = Some(root);
        Slot::View(id)
    }

    /// Forgets a subtree. The backend gets a release per node; the remove of its top node comes
    /// from the parent's `sync_children`.
    fn unmount(&mut self, slot: Slot, app: &mut App) {
        match slot {
            Slot::Node(n) => {
                let node = self.nodes.remove(&n).expect("node");
                let _ = self.taffy.remove(node.taffy);
                if self.dispatch.contains(n) {
                    self.dispatch.remove(n);
                }
                self.tx.push(Op::Release { node: n });
                for s in node.slots {
                    self.unmount(s, app);
                }
            }
            Slot::View(e) => {
                let view = self.views.remove(&e).expect("view");
                app.view_windows.remove(&e);
                if let Some(root) = view.root {
                    self.unmount(root, app);
                }
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
                        self.bind_dispatch(n);
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
        let node = self.nodes.get_mut(&n).expect("node");
        if !node.props.props_eq(&*el.props) {
            let patch = PropsPatch { old: mem::replace(&mut node.props, el.props), new: node.props.clone() };
            self.tx.push(Op::Update { node: n, patch });
            if node.measured {
                self.taffy.mark_dirty(node.taffy).expect("taffy node");
                self.needs_layout = true;
            }
        }
        if node.layout != el.layout {
            self.taffy.set_style(node.taffy, to_taffy(&el.layout)).expect("taffy node");
            node.layout = el.layout;
            self.needs_layout = true;
        }
        node.handlers = el.handlers;
        node.dispatch = el.dispatch;
        self.bind_dispatch(n);
        self.reconcile_children(n, el.children, depth, window, app);
    }

    /// Hands the node's key context and action listeners to the dispatch tree, replacing the
    /// previous render's, without a backend call.
    fn bind_dispatch(&mut self, n: NodeId) {
        if !self.dispatch.contains(n) {
            return;
        }
        let props = &self.nodes[&n].dispatch;
        self.dispatch.set_key_context(n, props.key_context.clone());
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
                (Some(slot @ Slot::View(_)), Element::View(_)) => (Ok(slot), None),
                (_, el) => (Err((AnyElement(el), ident)), None),
            });
        }
        // Unmount before mounting, so a view that moved between keys is free to mount again.
        let gone: HashSet<Slot> = by_ident.into_values().collect();
        for slot in old {
            if gone.contains(&slot) {
                self.unmount(slot, app);
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
            self.unmount(old, app);
        }
        self.sync_children(parent);
        self.sync_fresh(new);
    }

    fn layout(&mut self, backend: &mut dyn Backend, cache: &mut MeasureCache) -> Vec<Op> {
        if !self.needs_layout {
            return Vec::new();
        }
        self.needs_layout = false;
        let root = self.nodes[&self.root].taffy;
        let space = |s: taffy::AvailableSpace| match s {
            taffy::AvailableSpace::Definite(v) => AvailableSpace::Definite(v),
            taffy::AvailableSpace::MinContent => AvailableSpace::MinContent,
            taffy::AvailableSpace::MaxContent => AvailableSpace::MaxContent,
        };
        self.taffy
            .compute_layout_with_measure(root, taffy::Size::MAX_CONTENT, |known, available, _, node, _| {
                let Some(node) = node else { return taffy::Size::ZERO };
                let constraint = Constraint {
                    width: known.width.map_or(space(available.width), AvailableSpace::Definite),
                    height: known.height.map_or(space(available.height), AvailableSpace::Definite),
                };
                let size = cache.measure(backend, *node, constraint);
                taffy::Size { width: known.width.unwrap_or(size.width), height: known.height.unwrap_or(size.height) }
            })
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
        let root_view = tree.root_view.clone();
        if !tree.views.contains_key(&root_view.entity_id()) {
            let root = tree.root;
            let ident = Ident {
                kind: SlotKind::View(root_view.view_type()),
                key: Key::Entity(root_view.entity_id()),
                occurrence: 0,
            };
            let slot = tree.mount_view(root_view, ident, root, 0, &mut window, self);
            tree.nodes.get_mut(&root).expect("root").slots = vec![slot];
            tree.sync_children(root);
            tree.sync_fresh(slot);
        }
        // Each view renders at most once per frame; one that notifies itself while rendering
        // stays dirty for the next frame instead of spinning here.
        let mut rendered = HashSet::new();
        loop {
            let mut dirty: Vec<(u32, EntityId)> = self
                .dirty
                .iter()
                .filter(|e| !rendered.contains(*e))
                .filter_map(|e| tree.views.get(e).map(|v| (v.depth, *e)))
                .collect();
            if dirty.is_empty() {
                break;
            }
            dirty.sort();
            for (_, e) in dirty {
                // A parent rendered earlier in this pass may have unmounted it or mounted it fresh.
                if self.dirty.contains(&e) && tree.views.contains_key(&e) && rendered.insert(e) {
                    tree.rerender_view(e, &mut window, self);
                }
            }
        }
        let target = tree.view_node(tree.root_view.entity_id());
        tree.dispatch.set_default_target(target);
        let tx = mem::replace(&mut tree.tx, Transaction::new(id));
        let force = tree.needs_layout;
        tx.commit_with(&mut *self.backend, &mut self.measure_cache, force, |backend, cache| {
            tree.layout(backend, cache)
        });
        let slot = self.windows.get_mut(&id).expect("window");
        slot.window = Some(window);
        slot.tree = Some(tree);
    }
}
