//! The AppKit side of the backend seam (ARCHITECTURE.md 3.15). Retained nodes own NSViews; one
//! transaction is one `CATransaction` with implicit animations off.

use std::any::TypeId;
use std::collections::HashMap;

use nd_core::{NodeId, Rect, Size, WindowId};
use nd_ui::backend::{AvailableSpace, Backend, Constraint, NodeKind, Props, PropsPatch, SurfaceOp};
use objc2::rc::Retained;
use objc2_app_kit::{NSView, NSWindowOrderingMode};
use objc2_foundation::{MainThreadMarker, NSAlignmentOptions};
use objc2_quartz_core::CATransaction;

use crate::NdWindow;
use crate::button::NativeButtonView;
use crate::events::EventQueue;
use crate::props::{ButtonProps, NativeButton, TextProps, ViewProps};
use crate::views::{NdLayerView, NdRootView, NdTextView, ns_rect};

enum Native {
    Root(Retained<NdRootView>),
    Layer(Retained<NdLayerView>),
    Text(Retained<NdTextView>),
    Button(NativeButtonView),
    /// A view nd did not create (a toolbar item's field until toolbars are nodes), registered
    /// so focus and the key view loop can name it.
    External(Retained<NSView>),
}

impl Native {
    fn view(&self) -> &NSView {
        match self {
            Native::Root(v) => v,
            Native::Layer(v) => v,
            Native::Text(v) => v,
            Native::Button(b) => &b.button,
            Native::External(v) => v,
        }
    }
}

struct WindowRecord {
    window: Retained<NdWindow>,
    root: Retained<NdRootView>,
}

pub struct AppKitBackend {
    mtm: MainThreadMarker,
    nodes: HashMap<NodeId, Native>,
    windows: HashMap<WindowId, WindowRecord>,
    open: Option<WindowId>,
    queue: EventQueue,
}

fn props<T: 'static>(props: &Props, kind: &str) -> T
where
    T: Clone,
{
    props.downcast_ref::<T>().cloned().unwrap_or_else(|| panic!("{kind} node needs {} props", std::any::type_name::<T>()))
}

impl AppKitBackend {
    pub fn new(mtm: MainThreadMarker) -> Self {
        Self { mtm, nodes: HashMap::new(), windows: HashMap::new(), open: None, queue: EventQueue::default() }
    }

    /// Where native callbacks land. Drain it from its waker, outside any transaction.
    pub fn events(&self) -> EventQueue {
        self.queue.clone()
    }

    pub(crate) fn add_window(&mut self, id: WindowId, window: &NdWindow, root: &NdRootView) {
        use objc2::Message;
        self.windows.insert(id, WindowRecord { window: window.retain(), root: root.retain() });
    }

    pub(crate) fn remove_window(&mut self, id: WindowId) {
        self.windows.remove(&id);
    }

    pub fn register_external(&mut self, node: NodeId, view: &NSView) {
        use objc2::Message;
        self.nodes.insert(node, Native::External(view.retain()));
    }

    pub fn view(&self, node: NodeId) -> Option<Retained<NSView>> {
        use objc2::Message;
        self.nodes.get(&node).map(|n| n.view().retain())
    }

    /// The node whose view is `view`, if nd created or registered it.
    pub fn node_for_view(&self, view: *const NSView) -> Option<NodeId> {
        self.nodes.iter().find(|(_, n)| std::ptr::eq(n.view(), view)).map(|(id, _)| *id)
    }

    fn window(&self) -> &WindowRecord {
        let id = self.open.expect("seam ops arrive inside begin/commit");
        self.windows.get(&id).unwrap_or_else(|| panic!("window {id:?} has no host"))
    }

    fn native(&self, node: NodeId) -> &Native {
        self.nodes.get(&node).unwrap_or_else(|| panic!("{node:?} is not a live node"))
    }

    fn attach(&self, parent: NodeId, node: NodeId, before: Option<NodeId>) {
        let parent = self.native(parent).view();
        let view = self.native(node).view();
        match before.map(|b| self.native(b).view()) {
            Some(sibling) => parent.addSubview_positioned_relativeTo(view, NSWindowOrderingMode::Below, Some(sibling)),
            None => parent.addSubview(view),
        }
    }
}

impl Backend for AppKitBackend {
    fn begin(&mut self, window: WindowId) {
        assert!(self.open.is_none(), "transactions do not nest");
        self.open = Some(window);
        self.queue.set_programmatic(true);
        CATransaction::begin();
        CATransaction::setDisableActions(true);
    }

    fn create(&mut self, node: NodeId, kind: NodeKind, p: &Props) {
        let mtm = self.mtm;
        let native = match kind {
            NodeKind::Root => Native::Root(self.window().root.clone()),
            NodeKind::View => Native::Layer(NdLayerView::new(mtm, &props::<ViewProps>(p, "View"))),
            NodeKind::Text => Native::Text(NdTextView::new(mtm, &props::<TextProps>(p, "Text"))),
            NodeKind::Widget(w) if w.type_id == TypeId::of::<NativeButton>() => {
                Native::Button(NativeButtonView::new(mtm, node, self.queue.clone(), &props::<ButtonProps>(p, "Button")))
            }
            NodeKind::Canvas | NodeKind::Image | NodeKind::Widget(_) => {
                panic!("the AppKit backend has no {kind:?} yet")
            }
        };
        self.nodes.insert(node, native);
    }

    fn insert(&mut self, parent: NodeId, node: NodeId, before: Option<NodeId>) {
        self.attach(parent, node, before);
    }

    fn move_node(&mut self, node: NodeId, parent: NodeId, before: Option<NodeId>) {
        // addSubview re-parents in place: the view keeps its layer, state and first responder.
        self.attach(parent, node, before);
    }

    fn remove(&mut self, node: NodeId) {
        self.native(node).view().removeFromSuperview();
    }

    fn update(&mut self, node: NodeId, patch: &PropsPatch) {
        match self.native(node) {
            Native::Layer(v) => v.set_props(&props::<ViewProps>(&patch.new, "View")),
            Native::Text(v) => v.set_props(&props::<TextProps>(&patch.new, "Text")),
            Native::Button(b) => {
                b.update(&props::<ButtonProps>(&patch.old, "Button"), &props::<ButtonProps>(&patch.new, "Button"))
            }
            Native::Root(_) | Native::External(_) => {}
        }
    }

    fn set_frame(&mut self, node: NodeId, frame: Rect) {
        let view = self.native(node).view();
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
        let window = self.window().window.clone();
        let view = node.and_then(|n| self.nodes.get(&n)).map(|n| n.view());
        window.make_first_responder_from_nd(view);
    }

    fn release(&mut self, node: NodeId) {
        self.nodes.remove(&node);
    }

    fn commit(&mut self, window: WindowId) {
        assert_eq!(self.open, Some(window), "commit closes the open transaction");
        CATransaction::commit();
        self.queue.set_programmatic(false);
        self.open = None;
    }

    fn measure(&mut self, node: NodeId, constraint: Constraint) -> Size {
        let width = match constraint.width {
            AvailableSpace::Definite(w) => Some(w),
            AvailableSpace::MinContent | AvailableSpace::MaxContent => None,
        };
        match self.native(node) {
            Native::Text(v) => v.measure(width),
            Native::Button(b) => b.measure(),
            Native::Root(_) | Native::Layer(_) | Native::External(_) => Size::default(),
        }
    }

    fn boundary_size(&self, _node: NodeId) -> Option<Size> {
        None
    }

    fn retains_frames(&self) -> bool {
        true
    }
}
