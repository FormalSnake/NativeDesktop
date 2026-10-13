//! The AppKit side of the backend seam (ARCHITECTURE.md 3.15). Retained nodes own NSViews; one
//! transaction is one `CATransaction` with implicit animations off.

use std::any::TypeId;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use nd_core::{NodeId, Rect, Size, WindowId};
use nd_ui::backend::{AvailableSpace, Backend, Constraint, NodeKind, Props, PropsPatch, SurfaceOp};
use nd_ui::events::EventQueue;
use nd_ui::{Button, ButtonProps, DivProps, RootProps, WindowControls, WindowControlsProps};
use objc2::Message;
use objc2::rc::Retained;
use objc2_app_kit::{NSView, NSWindowOrderingMode};
use objc2_foundation::{MainThreadMarker, NSAlignmentOptions};
use objc2_quartz_core::CATransaction;

use crate::NdWindow;
use crate::button::NativeButtonView;
use crate::events::Wake;
use crate::look::{TextLook, ViewLook};
use crate::traffic_lights::TrafficLights;
use crate::views::{NdLayerView, NdRootView, NdTextView, ns_rect};

enum Native {
    Root(Retained<NdLayerView>),
    Layer(Retained<NdLayerView>),
    Text(Retained<NdTextView>),
    Button(NativeButtonView),
    /// A `window_controls()` node: no view, it places the window's own buttons.
    WindowControls(WindowId),
    /// A view nd did not create (a toolbar item's field until toolbars are nodes), registered
    /// so focus and the key view loop can name it.
    External(Retained<NSView>),
}

impl Native {
    fn view(&self) -> Option<&NSView> {
        match self {
            Native::Root(v) | Native::Layer(v) => Some(v),
            Native::Text(v) => Some(v),
            Native::Button(b) => Some(&b.button),
            Native::External(v) => Some(v),
            Native::WindowControls(_) => None,
        }
    }
}

pub(crate) struct WindowRecord {
    pub window: Retained<NdWindow>,
    pub root: Retained<NdRootView>,
    pub lights: Rc<TrafficLights>,
}

struct State {
    mtm: MainThreadMarker,
    nodes: HashMap<NodeId, Native>,
    windows: HashMap<WindowId, WindowRecord>,
    open: Option<WindowId>,
    events: EventQueue,
    wake: Wake,
    expose: bool,
}

/// The AppKit backend. A cheap handle: `nd-ui`'s `App` owns one clone as its `Box<dyn Backend>`,
/// window hosts and the dispatcher keep others.
#[derive(Clone)]
pub struct AppKitBackend(Rc<RefCell<State>>);

