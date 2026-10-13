//! Collects one window's ops for a frame and hands them to a [`Backend`] in [`Phase`] order.

use std::collections::HashMap;

use nd_core::{NodeId, Rect, Size, WindowId};

use crate::backend::{Backend, Constraint, NodeKind, Phase, Props, PropsPatch, SurfaceOp};

#[derive(Clone, Debug)]
pub enum Op {
    Create { node: NodeId, kind: NodeKind, props: Props },
    Insert { parent: NodeId, node: NodeId, before: Option<NodeId> },
    Move { node: NodeId, parent: NodeId, before: Option<NodeId> },
    Remove { node: NodeId },
    Update { node: NodeId, patch: PropsPatch },
    SetFrame { node: NodeId, frame: Rect },
    Surface { node: NodeId, op: SurfaceOp },
    Focus { node: Option<NodeId> },
    Release { node: NodeId },
}

impl Op {
    pub fn phase(&self) -> Phase {
        match self {
            Op::Create { .. } => Phase::Create,
            Op::Insert { .. } | Op::Move { .. } | Op::Remove { .. } => Phase::Structure,
            Op::Update { .. } | Op::Surface { .. } => Phase::Update,
            Op::SetFrame { .. } => Phase::Frame,
            Op::Focus { .. } => Phase::Focus,
            Op::Release { .. } => Phase::Release,
        }
    }

    pub fn apply(&self, backend: &mut dyn Backend) {
        match self {
            Op::Create { node, kind, props } => backend.create(*node, *kind, props),
            Op::Insert { parent, node, before } => backend.insert(*parent, *node, *before),
            Op::Move { node, parent, before } => backend.move_node(*node, *parent, *before),
            Op::Remove { node } => backend.remove(*node),
            Op::Update { node, patch } => backend.update(*node, patch),
            Op::SetFrame { node, frame } => backend.set_frame(*node, *frame),
            Op::Surface { node, op } => backend.surface(*node, op),
            Op::Focus { node } => backend.focus(*node),
            Op::Release { node } => backend.release(*node),
        }
    }
}

/// One window's ops for one frame. Ops are pushed in reconcile order (tree order, so parents
/// before children) and applied grouped by phase, keeping push order inside a phase.
pub struct Transaction {
    window: WindowId,
    ops: Vec<Op>,
}

impl Transaction {
    pub fn new(window: WindowId) -> Self {
        Self { window, ops: Vec::new() }
    }

    pub fn window(&self) -> WindowId {
        self.window
    }

    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    pub fn push(&mut self, op: Op) {
        // Only the last focus request of a frame reaches the platform.
        if matches!(op, Op::Focus { .. }) {
            self.ops.retain(|op| !matches!(op, Op::Focus { .. }));
        }
        self.ops.push(op);
    }

    /// Applies the ops and invalidates `cache` for every node whose props changed or that was
    /// released. Frames are dropped for backends that lay out themselves.
    pub fn commit(mut self, backend: &mut dyn Backend, cache: &mut MeasureCache) {
        if self.ops.is_empty() {
            return;
        }
        self.ops.sort_by_key(Op::phase);
        let frames = backend.retains_frames();
        backend.begin(self.window);
        for op in &self.ops {
            match op {
                Op::SetFrame { .. } if !frames => continue,
                Op::Update { node, .. } | Op::Release { node } => cache.invalidate(*node),
                _ => {}
            }
            op.apply(backend);
        }
        backend.commit(self.window);
    }
}

/// Measure results keyed by node and constraint. A node's entries drop when its props change,
/// which is the only input to a native leaf's measure besides the constraint.
#[derive(Default)]
pub struct MeasureCache {
    entries: HashMap<NodeId, Vec<(Constraint, Size)>>,
}

impl MeasureCache {
    pub fn measure(&mut self, backend: &mut dyn Backend, node: NodeId, constraint: Constraint) -> Size {
        let entries = self.entries.entry(node).or_default();
        if let Some((_, size)) = entries.iter().find(|(c, _)| *c == constraint) {
            return *size;
        }
        let size = backend.measure(node, constraint);
        entries.push((constraint, size));
        size
    }

    pub fn invalidate(&mut self, node: NodeId) {
        self.entries.remove(&node);
    }
}
