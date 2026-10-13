//! nd-owned focus, key contexts, keystroke matching and action dispatch (ARCHITECTURE.md 3.12).
//!
//! One [`DispatchTree`] per window mirrors the retained tree's structure plus the dispatch
//! properties of each node (`key_context`, `on_action` listeners, focusability). GPUI's
//! [`Keymap`] is only the matcher: binding parsing, context predicates and multi-stroke
//! pending state. Every backend feeds raw keystrokes in through [`DispatchTree::dispatch_key`]
//! before the platform's text handling, and mirrors focus from [`DispatchTree::take_focus_change`].
//!
//! `C` is whatever listeners need to mutate (the app context once `nd-ui` core lands).

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::rc::Rc;

use nd_core::NodeId;
use nd_gpui::gpui::{Action, KeyBinding, KeyContext, Keymap, Keystroke};

use crate::actions::{FocusNext, FocusPrevious};

type Listener<C> = Rc<dyn Fn(&dyn Action, &mut ActionCx<'_, C>)>;

/// What happened to a keystroke.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyDispatch {
    /// A binding matched and a listener handled its action. The platform drops the key.
    Handled,
    /// The keystroke is a prefix of a multi-stroke binding. The platform drops the key.
    Pending,
    /// No binding took it. The platform delivers it to the focused native view for text.
    Unhandled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FocusRequest {
    Node(NodeId),
    Next,
    Previous,
    Clear,
}

/// Passed to action listeners. Focus requests are applied after the listener chain returns,
/// so a listener never re-enters the tree.
pub struct ActionCx<'a, C> {
    pub cx: &'a mut C,
    propagate: bool,
    focus: &'a mut Option<FocusRequest>,
}

impl<C> ActionCx<'_, C> {
    /// Lets the action continue to the next listener towards the root.
    pub fn propagate(&mut self) {
        self.propagate = true;
    }

    pub fn focus(&mut self, node: NodeId) {
        *self.focus = Some(FocusRequest::Node(node));
    }

    pub fn focus_next(&mut self) {
        *self.focus = Some(FocusRequest::Next);
    }

    pub fn focus_previous(&mut self) {
        *self.focus = Some(FocusRequest::Previous);
    }

    pub fn blur(&mut self) {
        *self.focus = Some(FocusRequest::Clear);
    }
}

/// How a node takes part in focus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Focusable {
    /// Orders tab stops before tree order; equal indexes keep tree order (GPUI's semantics).
    pub tab_index: isize,
    /// False keeps the node focusable by click or `focus()` but out of tab traversal.
    pub tab_stop: bool,
}

impl Default for Focusable {
    fn default() -> Self {
        Self { tab_index: 0, tab_stop: true }
    }
}

struct Node<C> {
    parent: Option<NodeId>,
    children: Vec<NodeId>,
    key_context: Option<KeyContext>,
    listeners: Vec<(TypeId, Listener<C>)>,
    focusable: Option<Focusable>,
}

impl<C> Node<C> {
    fn new(parent: Option<NodeId>) -> Self {
        Self { parent, children: Vec::new(), key_context: None, listeners: Vec::new(), focusable: None }
    }
}

pub struct DispatchTree<C> {
    nodes: HashMap<NodeId, Node<C>>,
    root: Option<NodeId>,
    keymap: Keymap,
    focused: Option<NodeId>,
    /// Where dispatch starts while nothing is focused; the root when unset. The retained tree
    /// points it at the root view's node, as GPUI dispatches from the root view's element.
    default_target: Option<NodeId>,
    /// Set when nd focus changed and the platform has not been told yet.
    focus_change: Option<Option<NodeId>>,
    pending: Vec<Keystroke>,
}

