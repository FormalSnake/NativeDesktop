//! Builds the semantic tree from a window's retained tree (native children in sibling order,
//! frames accumulated to window coordinates) and the backend's probes, and routes semantic
//! actions to the backend.

use nd_core::semantic::{SemanticNode, SemanticTree, SemanticWindow};
use nd_core::{NodeId, Point, Rect, WindowId};

use super::{SlotKind, Tree};
use crate::app::App;
use crate::backend::Backend;
use crate::element::Key;
use crate::semantics::{
    ActionError, Capture, SemanticAction, SyntheticInput, WindowState, describe, part_id, part_owner,
};
use crate::window::Window;

fn intersects(a: Rect, b: Rect) -> bool {
    a.origin.x < b.origin.x + b.size.width
        && b.origin.x < a.origin.x + a.size.width
        && a.origin.y < b.origin.y + b.size.height
        && b.origin.y < a.origin.y + a.size.height
}

impl Tree {
    fn semantic_node(
        &self,
        id: NodeId,
        origin: Point,
        viewport: Rect,
        window_visible: bool,
        backend: &dyn Backend,
    ) -> SemanticNode {
        let node = &self.nodes[&id];
        let SlotKind::Node(kind) = node.ident.kind else { unreachable!("retained nodes are native nodes") };
        let described = describe(kind, &node.props);
        let laid_out = if id == self.root {
            Some(viewport)
        } else {
            node.frame.map(|f| Rect::new(origin.x + f.origin.x, origin.y + f.origin.y, f.size.width, f.size.height))
        };
        let child_origin = laid_out.map_or(origin, |r| r.origin);
        let probe = backend.probe(id);
        let bounds = probe.bounds.or(laid_out);
        let visible = window_visible
            && bounds.is_some_and(|b| b.size.width > 0. && b.size.height > 0. && intersects(b, viewport));
        let test_id = match &node.ident.key {
            Key::Id(id) => Some(id.to_string()),
            Key::Site(_) | Key::Entity(_) => None,
        };
        let focused = self.dispatch.focused() == Some(id);
        let enabled = probe.enabled.or(described.extra.enabled).unwrap_or(true);
        let parts = described.extra.parts.iter().enumerate().map(|(i, part)| SemanticNode {
            id: part_id(id, i),
            role: part.role,
            kind: format!("{}.{}", described.kind, part.key),
            name: part.name.clone(),
            value: part.value.clone(),
            test_id: test_id.as_ref().map(|t| format!("{t}.{}", part.key)),
            placeholder: part.placeholder.clone(),
            enabled,
            focused: focused && part.focused,
            checked: None,
            selected: None,
            expanded: None,
            bounds,
            visible,
            children: Vec::new(),
        });
        let parts: Vec<SemanticNode> = parts.collect();
        SemanticNode {
            id,
            role: described.role,
            kind: described.kind,
            name: probe.name.unwrap_or(described.name),
            value: probe.value.or(described.extra.value),
            test_id: if id == self.root { None } else { test_id },
            placeholder: probe.placeholder.or(described.extra.placeholder),
            enabled,
            focused,
            checked: probe.checked.or(described.extra.checked),
            selected: probe.selected.or(described.extra.selected),
            expanded: probe.expanded.or(described.extra.expanded),
            bounds,
            visible,
            children: parts
                .into_iter()
                .chain(
                    node.native.iter().map(|c| self.semantic_node(*c, child_origin, viewport, window_visible, backend)),
                )
                .collect(),
        }
    }

    pub(crate) fn semantic_window(
        &self,
        window: &Window,
        state: &WindowState,
        backend: &dyn Backend,
    ) -> SemanticWindow {
        let size = window.viewport_size();
        let viewport = Rect::new(0., 0., size.width, size.height);
        let mut root = self.semantic_node(self.root, Point::default(), viewport, state.visible, backend);
        root.name = window.title().to_string();
        SemanticWindow {
            id: self.window,
            title: window.title().to_string(),
            key: state.key,
            main: state.main,
            visible: state.visible,
            frame: state.frame,
            root,
        }
    }
}

impl App {
    /// The window as the OS has it, or, on a backend without OS windows, visible with the first
    /// window key and main.
    fn window_state_or_default(&self, id: WindowId) -> WindowState {
        self.backend.window_state(id).unwrap_or_else(|| {
            let first = self.windows.keys().next() == Some(&id);
            WindowState { key: first, main: first, visible: true, frame: None }
        })
    }

    /// One window's semantic tree. `None` when the window is closed or a handler holds it.
    pub fn semantic_window(&self, id: WindowId) -> Option<SemanticWindow> {
        let slot = self.windows.get(&id)?;
        let (window, tree) = (slot.window.as_ref()?, slot.tree.as_ref()?);
        Some(tree.semantic_window(window, &self.window_state_or_default(id), &*self.backend))
    }

    /// The semantic tree of every window (ARCHITECTURE.md 10.1): what automation, `nd-test`
    /// locators, MCP snapshots and incident bundles read. Built on demand from the retained tree
    /// and backend probes; costs one walk of the retained tree.
    pub fn semantic_tree(&self) -> SemanticTree {
        SemanticTree { windows: self.windows.keys().filter_map(|id| self.semantic_window(*id)).collect() }
    }

    /// The window holding `node`, while it is in a retained tree.
    pub fn window_of(&self, node: NodeId) -> Option<WindowId> {
        self.windows.iter().find(|(_, slot)| slot.tree.as_ref().is_some_and(|t| t.contains(node))).map(|(id, _)| *id)
    }

    /// Carries out a semantic action on `node` through the backend. `Focus` on a `focusable`
    /// element moves nd focus, which reaches the backend as the next frame's focus op; other
    /// nodes fall back to the backend's own focus (a native view nd does not track).
    /// Events the action causes are queued; hosts drain them on the next turn, tests with
    /// `run_until_idle`.
    pub fn perform(&mut self, node: NodeId, action: &SemanticAction) -> Result<(), ActionError> {
        let node = part_owner(node).unwrap_or(node);
        let Some(window) = self.window_of(node) else {
            return Err(ActionError::NotActionable(format!("{node:?} is not in any window")));
        };
        if *action == SemanticAction::Focus {
            if self.focus(window, node) {
                return Ok(());
            }
            return match self.backend.perform(node, action) {
                Err(ActionError::Unsupported(_)) => {
                    Err(ActionError::NotActionable(format!("{node:?} is not focusable")))
                }
                result => result,
            };
        }
        self.backend.perform(node, action)
    }

    /// Posts synthetic input into `window`'s event queue through the backend.
    pub fn synthesize(&mut self, window: WindowId, input: &SyntheticInput) -> Result<(), ActionError> {
        if !self.windows.contains_key(&window) {
            return Err(ActionError::WindowGone);
        }
        self.backend.synthesize(window, input)
    }

    /// A PNG of the window's content.
    pub fn capture(&mut self, window: WindowId) -> Result<Capture, ActionError> {
        if !self.windows.contains_key(&window) {
            return Err(ActionError::WindowGone);
        }
        self.backend.capture(window)
    }
}
