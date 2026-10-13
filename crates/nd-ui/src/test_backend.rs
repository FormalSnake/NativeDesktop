//! A backend with no OS: records every op and enforces the seam contract (see
//! [`crate::backend`]) by panicking on the first violation, so tier 1 tests fail at the op that
//! broke it.

use std::collections::HashMap;

use nd_core::{NodeId, Rect, Size, WindowId};

use crate::backend::{Backend, Constraint, NodeKind, Phase, Props, PropsPatch, SurfaceOp};
use crate::transaction::Op;

/// One recorded backend call.
#[derive(Clone, Debug)]
pub enum Recorded {
    Begin(WindowId),
    Op(Op),
    Commit(WindowId),
    Measure(NodeId, Constraint),
}

struct Node {
    kind: NodeKind,
    props: Props,
    parent: Option<NodeId>,
    children: Vec<NodeId>,
    frame: Option<Rect>,
}

pub struct TestBackend {
    log: Vec<Recorded>,
    nodes: HashMap<NodeId, Node>,
    open: Option<(WindowId, Phase)>,
    focused: Option<NodeId>,
    measure: Box<dyn FnMut(NodeId, Constraint) -> Size>,
    retains_frames: bool,
}

impl Default for TestBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl TestBackend {
    pub fn new() -> Self {
        Self {
            log: Vec::new(),
            nodes: HashMap::new(),
            open: None,
            focused: None,
            measure: Box::new(|_, _| Size::default()),
            retains_frames: true,
        }
    }

    /// Behaves like the GPUI backend, which lays out itself and never receives frames.
    pub fn without_frames(mut self) -> Self {
        self.retains_frames = false;
        self
    }

    pub fn on_measure(&mut self, f: impl FnMut(NodeId, Constraint) -> Size + 'static) {
        self.measure = Box::new(f);
    }

    pub fn log(&self) -> &[Recorded] {
        &self.log
    }

    pub fn take_log(&mut self) -> Vec<Recorded> {
        std::mem::take(&mut self.log)
    }

    /// The ops of the recorded log, without transaction brackets and queries.
    pub fn ops(&self) -> impl Iterator<Item = &Op> {
        self.log.iter().filter_map(|r| match r {
            Recorded::Op(op) => Some(op),
            _ => None,
        })
    }

    pub fn is_live(&self, node: NodeId) -> bool {
        self.nodes.contains_key(&node)
    }

    pub fn live_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn kind(&self, node: NodeId) -> Option<NodeKind> {
        self.nodes.get(&node).map(|n| n.kind)
    }

    pub fn props(&self, node: NodeId) -> Option<&Props> {
        self.nodes.get(&node).map(|n| &n.props)
    }

    pub fn parent(&self, node: NodeId) -> Option<NodeId> {
        self.nodes.get(&node).and_then(|n| n.parent)
    }

    pub fn children(&self, node: NodeId) -> &[NodeId] {
        self.nodes.get(&node).map_or(&[], |n| &n.children)
    }

    pub fn frame(&self, node: NodeId) -> Option<Rect> {
        self.nodes.get(&node).and_then(|n| n.frame)
    }

    pub fn focused(&self) -> Option<NodeId> {
        self.focused
    }

    fn enter(&mut self, phase: Phase, what: &str) {
        let Some((_, last)) = self.open.as_mut() else {
            panic!("TestBackend: {what} outside a transaction");
        };
        assert!(phase >= *last, "TestBackend: {what} in phase {phase:?} after {last:?}");
        *last = phase;
    }

    fn node(&mut self, node: NodeId, what: &str) -> &mut Node {
        match self.nodes.get_mut(&node) {
            Some(n) => n,
            None => panic!("TestBackend: {what} names {node:?}, which is not live"),
        }
    }

    /// True when `node` hangs off a root, through its ancestors.
    fn is_rooted(&self, mut node: NodeId) -> bool {
        loop {
            let Some(n) = self.nodes.get(&node) else { return false };
            if n.kind == NodeKind::Root {
                return true;
            }
            match n.parent {
                Some(p) => node = p,
                None => return false,
            }
        }
    }

    fn attach(&mut self, parent: NodeId, node: NodeId, before: Option<NodeId>, what: &str) {
        let mut ancestor = Some(parent);
        while let Some(a) = ancestor {
            assert!(a != node, "TestBackend: {what} would make {node:?} its own ancestor");
            ancestor = self.nodes.get(&a).and_then(|n| n.parent);
        }
        let children = &mut self.node(parent, what).children;
        let at = match before {
            Some(b) => match children.iter().position(|c| *c == b) {
                Some(i) => i,
                None => panic!("TestBackend: {what} before {b:?}, which is not a child of {parent:?}"),
            },
            None => children.len(),
        };
        children.insert(at, node);
        self.node(node, what).parent = Some(parent);
    }

