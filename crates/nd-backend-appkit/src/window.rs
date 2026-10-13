use std::cell::RefCell;
use std::rc::Rc;

use nd_ui::Keystroke;
use nd_ui::dispatch::KeyDispatch;
use objc2::rc::Retained;
use objc2::{DefinedClass, MainThreadOnly, Message, define_class, msg_send};
use objc2_app_kit::{NSBackingStoreType, NSEvent, NSEventType, NSResponder, NSWindow, NSWindowStyleMask};
use objc2_foundation::{MainThreadMarker, NSRect};

/// What an [`NdWindow`] calls into. Implemented by [`crate::AppKitDispatch`].
pub trait KeyRouter {
    /// A key-down, before AppKit's key equivalents and first responder see it.
    fn key_down(&self, keystroke: &Keystroke) -> KeyDispatch;
    /// The window's first responder changed, by nd's request or AppKit's own.
    fn responder_changed(&self, window: &NdWindow);
}

#[derive(Default)]
pub struct NdWindowIvars {
    router: RefCell<Option<Rc<dyn KeyRouter>>>,
    /// The last key-down routed to nd. AppKit offers one key-down to both
    /// `performKeyEquivalent:` and `sendEvent:`; nd must see it once.
    last_key: RefCell<Option<Retained<NSEvent>>>,
}

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
            if event.r#type() == NSEventType::KeyDown && self.route_key(event) {
                return;
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
            if changed && let Some(router) = self.router() {
                router.responder_changed(self);
            }
            changed
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
        this
    }

    pub fn set_router(&self, router: Option<Rc<dyn KeyRouter>>) {
        *self.ivars().router.borrow_mut() = router;
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
        router.key_down(&keystroke) != KeyDispatch::Unhandled
    }
}