impl<C: 'static> DispatchTree<C> {
    /// A tree with [`crate::actions::standard_bindings`] followed by `bindings`.
    pub fn new(bindings: impl IntoIterator<Item = KeyBinding>) -> Self {
        let mut keymap = Keymap::new(crate::actions::standard_bindings());
        keymap.add_bindings(bindings);
        Self {
            nodes: HashMap::new(),
            root: None,
            keymap,
            focused: None,
            default_target: None,
            focus_change: None,
            pending: Vec::new(),
        }
    }

    pub fn keymap_mut(&mut self) -> &mut Keymap {
        &mut self.keymap
    }

    pub fn set_root(&mut self, root: NodeId) {
        assert!(self.root.is_none(), "dispatch tree already has a root");
        self.nodes.insert(root, Node::new(None));
        self.root = Some(root);
    }

    /// Adds `node` under `parent`, before `before` or last.
    pub fn insert(&mut self, parent: NodeId, node: NodeId, before: Option<NodeId>) {
        assert!(!self.nodes.contains_key(&node), "{node:?} is already in the dispatch tree");
        let Some(p) = self.nodes.get_mut(&parent) else {
            panic!("parent {parent:?} is not in the dispatch tree");
        };
        let at = before.and_then(|b| p.children.iter().position(|c| *c == b)).unwrap_or(p.children.len());
        p.children.insert(at, node);
        self.nodes.insert(node, Node::new(Some(parent)));
    }

    /// Puts `node` under `parent`, before `before` or last. A node already in the tree moves
    /// with its subtree, listeners and context; an unknown one is added.
    pub fn place(&mut self, parent: NodeId, node: NodeId, before: Option<NodeId>) {
        if !self.nodes.contains_key(&node) {
            self.insert(parent, node, before);
            return;
        }
        self.detach(node);
        let Some(p) = self.nodes.get_mut(&parent) else {
            panic!("parent {parent:?} is not in the dispatch tree");
        };
        let at = before.and_then(|b| p.children.iter().position(|c| *c == b)).unwrap_or(p.children.len());
        p.children.insert(at, node);
        self.node_mut(node).parent = Some(parent);
    }

    /// Takes `node` out of its parent's children, keeping its subtree, until it is placed again
    /// or removed.
    pub fn detach(&mut self, node: NodeId) {
        let Some(parent) = self.nodes.get_mut(&node).and_then(|n| n.parent.take()) else { return };
        if let Some(p) = self.nodes.get_mut(&parent) {
            p.children.retain(|c| *c != node);
        }
    }

    /// Where dispatch starts while nothing is focused.
    pub fn set_default_target(&mut self, node: Option<NodeId>) {
        self.default_target = node;
    }

    /// Removes `node` and its subtree. Focus inside it is cleared.
    pub fn remove(&mut self, node: NodeId) {
        if let Some(parent) = self.nodes.get(&node).and_then(|n| n.parent)
            && let Some(p) = self.nodes.get_mut(&parent)
        {
            p.children.retain(|c| *c != node);
        }
        let mut stack = vec![node];
        while let Some(n) = stack.pop() {
            if let Some(removed) = self.nodes.remove(&n) {
                stack.extend(removed.children);
            }
            if self.focused == Some(n) {
                self.set_focus(None);
            }
            if self.default_target == Some(n) {
                self.default_target = None;
            }
        }
    }

    pub fn contains(&self, node: NodeId) -> bool {
        self.nodes.contains_key(&node)
    }

    pub fn set_key_context(&mut self, node: NodeId, context: Option<KeyContext>) {
        self.node_mut(node).key_context = context;
    }

    pub fn set_focusable(&mut self, node: NodeId, focusable: Option<Focusable>) {
        self.node_mut(node).focusable = focusable;
        if focusable.is_none() && self.focused == Some(node) {
            self.set_focus(None);
        }
    }

    /// Registers a bubble-phase listener for `A` on `node`.
    pub fn on_action<A: Action>(&mut self, node: NodeId, listener: impl Fn(&A, &mut ActionCx<'_, C>) + 'static) {
        let listener: Listener<C> = Rc::new(move |action, cx| {
            let action: &dyn Any = action;
            if let Some(action) = action.downcast_ref::<A>() {
                listener(action, cx);
            }
        });
        self.node_mut(node).listeners.push((TypeId::of::<A>(), listener));
    }

    /// Drops every listener on `node`. Retained nodes swap listeners each render without a
    /// backend call (3.7).
    pub fn clear_listeners(&mut self, node: NodeId) {
        self.node_mut(node).listeners.clear();
    }

    pub fn focused(&self) -> Option<NodeId> {
        self.focused
    }

    /// Focuses `node` from nd. Returns false when it is not focusable. The platform learns of
    /// it through [`Self::take_focus_change`].
    pub fn focus(&mut self, node: NodeId) -> bool {
        if !self.is_focusable(node) {
            return false;
        }
        self.set_focus(Some(node));
        true
    }

    pub fn blur(&mut self) {
        self.set_focus(None);
    }

    /// Records a focus change the platform made itself (a click into a native field, AppKit
    /// `becomeFirstResponder`). Produces no focus change for the platform, since it already
    /// has it. Returns false when `node` is unknown or not focusable, leaving nd focus as is.
    pub fn platform_focused(&mut self, node: Option<NodeId>) -> bool {
        if let Some(n) = node
            && !self.is_focusable(n)
        {
            return false;
        }
        if self.focused != node {
            self.focused = node;
            self.pending.clear();
        }
        self.focus_change = None;
        true
    }

    /// The focus the platform has to mirror, once per change.
    pub fn take_focus_change(&mut self) -> Option<Option<NodeId>> {
        self.focus_change.take()
    }

    /// Tab stops in traversal order: by `tab_index`, then tree order.
    pub fn focus_order(&self) -> Vec<NodeId> {
        let mut order = Vec::new();
        let mut stack: Vec<NodeId> = self.root.into_iter().collect();
        while let Some(n) = stack.pop() {
            let Some(node) = self.nodes.get(&n) else { continue };
            if let Some(f) = node.focusable
                && f.tab_stop
            {
                order.push((f.tab_index, order.len(), n));
            }
            stack.extend(node.children.iter().rev());
        }
        order.sort_by_key(|(tab_index, tree_index, _)| (*tab_index, *tree_index));
        order.into_iter().map(|(_, _, n)| n).collect()
    }

    pub fn focus_next(&mut self) {
        self.step_focus(1);
    }

    pub fn focus_previous(&mut self) {
        self.step_focus(-1);
    }

    fn step_focus(&mut self, step: isize) {
        let order = self.focus_order();
        if order.is_empty() {
            return;
        }
        let len = order.len() as isize;
        let next = match self.focused.and_then(|f| order.iter().position(|n| *n == f)) {
            Some(i) => (i as isize + step).rem_euclid(len),
            None if step > 0 => 0,
            None => len - 1,
        };
        self.set_focus(Some(order[next as usize]));
    }

    fn set_focus(&mut self, node: Option<NodeId>) {
        if self.focused != node {
            self.focused = node;
            self.focus_change = Some(node);
            self.pending.clear();
        }
    }

    fn is_focusable(&self, node: NodeId) -> bool {
        self.nodes.get(&node).is_some_and(|n| n.focusable.is_some())
    }

    fn node_mut(&mut self, node: NodeId) -> &mut Node<C> {
        match self.nodes.get_mut(&node) {
            Some(n) => n,
            None => panic!("{node:?} is not in the dispatch tree"),
        }
    }

    /// Focused node up to the root; from the default target, or just the root, when nothing
    /// has focus.
    fn dispatch_path(&self) -> Vec<NodeId> {
        let mut path = Vec::new();
        let mut cursor = self.focused.or(self.default_target).or(self.root);
        while let Some(n) = cursor {
            path.push(n);
            cursor = self.nodes.get(&n).and_then(|n| n.parent);
        }
        path
    }

    /// The key contexts along the dispatch path, root first, as the keymap expects.
    pub fn context_stack(&self) -> Vec<KeyContext> {
        let mut stack: Vec<KeyContext> =
            self.dispatch_path().iter().filter_map(|n| self.nodes[n].key_context.clone()).collect();
        stack.reverse();
        stack
    }

    /// Matches `keystroke` against the keymap in the focused node's contexts and dispatches
    /// the bound action. Runs before the platform's own key handling.
    pub fn dispatch_key(&mut self, keystroke: &Keystroke, cx: &mut C) -> KeyDispatch {
        let contexts = self.context_stack();
        let mut input = std::mem::take(&mut self.pending);
        let had_pending = !input.is_empty();
        input.push(keystroke.clone());
        let (bindings, pending) = self.keymap.bindings_for_input(&input, &contexts);
        if pending {
            self.pending = input;
            return KeyDispatch::Pending;
        }
        for binding in &bindings {
            if self.dispatch_action(binding.action(), cx) {
                return KeyDispatch::Handled;
            }
        }
        if bindings.is_empty() && had_pending {
            // The pending prefix led nowhere: match this keystroke on its own.
            return self.dispatch_key(keystroke, cx);
        }
        KeyDispatch::Unhandled
    }

    /// Bubbles `action` from the focused node to the root. Menus and toolbar items dispatch
    /// through here too, so they reach the same listeners as key bindings. Returns whether a
    /// listener (or nd's built-in focus traversal) handled it.
    pub fn dispatch_action(&mut self, action: &dyn Action, cx: &mut C) -> bool {
        let type_id = {
            let any: &dyn Any = action;
            any.type_id()
        };
        let path = self.dispatch_path();
        let listeners: Vec<Listener<C>> = path
            .iter()
            .flat_map(|n| self.nodes[n].listeners.iter())
            .filter(|(t, _)| *t == type_id)
            .map(|(_, l)| l.clone())
            .collect();
        let mut focus = None;
        let mut handled = false;
        for listener in listeners {
            let mut acx = ActionCx { cx: &mut *cx, propagate: false, focus: &mut focus };
            listener(action, &mut acx);
            if !acx.propagate {
                handled = true;
                break;
            }
        }
        if !handled {
            if type_id == TypeId::of::<FocusNext>() {
                focus = Some(FocusRequest::Next);
                handled = true;
            } else if type_id == TypeId::of::<FocusPrevious>() {
                focus = Some(FocusRequest::Previous);
                handled = true;
            }
        }
        match focus {
            Some(FocusRequest::Node(n)) => {
                self.focus(n);
            }
            Some(FocusRequest::Next) => self.focus_next(),
            Some(FocusRequest::Previous) => self.focus_previous(),
            Some(FocusRequest::Clear) => self.blur(),
            None => {}
        }
        handled
    }
}
