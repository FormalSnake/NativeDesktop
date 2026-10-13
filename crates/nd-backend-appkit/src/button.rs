//! The native push button: AppKit's own `NSButton`, its action routed into the event queue.

use nd_core::Size;
use nd_ui::events::Emit;
use nd_ui::{ButtonProps, ClickEvent};
use objc2::rc::Retained;
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{NSBezelStyle, NSButton};
use objc2_foundation::{MainThreadMarker, NSObject, NSObjectProtocol, NSString};

use crate::events::Wake;

pub struct TargetIvars {
    emit: Emit,
    wake: Wake,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements and this type does not implement Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = TargetIvars]
    pub struct NdButtonTarget;

    unsafe impl NSObjectProtocol for NdButtonTarget {}

    impl NdButtonTarget {
        #[unsafe(method(press:))]
        fn press(&self, _sender: &NSButton) {
            self.ivars().emit.emit(ClickEvent);
            self.ivars().wake.wake();
        }
    }
);

/// An `NSButton` and its target, which AppKit holds weakly.
pub(crate) struct NativeButtonView {
    pub button: Retained<NSButton>,
    _target: Retained<NdButtonTarget>,
}

impl NativeButtonView {
    pub fn new(mtm: MainThreadMarker, emit: Emit, wake: Wake, props: &ButtonProps) -> Self {
        let target = NdButtonTarget::alloc(mtm).set_ivars(TargetIvars { emit, wake });
        let target: Retained<NdButtonTarget> = unsafe { msg_send![super(target), init] };
        let button = unsafe {
            NSButton::buttonWithTitle_target_action(
                &NSString::from_str(&props.label),
                Some(&target),
                Some(sel!(press:)),
                mtm,
            )
        };
        button.setBezelStyle(NSBezelStyle::Push);
        Self { button, _target: target }
    }

    pub fn update(&self, old: &ButtonProps, new: &ButtonProps) {
        if old.label != new.label {
            self.button.setTitle(&NSString::from_str(&new.label));
        }
    }

    /// The control's own size for its title and the system control size.
    pub fn measure(&self) -> Size {
        let size = self.button.fittingSize();
        Size { width: size.width as f32, height: size.height as f32 }
    }
}
