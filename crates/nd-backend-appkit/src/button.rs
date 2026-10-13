//! The native push button: AppKit's own `NSButton`, with its action routed into the event queue.

use nd_core::{NodeId, Size};
use objc2::rc::Retained;
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{NSBezelStyle, NSButton};
use objc2_foundation::{MainThreadMarker, NSObject, NSObjectProtocol, NSString};

use crate::events::{EventQueue, NativeEvent, NativeEventKind};
use crate::props::ButtonProps;

pub struct TargetIvars {
    node: NodeId,
    queue: EventQueue,
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
            let ivars = self.ivars();
            ivars.queue.push(NativeEvent { node: ivars.node, kind: NativeEventKind::Click });
        }
    }
);

/// An `NSButton` and the target it holds weakly (AppKit targets are unretained).
pub(crate) struct NativeButtonView {
    pub button: Retained<NSButton>,
    _target: Retained<NdButtonTarget>,
}

impl NativeButtonView {
    pub fn new(mtm: MainThreadMarker, node: NodeId, queue: EventQueue, props: &ButtonProps) -> Self {
        let target = NdButtonTarget::alloc(mtm).set_ivars(TargetIvars { node, queue });
        let target: Retained<NdButtonTarget> = unsafe { msg_send![super(target), init] };
        let button = unsafe {
            NSButton::buttonWithTitle_target_action(&NSString::from_str(&props.title), Some(&target), Some(sel!(press:)), mtm)
        };
        button.setBezelStyle(NSBezelStyle::Push);
        button.setEnabled(props.enabled);
        Self { button, _target: target }
    }

    pub fn update(&self, old: &ButtonProps, new: &ButtonProps) {
        if old.title != new.title {
            self.button.setTitle(&NSString::from_str(&new.title));
        }
        if old.enabled != new.enabled {
            self.button.setEnabled(new.enabled);
        }
    }

    /// The control's own size for its title and the system control size.
    pub fn measure(&self) -> Size {
        let size = self.button.fittingSize();
        Size { width: size.width as f32, height: size.height as f32 }
    }
}
