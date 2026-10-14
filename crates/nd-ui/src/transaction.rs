//! Collects one window's ops for a frame and hands them to a [`Backend`] in [`Phase`] order.

use std::collections::HashMap;

use nd_core::{NodeId, Rect, Size, WindowId};

use crate::backend::{Backend, Constraint, NodeKind, Phase, Props, PropsPatch, SurfaceOp};

#[derive(Clone, Debug)]
pub enum Op {
    Create {
        node: NodeId,
        kind: NodeKind,
        props: Props,
    },
    Insert {
        parent: NodeId,
        node: NodeId,
        before: Option<NodeId>,
    },
    Move {
        node: NodeId,
        parent: NodeId,
        before: Option<NodeId>,
    },
    Remove {
        node: NodeId,
    },
    Update {
        node: NodeId,
        patch: PropsPatch,
    },
    /// The node emitted an event since the last frame: its native object takes its current
    /// props again, so a change the app refused (props unchanged) does not stay on screen.
    Reaffirm {
        node: NodeId,
    },
    SetFrame {
        node: NodeId,
        frame: Rect,
    },
    Surface {
        node: NodeId,
        op: SurfaceOp,
    },
    Focus {
        node: Option<NodeId>,
    },
    Release {
        node: NodeId,
    },
}

impl Op {
    pub fn phase(&self) -> Phase {
        match self {
            Op::Create { .. } => Phase::Create,
            Op::Insert { .. } | Op::Move { .. } | Op::Remove { .. } => Phase::Structure,
            Op::Update { .. } | Op::Reaffirm { .. } | Op::Surface { .. } => Phase::Update,
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
            Op::Reaffirm { node } => backend.reaffirm(*node),
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

    /// Drops a pending detach of `node`, for a node another window's transaction detaches
    /// instead. True when one was pending.
    pub(crate) fn take_remove(&mut self, node: NodeId) -> bool {
        let before = self.ops.len();
        self.ops.retain(|op| !matches!(op, Op::Remove { node: n } if *n == node));
        self.ops.len() != before
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
    pub fn commit(self, backend: &mut dyn Backend, cache: &mut MeasureCache) {
        self.commit_with(backend, cache, false, |_, _| Vec::new());
    }

    /// Like [`Self::commit`], with layout inside the transaction: ops up to the update phase
    /// apply first, then `layout` measures against the updated backend and returns the frame
    /// ops, then focus and releases. `force` opens the transaction even with no ops pushed
    /// (a resize that only moves frames).
    pub fn commit_with(
        mut self,
        backend: &mut dyn Backend,
        cache: &mut MeasureCache,
        force: bool,
        layout: impl FnOnce(&mut dyn Backend, &mut MeasureCache) -> Vec<Op>,
    ) {
        if self.ops.is_empty() && !force {
            return;
        }
        self.ops.sort_by_key(Op::phase);
        let frames = backend.retains_frames();
        let split = self.ops.partition_point(|op| op.phase() < Phase::Frame);
        let late = self.ops.split_off(split);
        backend.begin(self.window);
        for op in &self.ops {
            Self::apply(op, backend, cache, frames);
        }
        let mut late_ops = layout(backend, cache);
        late_ops.extend(late);
        late_ops.sort_by_key(Op::phase);
        for op in &late_ops {
            Self::apply(op, backend, cache, frames);
        }
        backend.commit(self.window);
    }

    fn apply(op: &Op, backend: &mut dyn Backend, cache: &mut MeasureCache, frames: bool) {
        match op {
            Op::SetFrame { .. } if !frames => return,
            Op::Update { node, .. } | Op::Release { node } => cache.invalidate(*node),
            _ => {}
        }
        op.apply(backend);
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
