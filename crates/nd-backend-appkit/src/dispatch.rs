use std::cell::{Cell, RefCell};
use std::rc::Rc;

use nd_core::WindowId;
use nd_ui::dispatch::{DispatchTree, KeyDispatch};
use nd_ui::transaction::{MeasureCache, Op, Transaction};
use nd_ui::{Keystroke, NodeId};
use objc2::Message;
use objc2::rc::{Retained, Weak};
use objc2::runtime::AnyObject;
use objc2_app_kit::{NSResponder, NSTextView, NSView};

use crate::{AppKitBackend, KeyRouter, NdWindow};

/// One window's nd dispatch on AppKit: owns the [`DispatchTree`] and the app context and keeps
/// the two focus models in step. nd focus reaches AppKit only as a transaction focus op through
/// the backend; AppKit-side changes (a click) are mirrored back into the tree.
pub struct AppKitDispatch<C> {
    tree: RefCell<DispatchTree<C>>,
    cx: RefCell<C>,
    backend: Rc<RefCell<AppKitBackend>>,
    window_id: WindowId,
    window: Weak<NdWindow>,
    /// The `nextKeyView` chain nd last wrote. AppKit re-links it on its own (installing an
    /// `NSSearchToolbarItem` rewrites the field's `nextKeyView` on macOS 27), so it is checked
    /// after every event.
    chain: RefCell<Vec<Retained<NSView>>>,
    resyncs: Cell<u32>,
}

impl<C: 'static> AppKitDispatch<C> {
    /// Installs a dispatcher as `window`'s key router.
    pub fn install(
        window: &NdWindow,
        window_id: WindowId,
        backend: &Rc<RefCell<AppKitBackend>>,
        tree: DispatchTree<C>,
        cx: C,
    ) -> Rc<Self> {
        let this = Rc::new(Self {
            tree: RefCell::new(tree),
            cx: RefCell::new(cx),
            backend: backend.clone(),
            window_id,
            window: Weak::from_retained(&window.retain()),
            chain: RefCell::new(Vec::new()),
            resyncs: Cell::new(0),
        });
        window.set_router(Some(this.clone()));
        this
    }

    /// Names a view nd did not create through the seam (a toolbar item's field), so focus ops
    /// and the key view loop can reach it.
    pub fn register_view(&self, node: NodeId, view: &NSView) {
        self.backend.borrow_mut().register_external(node, view);
    }

    pub fn tree(&self) -> std::cell::Ref<'_, DispatchTree<C>> {
        self.tree.borrow()
    }

    pub fn tree_mut(&self) -> std::cell::RefMut<'_, DispatchTree<C>> {
        self.tree.borrow_mut()
    }

    pub fn cx(&self) -> std::cell::Ref<'_, C> {
        self.cx.borrow()
    }

    /// Focuses `node` from nd and mirrors it onto AppKit.
    pub fn focus(&self, node: NodeId) -> bool {
        let focused = self.tree.borrow_mut().focus(node);
        self.apply_focus();
        focused
    }

    /// Rebuilds AppKit's `nextKeyView` chain from nd's focus order, so AppKit's own traversal
    /// (VoiceOver, `selectNextKeyView:`) agrees with nd's Tab. Toolbar item views are part of
    /// the chain like any other node's view.
    pub fn sync_key_view_loop(&self) {
        let order = self.tree.borrow().focus_order();
        let chain: Vec<Retained<NSView>> = {
            let backend = self.backend.borrow();
            order.iter().filter_map(|n| backend.view(*n)).collect()
        };
        for (i, view) in chain.iter().enumerate() {
            let next = &chain[(i + 1) % chain.len()];
            unsafe { view.setNextKeyView(Some(next)) };
        }
        if let (Some(window), Some(first)) = (self.window.load(), chain.first()) {
            window.setInitialFirstResponder(Some(first));
        }
        *self.chain.borrow_mut() = chain;
    }

    /// How often AppKit re-linked the chain and nd wrote it back.
    pub fn key_view_resyncs(&self) -> u32 {
        self.resyncs.get()
    }

    fn chain_intact(&self) -> bool {
        let chain = self.chain.borrow();
        chain.iter().enumerate().all(|(i, view)| {
            let next = &chain[(i + 1) % chain.len()];
            unsafe { view.nextKeyView() }.is_some_and(|n| std::ptr::eq(&*n, &**next))
        })
    }

    fn apply_focus(&self) {
        let Some(change) = self.tree.borrow_mut().take_focus_change() else { return };
        let mut tx = Transaction::new(self.window_id);
        tx.push(Op::Focus { node: change });
        // A focus-only transaction invalidates no measurement.
        tx.commit(&mut *self.backend.borrow_mut(), &mut MeasureCache::default());
    }

    /// The nd node behind AppKit's first responder: `Some(None)` when the window itself holds
    /// it, `None` when it is a view nd does not know (nd focus then stays where it was).
    fn node_for_responder(&self, responder: Option<&NSResponder>) -> Option<Option<NodeId>> {
        let Some(responder) = responder else { return Some(None) };
        let backend = self.backend.try_borrow().ok()?;
        let find = |object: *const AnyObject| backend.node_for_view(object.cast());
        let any: &AnyObject = responder;
        // A focused NSTextField hands first responder to the window's shared field editor,
        // whose delegate is the field.
        if let Some(text) = any.downcast_ref::<NSTextView>()
            && text.isFieldEditor()
            && let Some(delegate) = text.delegate()
        {
            let delegate: *const AnyObject = Retained::as_ptr(&delegate).cast();
            if let Some(node) = find(delegate) {
                return Some(Some(node));
            }
        }
        let mut view = any.downcast_ref::<NSView>().map(|v| v.retain());
        while let Some(v) = view {
            if let Some(node) = find(Retained::as_ptr(&v).cast()) {
                return Some(Some(node));
            }
            // SAFETY: reads the view hierarchy on the main thread; no view is mutated here.
            view = unsafe { v.superview() };
        }
        if any.downcast_ref::<NdWindow>().is_some() {
            return Some(None);
        }
        None
    }
}

impl<C: 'static> KeyRouter for AppKitDispatch<C> {
    fn key_down(&self, keystroke: &Keystroke) -> KeyDispatch {
        let result = {
            let mut tree = self.tree.borrow_mut();
            let mut cx = self.cx.borrow_mut();
            tree.dispatch_key(keystroke, &mut cx)
        };
        self.apply_focus();
        result
    }

    fn responder_changed(&self, window: &NdWindow) {
        let Some(node) = self.node_for_responder(window.firstResponder().as_deref()) else { return };
        // A listener that moved AppKit focus directly is still inside dispatch; nd learns of it
        // from its own focus request instead.
        if let Ok(mut tree) = self.tree.try_borrow_mut() {
            tree.platform_focused(node);
        }
    }

    fn window_updated(&self, _window: &NdWindow) {
        if !self.chain.borrow().is_empty() && !self.chain_intact() {
            self.resyncs.set(self.resyncs.get() + 1);
            self.sync_key_view_loop();
        }
    }
}
