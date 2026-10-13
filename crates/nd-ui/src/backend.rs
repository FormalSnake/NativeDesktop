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
//! - Main thread only. A backend never calls back into `nd-ui`; native callbacks push onto the
//!   [`EventQueue`] it was given, which `nd-ui` drains after the transaction (3.13).
//! - Measuring happens inside the transaction, after updates and before frames, so a node
//!   created in this frame can be measured in the frame that lays it out.
//! - During live resize the whole transaction runs synchronously inside the OS resize pass.

use std::any::{Any, TypeId};
use std::fmt;
use std::rc::Rc;

use nd_core::{NodeId, Rect, Size, WindowId};

use crate::events::EventQueue;

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

type SemanticsFn = fn(&dyn AnyProps) -> Option<crate::semantics::Semantics>;
type RespondFn = fn(&dyn AnyProps, &crate::semantics::SemanticAction) -> Option<crate::semantics::Response>;

/// Names a `NativeWidget` implementation by its Rust type. Equal kinds have the same type.
#[derive(Clone, Copy, Debug)]
pub struct WidgetKind {
    pub name: &'static str,
    pub type_id: TypeId,
    semantics: Option<SemanticsFn>,
    respond: Option<RespondFn>,
}

impl PartialEq for WidgetKind {
    fn eq(&self, other: &Self) -> bool {
        self.type_id == other.type_id
    }
}

impl Eq for WidgetKind {}

impl std::hash::Hash for WidgetKind {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.type_id.hash(state);
    }
}

impl WidgetKind {
    pub fn of<W: 'static>() -> Self {
        Self { name: std::any::type_name::<W>(), type_id: TypeId::of::<W>(), semantics: None, respond: None }
    }

    /// A kind whose props `P` describe the node to the semantic tree and answer semantic
    /// actions (see [`WidgetSemantics`](crate::semantics::WidgetSemantics)).
    pub fn described<W: 'static, P: crate::semantics::WidgetSemantics>() -> Self {
        fn semantics<P: crate::semantics::WidgetSemantics>(
            props: &dyn AnyProps,
        ) -> Option<crate::semantics::Semantics> {
            props.downcast_ref::<P>().map(P::semantics)
        }
        fn respond<P: crate::semantics::WidgetSemantics>(
            props: &dyn AnyProps,
            action: &crate::semantics::SemanticAction,
        ) -> Option<crate::semantics::Response> {
            props.downcast_ref::<P>().and_then(|p| p.respond(action))
        }
        Self { semantics: Some(semantics::<P>), respond: Some(respond::<P>), ..Self::of::<W>() }
    }

    /// What the props say about the node, for kinds built with [`WidgetKind::described`].
    pub fn semantics(&self, props: &dyn AnyProps) -> Option<crate::semantics::Semantics> {
        self.semantics.and_then(|f| f(props))
    }

    /// The event the control emits for `action`, for kinds built with [`WidgetKind::described`]
    /// whose props answer it. Backends that draw the widget themselves emit it.
    pub fn respond(
        &self,
        props: &dyn AnyProps,
        action: &crate::semantics::SemanticAction,
    ) -> Option<crate::semantics::Response> {
        self.respond.and_then(|f| f(props, action))
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

/// Edges a layout container keeps clear inside the box it gives a child, in points.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BoundaryInsets {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
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

pub trait Backend: Any {
    /// Called once, before any transaction. Native callbacks push here, never into app code.
    fn set_event_queue(&mut self, events: EventQueue);
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
    /// The node emitted an event since the last frame and its props may be unchanged (the app
    /// refused the change). A controlled native object re-applies its current props, so it
    /// shows the app's value again; a no-op where the backend draws from props each frame.
    fn reaffirm(&mut self, _node: NodeId) {}
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

    /// Query, valid for any created node, including inside a transaction once its updates are
    /// applied. Callers go through
    /// [`crate::transaction::MeasureCache`], which caches by props and constraint.
    fn measure(&mut self, node: NodeId, constraint: Constraint) -> Size;
    /// The size a native layout container (table row, split pane, toolbar item, scroll
    /// document) gives `node`, when `node` is the child of such a layout boundary.
    fn boundary_size(&self, node: NodeId) -> Option<Size>;
    /// Inside that box, the edges the container keeps clear of content (a split pane under
    /// the toolbar): added to the node's padding, so its background still fills the box.
    fn boundary_insets(&self, _node: NodeId) -> BoundaryInsets {
        BoundaryInsets::default()
    }
    /// True when `nd-ui` lays out with Taffy and pushes frames (AppKit, tests); false when the
    /// backend lays out itself (GPUI runs Taffy inside its own element tree).
    fn retains_frames(&self) -> bool;

    // Automation and the semantic tree (ARCHITECTURE.md 10.1): queries and commands, never inside a
    // transaction. Each has a default, so a backend without them still renders.

    /// What the native object adds to the node's semantics (live bounds, enabled, value).
    fn probe(&self, _node: NodeId) -> crate::semantics::Probe {
        crate::semantics::Probe::default()
    }

    /// The window as the OS has it; `None` where the backend has no OS window.
    fn window_state(&self, _window: WindowId) -> Option<crate::semantics::WindowState> {
        None
    }

    /// Carries out `action` through the native control. Events it causes go onto the event
    /// queue like any user action's.
    fn perform(
        &mut self,
        _node: NodeId,
        action: &crate::semantics::SemanticAction,
    ) -> Result<(), crate::semantics::ActionError> {
        Err(crate::semantics::ActionError::Unsupported(format!("{action:?}")))
    }

    /// Posts input events into the app's own event queue for `window`.
    fn synthesize(
        &mut self,
        _window: WindowId,
        _input: &crate::semantics::SyntheticInput,
    ) -> Result<(), crate::semantics::ActionError> {
        Err(crate::semantics::ActionError::InputUnsupported)
    }

    /// Plays `haptic` on the trackpad, timed to the next drawn frame. A no-op where the OS has no
    /// haptics (everything but macOS).
    fn haptic(&mut self, _haptic: nd_gesture::Haptic) {}

    /// Renders the window's content to a PNG.
    fn capture(&mut self, _window: WindowId) -> Result<crate::semantics::Capture, crate::semantics::ActionError> {
        Err(crate::semantics::ActionError::Unsupported("capture".into()))
    }
}
