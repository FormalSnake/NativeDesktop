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

use crate::actions::{Cancel, Confirm, FocusNext, FocusPrevious};

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

/// The last input that moved things: decides whether focus shows a ring. A click focuses
/// quietly; Tab, or any key, shows where focus is (`:focus-visible` semantics).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InputModality {
    #[default]
    Pointer,
    Keyboard,
}

/// A subtree that owns focus while it is mounted: an overlay, a sheet, a palette. Mounting it
/// saves the focus it took over; unmounting restores it (ARCHITECTURE.md 6.2, 6.3).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FocusScope {
    /// Tab cycles inside the scope, and mounting it moves focus to its first tab stop (modal).
    pub trap: bool,
    /// Escape that no listener handled dismisses this scope.
    pub dismiss_on_escape: bool,
}

/// Queued to a focus scope's root when Escape dismissed it; the overlay closes itself in its
/// handler.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DismissRequested;

/// Per-node roles in the Return and Escape defaults.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KeyRoles {
    /// Activated by Return when no listener handled it (the window's or the scope's default button).
    pub default_button: bool,
    /// Keeps Return for itself while focused (a multi-line text view): the default button waits.
    pub takes_return: bool,
}

struct ScopeEntry {
    node: NodeId,
    scope: FocusScope,
    /// Focus when the scope mounted, restored when it unmounts.
    restore: Option<NodeId>,
}

struct Node<C> {
    parent: Option<NodeId>,
    children: Vec<NodeId>,
    key_context: Option<KeyContext>,
    listeners: Vec<(TypeId, Listener<C>)>,
    focusable: Option<Focusable>,
    roles: KeyRoles,
}

impl<C> Node<C> {
    fn new(parent: Option<NodeId>) -> Self {
        Self {
            parent,
            children: Vec::new(),
            key_context: None,
            listeners: Vec::new(),
            focusable: None,
            roles: KeyRoles::default(),
        }
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
    modality: InputModality,
    /// Mounted focus scopes, outermost first.
    scopes: Vec<ScopeEntry>,
    /// A default button Return activated, for the app to click.
    activate: Option<NodeId>,
    /// A scope Escape dismissed, for the app to tell.
    dismiss: Option<NodeId>,
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
            modality: InputModality::default(),
            scopes: Vec::new(),
            activate: None,
            dismiss: None,
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
        let mut removed_nodes = Vec::new();
        while let Some(n) = stack.pop() {
            if let Some(removed) = self.nodes.remove(&n) {
                stack.extend(removed.children);
            }
            removed_nodes.push(n);
            if self.focused == Some(n) {
                self.set_focus(None);
            }
            if self.default_target == Some(n) {
                self.default_target = None;
            }
        }
        for n in removed_nodes {
            self.unmount_scope(n);
        }
    }

    pub fn contains(&self, node: NodeId) -> bool {
        self.nodes.contains_key(&node)
    }

    /// Marks `node` as a focus scope root (`Some`) or not. Becoming one saves the current focus
    /// and, for a trapping scope, moves focus to its first tab stop unless focus is already
    /// inside; ceasing to be one (or leaving the tree) restores the saved focus.
    pub fn set_focus_scope(&mut self, node: NodeId, scope: Option<FocusScope>) {
        let existing = self.scopes.iter().position(|s| s.node == node);
        match (existing, scope) {
            (Some(i), Some(scope)) => self.scopes[i].scope = scope,
            (Some(_), None) => self.unmount_scope(node),
            (None, Some(scope)) => {
                let restore = self.focused;
                self.scopes.push(ScopeEntry { node, scope, restore });
                if scope.trap && !self.focused.is_some_and(|f| self.is_within(f, node)) {
                    // The scope's children may not be bound yet in this render; the tree calls
                    // `settle_scopes` after the pass to finish the move.
                    if let Some(first) = self.focus_order_in(Some(node)).first().copied() {
                        self.set_focus(Some(first));
                    }
                }
            }
            (None, None) => {}
        }
    }

    /// After a render pass: a trapping scope that mounted before its content gets its focus now.
    pub fn settle_scopes(&mut self) {
        let Some(top) = self.scopes.iter().rev().find(|s| s.scope.trap).map(|s| s.node) else { return };
        if self.focused.is_some_and(|f| self.is_within(f, top)) {
            return;
        }
        if let Some(first) = self.focus_order_in(Some(top)).first().copied() {
            self.set_focus(Some(first));
        }
    }

