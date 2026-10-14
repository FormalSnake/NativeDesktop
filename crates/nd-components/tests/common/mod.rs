#![allow(dead_code)]

use nd_ui::backend::{NodeKind, WidgetKind};
use nd_ui::prelude::*;
use nd_ui::semantics::{ActionError, SemanticAction};
use nd_ui::test_app::TestApp;
use nd_ui::transaction::Op;
use nd_ui::{NodeId, SemanticNode, Size};

pub const WINDOW: Size = Size { width: 480., height: 640. };

/// A view holding test state `S`, rendered by `view`.
pub struct Host<S: 'static> {
    pub s: S,
    view: fn(&S, &mut Context<Host<S>>) -> AnyElement,
}

impl<S: 'static> Render for Host<S> {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        (self.view)(&self.s, cx)
    }
}

pub struct Rig<S: 'static> {
    pub cx: TestApp,
    pub win: WindowHandle<Host<S>>,
}

impl<S: 'static> Rig<S> {
    pub fn new(s: S, view: fn(&S, &mut Context<Host<S>>) -> AnyElement) -> Self {
        Self::build(|_| s, view)
    }

    /// As [`Rig::new`], with the state built against the app (for entities).
    pub fn build(s: impl FnOnce(&mut TestApp) -> S, view: fn(&S, &mut Context<Host<S>>) -> AnyElement) -> Self {
        Self::sized(WINDOW, s, view)
    }

    pub fn sized(
        size: Size,
        s: impl FnOnce(&mut TestApp) -> S,
        view: fn(&S, &mut Context<Host<S>>) -> AnyElement,
    ) -> Self {
        let mut cx = TestApp::new();
        let s = s(&mut cx);
        let win = cx.open_window(size, |_, cx| cx.new(|_| Host { s, view }));
        Self { cx, win }
    }

    pub fn state(&self) -> &S {
        &self.win.root().read(&self.cx).s
    }

    /// Changes the state the way an app does and renders again.
    pub fn mutate(&mut self, f: impl FnOnce(&mut S)) {
        self.win.root().clone().update(&mut self.cx, |h, cx| {
            f(&mut h.s);
            cx.notify();
        });
        self.cx.run_until_idle();
    }

    pub fn node(&self, id: &str) -> NodeId {
        self.sem(id).id
    }

    pub fn sem(&self, id: &str) -> SemanticNode {
        let tree = self.cx.semantic_tree();
        match tree.nodes().find(|(_, n)| n.test_id.as_deref() == Some(id)) {
            Some((_, n)) => n.clone(),
            None => panic!("no node with id {id:?} in\n{}", tree.snapshot()),
        }
    }

    /// The props the backend holds for the node with `id`.
    pub fn props<P: Clone + 'static>(&self, id: &str) -> P {
        let node = self.node(id);
        let props = self.cx.backend().props(node).expect("live node");
        props.downcast_ref::<P>().unwrap_or_else(|| panic!("{id} has other props: {props:?}")).clone()
    }

    pub fn kind_is<K: 'static>(&self, id: &str) -> bool {
        self.cx.backend().kind(self.node(id)) == Some(NodeKind::Widget(WidgetKind::of::<K>()))
    }

    /// Emits `event` from the node as its native control would.
    pub fn emit<E: 'static>(&mut self, id: &str, event: E) {
        let node = self.node(id);
        self.cx.emit(node, event);
    }

    pub fn perform(&mut self, id: &str, action: SemanticAction) -> Result<(), ActionError> {
        let node = self.node(id);
        let result = self.cx.perform(node, &action);
        self.cx.run_until_idle();
        result
    }

    pub fn set_value(&mut self, id: &str, value: &str) -> Result<(), ActionError> {
        self.perform(id, SemanticAction::SetValue(value.into()))
    }

    /// Forgets the log so far, so the next count covers only what follows.
    pub fn fresh(&mut self) {
        self.cx.backend_mut().take_log();
    }

    /// Update ops sent for the node with `id` since the log was last taken.
    pub fn updates(&self, id: &str) -> usize {
        let node = self.node(id);
        self.cx.backend().ops().filter(|op| matches!(op, Op::Update { node: n, .. } if *n == node)).count()
    }

    /// Update ops for any node since the log was last taken.
    pub fn all_updates(&self) -> usize {
        self.cx.backend().ops().filter(|op| matches!(op, Op::Update { .. })).count()
    }

    /// Renders again with nothing changed.
    pub fn rerender(&mut self) {
        self.win.root().clone().update(&mut self.cx, |_, cx| cx.notify());
        self.cx.run_until_idle();
    }

    /// Nodes whose props are of type `P`, in id order.
    pub fn find_props<P: 'static>(&self) -> Vec<NodeId> {
        self.cx.backend().find(|_, p| p.downcast_ref::<P>().is_some())
    }
}

pub fn refused(result: Result<(), ActionError>) -> bool {
    matches!(result, Err(ActionError::NotActionable(_)))
}