    fn detach(&mut self, node: NodeId, what: &str) {
        let Some(parent) = self.node(node, what).parent.take() else {
            panic!("TestBackend: {what} on {node:?}, which is not attached");
        };
        self.node(parent, what).children.retain(|c| *c != node);
    }
}

impl Backend for TestBackend {
    fn begin(&mut self, window: WindowId) {
        assert!(self.open.is_none(), "TestBackend: begin inside an open transaction");
        self.open = Some((window, Phase::Create));
        self.log.push(Recorded::Begin(window));
    }

    fn create(&mut self, node: NodeId, kind: NodeKind, props: &Props) {
        self.enter(Phase::Create, "create");
        assert!(!self.nodes.contains_key(&node), "TestBackend: create of live {node:?}");
        self.nodes.insert(node, Node { kind, props: props.clone(), parent: None, children: Vec::new(), frame: None });
        self.log.push(Recorded::Op(Op::Create { node, kind, props: props.clone() }));
    }

    fn insert(&mut self, parent: NodeId, node: NodeId, before: Option<NodeId>) {
        self.enter(Phase::Structure, "insert");
        let n = self.node(node, "insert");
        assert!(n.kind != NodeKind::Root, "TestBackend: insert of root {node:?}");
        assert!(n.parent.is_none(), "TestBackend: insert of attached {node:?}");
        self.attach(parent, node, before, "insert");
        self.log.push(Recorded::Op(Op::Insert { parent, node, before }));
    }

    fn move_node(&mut self, node: NodeId, parent: NodeId, before: Option<NodeId>) {
        self.enter(Phase::Structure, "move");
        self.detach(node, "move");
        self.attach(parent, node, before, "move");
        self.log.push(Recorded::Op(Op::Move { node, parent, before }));
    }

    fn remove(&mut self, node: NodeId) {
        self.enter(Phase::Structure, "remove");
        self.detach(node, "remove");
        if self.focused.is_some_and(|f| !self.is_rooted(f)) {
            self.focused = None;
        }
        self.log.push(Recorded::Op(Op::Remove { node }));
    }

    fn update(&mut self, node: NodeId, patch: &PropsPatch) {
        self.enter(Phase::Update, "update");
        let n = self.node(node, "update");
        assert!(
            n.props.props_eq(&*patch.old),
            "TestBackend: update of {node:?} from stale props {:?}, backend has {:?}",
            patch.old,
            n.props
        );
        assert!(!patch.old.props_eq(&*patch.new), "TestBackend: update of {node:?} with equal props");
        n.props = patch.new.clone();
        self.log.push(Recorded::Op(Op::Update { node, patch: patch.clone() }));
    }

    fn set_frame(&mut self, node: NodeId, frame: Rect) {
        self.enter(Phase::Frame, "set_frame");
        assert!(self.retains_frames, "TestBackend: set_frame on a backend that lays out itself");
        self.node(node, "set_frame").frame = Some(frame);
        self.log.push(Recorded::Op(Op::SetFrame { node, frame }));
    }

    fn surface(&mut self, node: NodeId, op: &SurfaceOp) {
        self.enter(Phase::Update, "surface");
        self.node(node, "surface");
        self.log.push(Recorded::Op(Op::Surface { node, op: op.clone() }));
    }

    fn focus(&mut self, node: Option<NodeId>) {
        self.enter(Phase::Focus, "focus");
        if let Some(n) = node {
            assert!(self.is_rooted(n), "TestBackend: focus on {n:?}, which is not in a window");
        }
        self.focused = node;
        self.log.push(Recorded::Op(Op::Focus { node }));
    }

    fn release(&mut self, node: NodeId) {
        self.enter(Phase::Release, "release");
        let n = self.node(node, "release");
        let kind = n.kind;
        assert!(
            kind == NodeKind::Root || !self.is_rooted(node),
            "TestBackend: release of {node:?} while it is still in a window"
        );
        if let Some(parent) = self.nodes[&node].parent
            && let Some(p) = self.nodes.get_mut(&parent)
        {
            p.children.retain(|c| *c != node);
        }
        let children = self.nodes.remove(&node).map(|n| n.children).unwrap_or_default();
        for c in children {
            if let Some(child) = self.nodes.get_mut(&c) {
                child.parent = None;
            }
        }
        if self.focused == Some(node) {
            self.focused = None;
        }
        self.log.push(Recorded::Op(Op::Release { node }));
    }

    fn commit(&mut self, window: WindowId) {
        match self.open.take() {
            Some((open, _)) => assert_eq!(open, window, "TestBackend: commit for another window"),
            None => panic!("TestBackend: commit without begin"),
        }
        self.log.push(Recorded::Commit(window));
    }

    fn measure(&mut self, node: NodeId, constraint: Constraint) -> Size {
        self.node(node, "measure");
        self.log.push(Recorded::Measure(node, constraint));
        (self.measure)(node, constraint)
    }

    fn boundary_size(&self, _node: NodeId) -> Option<Size> {
        None
    }

    fn retains_frames(&self) -> bool {
        self.retains_frames
    }
}
