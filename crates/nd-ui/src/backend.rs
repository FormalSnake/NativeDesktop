//! The backend seam (ARCHITECTURE.md 3.15). `nd-ui` drives every backend through [`Backend`]:
//! `nd-backend-appkit`, `nd-backend-gpui` and [`crate::test_backend::TestBackend`].
//!
//! Contract every implementation can rely on, and the conformance suite checks:
//! - Mutating ops arrive only between [`Backend::begin`] and [`Backend::commit`], one transaction
//!   per window per frame. [`crate::transaction::Transaction`] is the only producer.
//! - Inside a transaction ops arrive in [`Phase`] order: creates, structural ops (parents
//!   first), updates, frames, focus, releases.
//! - A node is named only between its `create` and its `release`. The backend holds one strong
//!   reference per node from `create` and drops it on `release`.
//! - Main thread only. A backend never calls back into `nd-ui` during apply; native callbacks
//!   go through the per-window event queue (3.13).
//! - During live resize the whole transaction runs synchronously inside the OS resize pass.

use std::any::{Any, TypeId};
use std::fmt;
use std::rc::Rc;

use nd_core::{NodeId, Rect, Size, WindowId};

/// What a retained node is, decided at create time and never changed. A node whose kind changes
/// is released and recreated by the reconciler.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NodeKind {
    /// The window's content root. Created once per window and never inserted anywhere.
    Root,
    /// A styled container (`div`); a layer-backed view on AppKit.
    View,
    /// A text run.
    Text,
    /// A `canvas` painted through `Painter`.
    Canvas,
    /// An `img`.
    Image,
    /// A `NativeWidget`, built-in or app-defined.
    Widget(WidgetKind),
}

/// Names a `NativeWidget` implementation by its Rust type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WidgetKind {
    pub name: &'static str,
    pub type_id: TypeId,
}

impl WidgetKind {
    pub fn of<W: 'static>() -> Self {
        Self { name: std::any::type_name::<W>(), type_id: TypeId::of::<W>() }
    }
}

/// Type-erased props of one node. Each kind downcasts to its own props type.
pub trait AnyProps: Any + fmt::Debug {
    fn props_eq(&self, other: &dyn AnyProps) -> bool;
}

impl<T: Any + fmt::Debug + PartialEq> AnyProps for T {
    fn props_eq(&self, other: &dyn AnyProps) -> bool {
        let other: &dyn Any = other;
        other.downcast_ref::<T>().is_some_and(|other| self == other)
    }
}

impl dyn AnyProps {
    pub fn downcast_ref<T: Any>(&self) -> Option<&T> {
        let this: &dyn Any = self;
        this.downcast_ref()
    }
}

pub type Props = Rc<dyn AnyProps>;

/// Both sides of a props change. The reconciler sends an update only when `old != new`, and a
/// widget's `update` diffs the two to call only the setters that changed.
#[derive(Clone, Debug)]
pub struct PropsPatch {
    pub old: Props,
    pub new: Props,
}

/// One axis of a measure constraint, in logical points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AvailableSpace {
    Definite(f32),
    MinContent,
    MaxContent,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Constraint {
    pub width: AvailableSpace,
    pub height: AvailableSpace,
}

/// Ops on a `NativeSurface` (section 8). The full set lands with surfaces in M4.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum SurfaceOp {
    SetVisible(bool),
    /// Regions covered by overlays, in the surface's coordinates.
    SetOcclusion(Vec<Rect>),
}

/// Where an op runs inside a transaction. Ordered: a transaction never sends an op of an
/// earlier phase after one of a later phase.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Phase {
    Create,
    Structure,
    Update,
    Frame,
    Focus,
    Release,
}

pub trait Backend {
    /// Opens the transaction for one window's frame. AppKit begins a `CATransaction` with
    /// implicit animations off.
    fn begin(&mut self, window: WindowId);
    /// Builds the native object and takes the node's strong reference. Not yet attached.
    fn create(&mut self, node: NodeId, kind: NodeKind, props: &Props);
    /// Structural: attaches a created, detached node under `parent`, before `before` or last.
    fn insert(&mut self, parent: NodeId, node: NodeId, before: Option<NodeId>);
    /// Structural: moves an attached node, keeping its native object and state.
    fn move_node(&mut self, node: NodeId, parent: NodeId, before: Option<NodeId>);
    /// Structural: detaches a node (and with it its subtree). The native object stays alive.
    fn remove(&mut self, node: NodeId);
    fn update(&mut self, node: NodeId, patch: &PropsPatch);
    /// Retained backends only (see [`Backend::retains_frames`]); frames are relative to the
    /// parent and rounded by the backend to its pixel grid.
    fn set_frame(&mut self, node: NodeId, frame: Rect);
    fn surface(&mut self, node: NodeId, op: &SurfaceOp);
    /// Mirrors nd focus onto the platform (`makeFirstResponder` on AppKit). `None` clears it.
    fn focus(&mut self, node: Option<NodeId>);
    /// Drops the backend's strong reference. The node is detached or under a detached subtree.
    fn release(&mut self, node: NodeId);
    /// Closes the transaction. AppKit commits the `CATransaction`; GPUI marks the affected
    /// cached views dirty.
    fn commit(&mut self, window: WindowId);

    /// Query, valid outside transactions. Callers go through
    /// [`crate::transaction::MeasureCache`], which caches by props and constraint.
    fn measure(&mut self, node: NodeId, constraint: Constraint) -> Size;
    /// The size a native layout container (table row, split pane, toolbar item, scroll
    /// document) gives `node`, when `node` is the child of such a layout boundary.
    fn boundary_size(&self, node: NodeId) -> Option<Size>;
    /// True when `nd-ui` lays out with Taffy and pushes frames (AppKit, tests); false when the
    /// backend lays out itself (GPUI runs Taffy inside its own element tree).
    fn retains_frames(&self) -> bool;
}
