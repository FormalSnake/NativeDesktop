//! Widgets drawn by crates outside the backend (the terminal, later the code editor): a crate
//! registers a [`CustomWidget`] for its widget kind and builds that node's GPUI element itself,
//! with the theme in effect, the node's emitter and the paint recorder the appearance audit reads.

use std::any::TypeId;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use nd_core::{NodeId, Rect, WindowId};
use nd_gpui::gpui::{self, AnyElement, Hsla};
use nd_theme::Theme;
use nd_ui::backend::Props;
use nd_ui::events::Emit;

use crate::render::Build;

/// Builds the GPUI element of one widget kind's nodes.
pub trait CustomWidget: 'static {
    fn element(&self, cx: &CustomCx<'_>, window: &mut gpui::Window) -> AnyElement;

    /// The node is gone; drop what was kept for it.
    fn released(&self, _node: NodeId) {}
}

thread_local! {
    static WIDGETS: RefCell<HashMap<TypeId, Rc<dyn CustomWidget>>> = RefCell::default();
}

/// Draws nodes of widget kind `W` with `widget`. Registering a kind again replaces it.
pub fn register<W: 'static>(widget: impl CustomWidget) {
    WIDGETS.with(|w| w.borrow_mut().insert(TypeId::of::<W>(), Rc::new(widget)));
}

pub(crate) fn lookup(kind: TypeId) -> Option<Rc<dyn CustomWidget>> {
    WIDGETS.with(|w| w.borrow().get(&kind).cloned())
}

/// Reports GPUI focus a custom widget took to nd; see [`CustomCx::focus_reporter`].
#[derive(Clone)]
pub struct FocusReporter {
    platform_focus: crate::backend::PlatformFocus,
    window: WindowId,
    node: NodeId,
    wake: Rc<dyn Fn(&mut gpui::App)>,
}

impl FocusReporter {
    /// The node took GPUI focus: nd focus moves to it on the host's next pump.
    pub fn focused(&self, cx: &mut gpui::App) {
        self.platform_focus.borrow_mut().push((self.window, Some(self.node)));
        (self.wake)(cx);
    }
}

/// What a custom widget builds its node with.
pub struct CustomCx<'a> {
    build: &'a Build<'a>,
    node: NodeId,
    props: &'a Props,
}

impl<'a> CustomCx<'a> {
    pub(crate) fn new(build: &'a Build<'a>, node: NodeId, props: &'a Props) -> Self {
        Self { build, node, props }
    }

    /// The backend the node lives in.
    pub fn backend(&self) -> &crate::GpuiBackend {
        self.build.backend
    }

    pub fn node(&self) -> NodeId {
        self.node
    }

    pub fn window_id(&self) -> WindowId {
        self.build.window_id
    }

    pub fn props(&self) -> &Props {
        self.props
    }

    /// The theme in effect at the node, subtree overrides included.
    pub fn theme(&self) -> Rc<Theme> {
        self.build.theme()
    }

    /// Queues the node's events for the app; call [`Self::waker`]'s function after.
    pub fn emitter(&self) -> Emit {
        self.build.backend.state.borrow().events.emitter(self.node)
    }

    /// Schedules the host's pump, which delivers queued events. Call with GPUI's app, from an
    /// element's prepaint or paint or a task.
    pub fn waker(&self) -> Rc<dyn Fn(&mut gpui::App)> {
        self.build.wake.clone()
    }

    /// Whether nd moved focus to this node (a focus traversal, an app's focus call) since the
    /// last build. Consumes the request: the widget gives its own GPUI focus handle focus when
    /// it next prepaints; unconsumed, the window drops GPUI focus.
    pub fn take_focus_request(&self) -> bool {
        let window = self.build.window_id;
        let mut requests = self.build.backend.focus_requests.borrow_mut();
        let ours = requests.get(&window) == Some(&Some(self.node));
        if ours {
            requests.remove(&window);
        }
        ours
    }

    /// Tells nd when the user focuses the node itself (a click into it), so nd focus and the
    /// key bindings of its focus path follow GPUI focus.
    pub fn focus_reporter(&self) -> FocusReporter {
        FocusReporter {
            platform_focus: self.build.backend.platform_focus.clone(),
            window: self.build.window_id,
            node: self.node,
            wake: self.build.wake.clone(),
        }
    }

    /// Where the node was painted last frame, in window coordinates.
    pub fn last_bounds(&self) -> Option<Rect> {
        self.build.backend.bounds.borrow().get(&self.node).copied()
    }

    /// Records a text run the node paints, for the appearance audit: its colour, what it sits on
    /// (fills under it within the node, innermost last) and its size and CSS weight.
    pub fn record_text(&self, label: &str, color: Hsla, under: &[Hsla], size: f32, weight: f32) {
        self.build.record_marks(
            self.node,
            vec![crate::nav::Mark { label: label.into(), color, under: under.to_vec(), text: Some((size, weight)) }],
        );
    }

    /// Records a fill the node paints, for the appearance audit.
    pub fn record_fill(&self, label: &str, color: Hsla, under: &[Hsla]) {
        self.build.record_marks(
            self.node,
            vec![crate::nav::Mark { label: label.into(), color, under: under.to_vec(), text: None }],
        );
    }
}