fn props<'a, T: 'static>(props: &'a Props, kind: &str) -> &'a T {
    props.downcast_ref::<T>().unwrap_or_else(|| panic!("{kind} node needs {} props", std::any::type_name::<T>()))
}

impl AppKitBackend {
    pub fn new(mtm: MainThreadMarker) -> Self {
        Self(Rc::new(RefCell::new(State {
            mtm,
            nodes: HashMap::new(),
            windows: HashMap::new(),
            open: None,
            events: EventQueue::default(),
            wake: Wake::default(),
            expose: std::env::var_os("ND_DEBUG_EXPOSE").is_some_and(|v| v == "1"),
        })))
    }

    /// Runs `callback` on the main run loop turn after a native callback queued an event.
    pub fn on_wake(&self, callback: impl Fn() + 'static) {
        self.0.borrow().wake.set_callback(callback);
    }

    pub(crate) fn add_window(&self, id: WindowId, record: WindowRecord) {
        self.0.borrow_mut().windows.insert(id, record);
    }

    pub(crate) fn remove_window(&self, id: WindowId) {
        if let Ok(mut state) = self.0.try_borrow_mut() {
            state.windows.remove(&id);
        }
    }

    pub fn register_external(&self, node: NodeId, view: &NSView) {
        self.0.borrow_mut().nodes.insert(node, Native::External(view.retain()));
    }

    pub fn view(&self, node: NodeId) -> Option<Retained<NSView>> {
        self.0.borrow().nodes.get(&node)?.view().map(|v| v.retain())
    }

    /// The node whose view is `view`, if nd created or registered it. `None` while a
    /// transaction is applying.
    pub fn node_for_view(&self, view: *const NSView) -> Option<NodeId> {
        let state = self.0.try_borrow().ok()?;
        state.nodes.iter().find(|(_, n)| n.view().is_some_and(|v| std::ptr::eq(v, view))).map(|(id, _)| *id)
    }
}

impl State {
    fn window(&self) -> &WindowRecord {
        let id = self.open.expect("seam ops arrive inside begin/commit");
        self.windows.get(&id).unwrap_or_else(|| panic!("window {id:?} has no host"))
    }

    fn native(&self, node: NodeId) -> &Native {
        self.nodes.get(&node).unwrap_or_else(|| panic!("{node:?} is not a live node"))
    }

    fn view(&self, node: NodeId) -> Option<&NSView> {
        self.native(node).view()
    }

    fn attach(&self, parent: NodeId, node: NodeId, before: Option<NodeId>) {
        let (Some(parent), Some(view)) = (self.view(parent), self.view(node)) else { return };
        match before.and_then(|b| self.view(b)) {
            Some(sibling) => parent.addSubview_positioned_relativeTo(view, NSWindowOrderingMode::Below, Some(sibling)),
            None => parent.addSubview(view),
        }
    }

    fn place_lights(&self, window: WindowId, p: &WindowControlsProps) {
        if let Some(record) = self.windows.get(&window) {
            record.lights.set_at(&record.window, p.at);
        }
    }
}

impl Backend for AppKitBackend {
    fn set_event_queue(&mut self, events: EventQueue) {
        self.0.borrow_mut().events = events;
    }

    fn begin(&mut self, window: WindowId) {
        let mut state = self.0.borrow_mut();
        assert!(state.open.is_none(), "transactions do not nest");
        state.open = Some(window);
        CATransaction::begin();
        CATransaction::setDisableActions(true);
    }

    fn create(&mut self, node: NodeId, kind: NodeKind, p: &Props) {
        let mut state = self.0.borrow_mut();
        let mtm = state.mtm;
        let native = match kind {
            NodeKind::Root => {
                let _: &RootProps = props(p, "Root");
                Native::Root(state.window().root.content().retain())
            }
            NodeKind::View => Native::Layer(NdLayerView::new(
                mtm,
                &ViewLook::from_style(&props::<DivProps>(p, "View").style, state.expose),
            )),
            NodeKind::Text => Native::Text(NdTextView::new(mtm, &TextLook::from_props(props(p, "Text")))),
            NodeKind::Widget(w) if w.type_id == TypeId::of::<Button>() => {
                let emit = state.events.emitter(node);
                Native::Button(NativeButtonView::new(mtm, emit, state.wake.clone(), props(p, "Button")))
            }
            NodeKind::Widget(w) if w.type_id == TypeId::of::<WindowControls>() => {
                let window = state.open.expect("inside a transaction");
                state.place_lights(window, props(p, "WindowControls"));
                Native::WindowControls(window)
            }
            NodeKind::Canvas | NodeKind::Image | NodeKind::Widget(_) => {
                panic!("the AppKit backend has no {kind:?} yet")
            }
        };
        state.nodes.insert(node, native);
    }

    fn insert(&mut self, parent: NodeId, node: NodeId, before: Option<NodeId>) {
        self.0.borrow().attach(parent, node, before);
    }

    fn move_node(&mut self, node: NodeId, parent: NodeId, before: Option<NodeId>) {
        // addSubview re-parents in place: the view keeps its layer, state and first responder.
        self.0.borrow().attach(parent, node, before);
    }

    fn remove(&mut self, node: NodeId) {
        if let Some(view) = self.0.borrow().view(node) {
            view.removeFromSuperview();
        }
    }

    fn update(&mut self, node: NodeId, patch: &PropsPatch) {
        let state = self.0.borrow();
        let events = state.events.clone();
        events.programmatic(|| match state.native(node) {
            Native::Layer(v) => {
                v.set_props(&ViewLook::from_style(&props::<DivProps>(&patch.new, "View").style, state.expose))
            }
            Native::Text(v) => v.set_props(&TextLook::from_props(props(&patch.new, "Text"))),
            Native::Button(b) => b.update(props::<ButtonProps>(&patch.old, "Button"), props(&patch.new, "Button")),
            Native::WindowControls(window) => state.place_lights(*window, props(&patch.new, "WindowControls")),
            Native::Root(_) | Native::External(_) => {}
        });
    }

    fn set_frame(&mut self, node: NodeId, frame: Rect) {
        let state = self.0.borrow();
        let Some(view) = state.view(node) else { return };
        let rect = ns_rect(frame);
        // Rounded outward on the backing grid, so a label rounded down never loses its last glyph.
        let rect = match unsafe { view.superview() } {
            Some(parent) => parent.backingAlignedRect_options(rect, NSAlignmentOptions::AlignAllEdgesOutward),
            None => rect,
        };
        if view.frame() != rect {
            view.setFrame(rect);
        }
    }

    fn surface(&mut self, _node: NodeId, _op: &SurfaceOp) {}

    fn focus(&mut self, node: Option<NodeId>) {
        let state = self.0.borrow();
        let window = state.window().window.clone();
        let view = node.and_then(|n| state.nodes.get(&n)).and_then(Native::view).map(|v| v.retain());
        drop(state);
        window.make_first_responder_from_nd(view.as_deref());
    }

    fn release(&mut self, node: NodeId) {
        let native = self.0.borrow_mut().nodes.remove(&node);
        drop(native);
    }

    fn commit(&mut self, window: WindowId) {
        let mut state = self.0.borrow_mut();
        assert_eq!(state.open, Some(window), "commit closes the open transaction");
        CATransaction::commit();
        state.open = None;
    }

    fn measure(&mut self, node: NodeId, constraint: Constraint) -> Size {
        let width = match constraint.width {
            AvailableSpace::Definite(w) => Some(w),
            AvailableSpace::MinContent | AvailableSpace::MaxContent => None,
        };
        match self.0.borrow().native(node) {
            Native::Text(v) => v.measure(width),
            Native::Button(b) => b.measure(),
            Native::Root(_) | Native::Layer(_) | Native::External(_) | Native::WindowControls(_) => Size::default(),
        }
    }

    fn boundary_size(&self, _node: NodeId) -> Option<Size> {
        None
    }

    fn retains_frames(&self) -> bool {
        true
    }
}