    fn unmount_scope(&mut self, node: NodeId) {
        let Some(i) = self.scopes.iter().position(|s| s.node == node) else { return };
        let entry = self.scopes.remove(i);
        // Restore only when focus was inside the closing scope (or went with it): an overlay
        // closing never steals focus the user moved elsewhere.
        let inside = match self.focused {
            None => true,
            Some(f) => !self.nodes.contains_key(&f) || self.is_within(f, node),
        };
        if !inside {
            return;
        }
        let target = entry.restore.filter(|r| self.is_focusable(*r) && self.in_active_scope(*r));
        match target {
            Some(r) => self.set_focus(Some(r)),
            None => {
                let fallback = self.scopes.iter().rev().find(|s| s.scope.trap).map(|s| s.node);
                let first = fallback.and_then(|f| self.focus_order_in(Some(f)).first().copied());
                self.set_focus(first);
            }
        }
    }

    pub fn set_key_roles(&mut self, node: NodeId, roles: KeyRoles) {
        self.node_mut(node).roles = roles;
    }

    /// Records the input that last moved things. Keys show focus rings, pointers hide them.
    pub fn set_modality(&mut self, modality: InputModality) {
        self.modality = modality;
    }

    pub fn modality(&self) -> InputModality {
        self.modality
    }

    /// Whether the focused node draws its focus ring: something has focus and the keyboard
    /// moved last.
    pub fn focus_visible(&self) -> bool {
        self.focused.is_some() && self.modality == InputModality::Keyboard
    }

    /// The default button Return activated in the last dispatch, once.
    pub fn take_activation(&mut self) -> Option<NodeId> {
        self.activate.take()
    }

    /// The focus scope Escape dismissed in the last dispatch, once.
    pub fn take_dismissal(&mut self) -> Option<NodeId> {
        self.dismiss.take()
    }

    fn is_within(&self, node: NodeId, ancestor: NodeId) -> bool {
        let mut cursor = Some(node);
        while let Some(n) = cursor {
            if n == ancestor {
                return true;
            }
            cursor = self.nodes.get(&n).and_then(|n| n.parent);
        }
        false
    }

    /// The innermost trapping scope, which confines Tab and focus.
    fn trap(&self) -> Option<NodeId> {
        self.scopes.iter().rev().find(|s| s.scope.trap).map(|s| s.node)
    }

    fn in_active_scope(&self, node: NodeId) -> bool {
        self.trap().is_none_or(|t| self.is_within(node, t))
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

    /// Tab stops in traversal order: by `tab_index`, then tree order. Inside a trapping focus
    /// scope (a modal overlay), only the scope's own.
    pub fn focus_order(&self) -> Vec<NodeId> {
        self.focus_order_in(self.trap())
    }

    fn focus_order_in(&self, within: Option<NodeId>) -> Vec<NodeId> {
        let mut order = Vec::new();
        let mut stack: Vec<NodeId> = within.or(self.root).into_iter().collect();
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

    pub(crate) fn is_focusable(&self, node: NodeId) -> bool {
        self.nodes.get(&node).is_some_and(|n| n.focusable.is_some())
    }

    fn node_mut(&mut self, node: NodeId) -> &mut Node<C> {
        match self.nodes.get_mut(&node) {
            Some(n) => n,
            None => panic!("{node:?} is not in the dispatch tree"),
        }
    }

    /// The button Return activates: the first default button in tree order inside the innermost
    /// scope (or the window), unless the focused node keeps Return.
    fn default_button(&self) -> Option<NodeId> {
        if self.focused.and_then(|f| self.nodes.get(&f)).is_some_and(|n| n.roles.takes_return) {
            return None;
        }
        let within = self.scopes.last().map(|s| s.node).or(self.root)?;
        let mut stack = vec![within];
        while let Some(n) = stack.pop() {
            let Some(node) = self.nodes.get(&n) else { continue };
            if node.roles.default_button {
                return Some(n);
            }
            stack.extend(node.children.iter().rev());
        }
        None
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
        self.modality = InputModality::Keyboard;
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
            } else if type_id == TypeId::of::<Confirm>() {
                self.activate = self.default_button();
                handled = self.activate.is_some();
            } else if type_id == TypeId::of::<Cancel>() {
                self.dismiss = self.scopes.iter().rev().find(|s| s.scope.dismiss_on_escape).map(|s| s.node);
                handled = self.dismiss.is_some();
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
