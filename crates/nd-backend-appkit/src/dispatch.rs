use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use nd_ui::dispatch::{DispatchTree, KeyDispatch};
use nd_ui::{Keystroke, NodeId};
use objc2::Message;
use objc2::rc::{Retained, Weak};
use objc2::runtime::AnyObject;
use objc2_app_kit::{NSResponder, NSTextView, NSView};

use crate::{KeyRouter, NdWindow};

/// One window's nd dispatch on AppKit: owns the [`DispatchTree`] and the app context, maps nd
/// nodes to the views that take first responder for them, and keeps the two focus models in
/// step. Until the AppKit `Backend` lands, focus changes go straight to `makeFirstResponder`
/// instead of through a transaction's focus op.
pub struct AppKitDispatch<C> {
    tree: RefCell<DispatchTree<C>>,
    cx: RefCell<C>,
    views: RefCell<HashMap<NodeId, Retained<NSView>>>,
    window: Weak<NdWindow>,
    /// Set while nd itself moves the first responder, so the change is not reported back.
    applying: Cell<bool>,
}

impl<C: 'static> AppKitDispatch<C> {
    /// Installs a dispatcher as `window`'s key router.
    pub fn install(window: &NdWindow, tree: DispatchTree<C>, cx: C) -> Rc<Self> {
        let this = Rc::new(Self {
            tree: RefCell::new(tree),
            cx: RefCell::new(cx),
            views: RefCell::new(HashMap::new()),
            window: Weak::from_retained(&window.retain()),
            applying: Cell::new(false),
        });
        window.set_router(Some(this.clone()));
        this
    }

    /// The view AppKit makes first responder when nd focuses `node`.
    pub fn register_view(&self, node: NodeId, view: &NSView) {
        self.views.borrow_mut().insert(node, view.retain());
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
    /// the chain like any other registered view.
    pub fn sync_key_view_loop(&self) {
        let order = self.tree.borrow().focus_order();
        let views = self.views.borrow();
        let chain: Vec<&Retained<NSView>> = order.iter().filter_map(|n| views.get(n)).collect();
        for (i, view) in chain.iter().enumerate() {
            let next = chain[(i + 1) % chain.len()];
            unsafe { view.setNextKeyView(Some(next)) };
        }
        if let (Some(window), Some(first)) = (self.window.load(), chain.first()) {
            window.setInitialFirstResponder(Some(first));
        }
    }

    fn apply_focus(&self) {
        let Some(change) = self.tree.borrow_mut().take_focus_change() else { return };
        let Some(window) = self.window.load() else { return };
        let view = change.and_then(|n| self.views.borrow().get(&n).cloned());
        self.applying.set(true);
        window.makeFirstResponder(view.as_deref().map(|v| &**v));
        self.applying.set(false);
    }

    /// The nd node behind AppKit's first responder: `Some(None)` when the window itself holds
    /// it, `None` when it is a view nd does not know (nd focus then stays where it was).
    fn node_for_responder(&self, responder: Option<&NSResponder>) -> Option<Option<NodeId>> {
        let Some(responder) = responder else { return Some(None) };
        let views = self.views.borrow();
        let find = |object: *const AnyObject| {
            views.iter().find(|(_, v)| std::ptr::eq(Retained::as_ptr(v).cast(), object)).map(|(n, _)| *n)
        };
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
        if self.applying.get() {
            return;
        }
        let Some(node) = self.node_for_responder(window.firstResponder().as_deref()) else { return };
        // A listener that moved AppKit focus directly is still inside dispatch; nd learns of it
        // from its own focus request instead.
        if let Ok(mut tree) = self.tree.try_borrow_mut() {
            tree.platform_focused(node);
        }
    }
}
