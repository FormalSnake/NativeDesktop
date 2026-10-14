use std::cell::{Cell, RefCell};
use std::rc::Rc;

use nd_ui::Keystroke;
use nd_ui::dispatch::KeyDispatch;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, ProtocolObject};
use objc2::{ClassType, DefinedClass, MainThreadOnly, Message, define_class, msg_send};
use objc2_app_kit::{
    NSBackingStoreType, NSDragOperation, NSDraggingInfo, NSEvent, NSEventType, NSPasteboardTypeFileURL, NSResponder,
    NSView, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{MainThreadMarker, NSArray, NSRect, NSURL};

fn drag_operation(info: &ProtocolObject<dyn NSDraggingInfo>) -> NSDragOperation {
    if dropped_files(info).is_empty() { NSDragOperation::None } else { NSDragOperation::Copy }
}

/// The file paths on a drag's pasteboard.
fn dropped_files(info: &ProtocolObject<dyn NSDraggingInfo>) -> Vec<std::path::PathBuf> {
    let classes: Retained<NSArray<AnyClass>> = NSArray::from_slice(&[NSURL::class()]);
    // SAFETY: NSURL is a pasteboard-readable class; no options.
    let Some(objects) = (unsafe { info.draggingPasteboard().readObjectsForClasses_options(&classes, None) }) else {
        return Vec::new();
    };
    objects
        .iter()
        .filter_map(|o| o.downcast::<NSURL>().ok())
        .filter(|u| u.isFileURL())
        .filter_map(|u| u.path())
        .map(|p| std::path::PathBuf::from(p.to_string()))
        .collect()
}

/// What an [`NdWindow`] calls into. Implemented by [`crate::AppKitDispatch`].
pub trait KeyRouter {
    /// A key-down, before AppKit's key equivalents and first responder see it.
    fn key_down(&self, keystroke: &Keystroke) -> KeyDispatch;
    /// The window's first responder changed, by nd's request or AppKit's own.
    fn responder_changed(&self, window: &NdWindow);
    /// AppKit's per-event `update` pass: the window may have re-linked views behind nd's back.
    fn window_updated(&self, _window: &NdWindow) {}
    /// A primary press anywhere in the window: focus rings hide until the keyboard moves focus.
    fn pointer_down(&self) {}
    /// Files dropped onto the window, at `at` in window points from the top left.
    fn files_dropped(&self, _paths: Vec<std::path::PathBuf>, _at: (f32, f32)) {}
}

#[derive(Default)]
pub struct NdWindowIvars {
    router: RefCell<Option<Rc<dyn KeyRouter>>>,
    /// The last key-down routed to nd. AppKit offers one key-down to both
    /// `performKeyEquivalent:` and `sendEvent:`; nd must see it once.
    last_key: RefCell<Option<Retained<NSEvent>>>,
    /// Set while nd's own focus op moves the first responder, so it is not reported back.
    nd_focus: Cell<bool>,
    /// A framework panel holding text focus outside nd's tree (the command palette) claims the
    /// keys it edits with before nd's bindings see them.
    key_capture: RefCell<Option<KeyCapture>>,
}

/// Says whether a framework panel takes a keystroke before nd dispatch.
pub(crate) type KeyCapture = Rc<dyn Fn(&Keystroke) -> bool>;

define_class!(
    // SAFETY: NSWindow has no subclassing requirements beyond calling super in the overrides
    // below, and this type does not implement Drop.
    #[unsafe(super(NSWindow, NSResponder))]
    #[thread_kind = MainThreadOnly]
    #[ivars = NdWindowIvars]
    pub struct NdWindow;

    impl NdWindow {
        #[unsafe(method(sendEvent:))]
        fn send_event(&self, event: &NSEvent) {
            crate::diag::record_event(self, event);
            if event.r#type() == NSEventType::KeyDown && self.route_key(event) {
                return;
            }
            if event.r#type() == NSEventType::LeftMouseDown && let Some(router) = self.router() {
                router.pointer_down();
            }
            unsafe { msg_send![super(self), sendEvent: event] }
        }

        #[unsafe(method(performKeyEquivalent:))]
        fn perform_key_equivalent(&self, event: &NSEvent) -> bool {
            if event.r#type() == NSEventType::KeyDown && self.route_key(event) {
                true
            } else {
                unsafe { msg_send![super(self), performKeyEquivalent: event] }
            }
        }

        #[unsafe(method(makeFirstResponder:))]
        fn make_first_responder(&self, responder: Option<&NSResponder>) -> bool {
            let changed: bool = unsafe { msg_send![super(self), makeFirstResponder: responder] };
            if changed && !self.ivars().nd_focus.get() && let Some(router) = self.router() {
                router.responder_changed(self);
            }
            changed
        }

        #[unsafe(method(draggingEntered:))]
        fn dragging_entered(&self, sender: &ProtocolObject<dyn NSDraggingInfo>) -> NSDragOperation {
            drag_operation(sender)
        }

        #[unsafe(method(draggingUpdated:))]
        fn dragging_updated(&self, sender: &ProtocolObject<dyn NSDraggingInfo>) -> NSDragOperation {
            drag_operation(sender)
        }

        #[unsafe(method(performDragOperation:))]
        fn perform_drag_operation(&self, sender: &ProtocolObject<dyn NSDraggingInfo>) -> bool {
            self.drop_files(sender)
        }

        #[unsafe(method(update))]
        fn update(&self) {
            let _: () = unsafe { msg_send![super(self), update] };
            if let Some(router) = self.router() {
                router.window_updated(self);
            }
        }
    }
);

