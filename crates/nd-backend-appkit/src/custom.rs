//! Widgets whose AppKit view lives in a crate outside the backend (the terminal's Metal view):
//! a crate registers a [`CustomWidget`] for its widget kind and the backend keeps the view it
//! creates like any other node's, framing it, focusing it and reading its paints for the
//! appearance audit.

use std::any::TypeId;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use nd_core::{NodeId, Size, WindowId};
use nd_theme::Theme;
use nd_ui::backend::Props;
use nd_ui::events::Emit;
use nd_ui::prelude::Hsla;
use objc2_app_kit::NSView;
use objc2_foundation::MainThreadMarker;

/// Creates the native object of one widget kind's nodes.
pub trait CustomWidget: 'static {
    fn create(&self, cx: &CustomCx<'_>) -> Box<dyn CustomNative>;
}

/// One node's native object.
pub trait CustomNative {
    /// The view the parent holds; the backend sets its frame.
    fn view(&self) -> &NSView;

    /// The node's props changed (`cx.props()` holds the new ones).
    fn update(&mut self, cx: &CustomCx<'_>);

    /// The theme in effect at the node changed.
    fn set_theme(&mut self, theme: &Theme);

    /// The natural size for `width` (`None` when unconstrained), for nodes that size
    /// themselves; zero takes the size layout gives.
    fn measure(&self, _width: Option<f32>) -> Size {
        Size::default()
    }

    /// What the view painted last, for the appearance audit.
    fn paints(&self) -> Vec<CustomPaint> {
        Vec::new()
    }
}

/// A text run or fill a custom view painted: its colour and the fills under it inside the
/// view, innermost last.
#[derive(Clone, Debug, PartialEq)]
pub struct CustomPaint {
    pub label: String,
    pub color: Hsla,
    pub under: Vec<Hsla>,
    /// Size in points and CSS weight for text, `None` for a fill.
    pub text: Option<(f32, f32)>,
}

thread_local! {
    static WIDGETS: RefCell<HashMap<TypeId, Rc<dyn CustomWidget>>> = RefCell::default();
}

/// Creates nodes of widget kind `W` with `widget`. Registering a kind again replaces it.
pub fn register<W: 'static>(widget: impl CustomWidget) {
    WIDGETS.with(|w| w.borrow_mut().insert(TypeId::of::<W>(), Rc::new(widget)));
}

pub(crate) fn lookup(kind: TypeId) -> Option<Rc<dyn CustomWidget>> {
    WIDGETS.with(|w| w.borrow().get(&kind).cloned())
}

/// Wakes the host so it delivers the events a custom view queued, on a later run loop turn.
#[derive(Clone)]
pub struct Waker(pub(crate) crate::events::Wake);

impl Waker {
    pub fn wake(&self) {
        self.0.wake();
    }
}

/// What a custom widget creates and updates its node with.
pub struct CustomCx<'a> {
    pub(crate) mtm: MainThreadMarker,
    pub(crate) node: NodeId,
    pub(crate) window: WindowId,
    pub(crate) props: &'a Props,
    pub(crate) emit: Emit,
    pub(crate) wake: Waker,
    pub(crate) theme: Theme,
}

impl CustomCx<'_> {
    pub fn mtm(&self) -> MainThreadMarker {
        self.mtm
    }

    pub fn node(&self) -> NodeId {
        self.node
    }

    pub fn window(&self) -> WindowId {
        self.window
    }

    pub fn props(&self) -> &Props {
        self.props
    }

    /// Queues the node's events for the app; wake the [`Self::waker`] after.
    pub fn emitter(&self) -> Emit {
        self.emit.clone()
    }

    pub fn waker(&self) -> Waker {
        self.wake.clone()
    }

    /// The theme in effect at the node.
    pub fn theme(&self) -> &Theme {
        &self.theme
    }
}