impl NdWindow {
    pub fn new(mtm: MainThreadMarker, content_rect: NSRect, style: NSWindowStyleMask) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(NdWindowIvars::default());
        let this: Retained<Self> = unsafe {
            msg_send![
                super(this),
                initWithContentRect: content_rect,
                styleMask: style,
                backing: NSBackingStoreType::Buffered,
                defer: false
            ]
        };
        // The Rust side owns the window; AppKit releasing it on close would double free.
        unsafe { this.setReleasedWhenClosed(false) };
        // Files dropped anywhere no view claims reach `files_dropped`.
        // SAFETY: a framework constant.
        this.registerForDraggedTypes(&NSArray::from_slice(&[unsafe { NSPasteboardTypeFileURL }]));
        this
    }

    pub fn set_router(&self, router: Option<Rc<dyn KeyRouter>>) {
        *self.ivars().router.borrow_mut() = router;
    }

    pub(crate) fn set_key_capture(&self, capture: Option<KeyCapture>) {
        *self.ivars().key_capture.borrow_mut() = capture;
    }

    /// The backend's focus op: moves AppKit's first responder to `view` (the window itself for
    /// `None`) without reporting it back to nd as a platform focus change.
    pub(crate) fn make_first_responder_from_nd(&self, view: Option<&NSView>) {
        self.ivars().nd_focus.set(true);
        self.makeFirstResponder(view.map(|v| &**v));
        self.ivars().nd_focus.set(false);
    }

    /// Hands a drop's files to the router; false when the drag carried none.
    fn drop_files(&self, sender: &ProtocolObject<dyn NSDraggingInfo>) -> bool {
        let paths = dropped_files(sender);
        let Some(router) = self.router().filter(|_| !paths.is_empty()) else { return false };
        let at = sender.draggingLocation();
        let height = self.contentView().map_or(0., |v| v.frame().size.height);
        router.files_dropped(paths, (at.x as f32, (height - at.y) as f32));
        true
    }

    fn router(&self) -> Option<Rc<dyn KeyRouter>> {
        self.ivars().router.borrow().clone()
    }

    /// Offers a key-down to nd once. True when nd consumed it.
    fn route_key(&self, event: &NSEvent) -> bool {
        {
            let mut last = self.ivars().last_key.borrow_mut();
            if last.as_deref().is_some_and(|l| std::ptr::eq(l, event)) {
                return false;
            }
            *last = Some(event.retain());
        }
        let (Some(router), Some(keystroke)) = (self.router(), crate::keystroke(event)) else {
            return false;
        };
        let capture = self.ivars().key_capture.borrow().clone();
        if capture.is_some_and(|c| c(&keystroke)) {
            return false;
        }
        router.key_down(&keystroke) != KeyDispatch::Unhandled
    }
}
