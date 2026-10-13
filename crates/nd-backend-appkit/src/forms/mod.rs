//! The forms family on AppKit (`nd_components::forms`): each control is AppKit's own class
//! (`NSTextField`, `NSSwitch`, `NSSegmentedControl`, `NSDatePicker`, ...), behind one
//! [`FormControl`] trait the backend holds as `Native::Control`.
//!
//! Rules every control follows:
//! - Controlled values go through a [`Synced`](nd_ui::events::Synced) slot; the backend runs
//!   `update` inside `EventQueue::programmatic`, so a delegate a setter fires never reaches the
//!   app (ARCHITECTURE.md 3.13).
//! - User changes report into the slot first, then emit, then wake the host.
//! - No system metric is a constant here: sizes come from `fittingSize` or the control's
//!   `cell().cellSize()`, colours from `NSColor` semantic colours, fonts from
//!   `NSFont.preferredFont(forTextStyle:)`.
//! - Automation acts through the control: `perform` sets the native value and sends the
//!   control's own action, so the same report-and-emit path runs as for the user.

mod choice;
mod date;
mod display;
mod toggle;
mod value;

use std::any::TypeId;

use nd_core::Size;
use nd_ui::backend::{Props, WidgetKind};
use nd_ui::events::Emit;
use nd_ui::prelude::{Hsla, SharedString};
use nd_ui::semantics::{ActionError, Probe, SemanticAction};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{NSAccessibility, NSApplication, NSControl, NSView};
use objc2_foundation::{MainThreadMarker, NSObject, NSObjectProtocol, NSString};

use crate::events::Wake;

/// One native form control the backend holds.
pub(crate) trait FormControl {
    /// The view its parent holds.
    fn view(&self) -> &NSView;
    /// The view that takes first responder when nd focuses the node; a field inside a container
    /// returns the field.
    fn focus_view(&self) -> &NSView {
        self.view()
    }
    /// Applies `new` (inside `programmatic`). `false` when the control cannot show it and the
    /// backend must create a fresh one (a select switching between pop-up and segmented).
    fn update(&mut self, old: &Props, new: &Props) -> bool;
    /// The control's own size; `width` is the definite width offered, if any.
    fn measure(&self, width: Option<f32>) -> Size;
    /// Live state for the semantic tree (enabled, value, checked).
    fn probe(&self, _probe: &mut Probe) {}
    /// Carries out `action` through the native control. `None` falls back to the props'
    /// [`respond`](nd_ui::semantics::WidgetSemantics::respond), emitted as the control would.
    fn perform(&self, _action: &SemanticAction) -> Option<Result<(), ActionError>> {
        None
    }
    /// The accent a theme override sets at this control.
    fn set_accent(&self, _accent: Option<Hsla>) {}
}

/// What a control needs to report user changes.
#[derive(Clone)]
pub(crate) struct Ctx {
    pub mtm: MainThreadMarker,
    pub emit: Emit,
    pub wake: Wake,
}

impl Ctx {
    /// Emits `event` and wakes the host so the app sees it on the next turn.
    pub fn send<E: 'static>(&self, event: E) {
        self.emit.emit(event);
        self.wake.wake();
    }
}

fn is<W: 'static>(kind: WidgetKind) -> bool {
    kind.type_id == TypeId::of::<W>()
}

fn props<'a, T: 'static>(p: &'a Props) -> &'a T {
    p.downcast_ref::<T>().unwrap_or_else(|| panic!("form control needs {} props", std::any::type_name::<T>()))
}

/// The forms control for `kind`, or `None` when `kind` is not one.
pub(crate) fn create(kind: WidgetKind, p: &Props, cx: Ctx) -> Option<Box<dyn FormControl>> {
    use nd_components::forms::*;
    let control: Box<dyn FormControl> = if is::<SelectKind>(kind) {
        Box::new(choice::SelectControl::new(cx, props(p)))
    } else if is::<CheckboxKind>(kind) {
        Box::new(toggle::CheckboxControl::new(cx, props(p)))
    } else if is::<SwitchKind>(kind) {
        Box::new(toggle::SwitchControl::new(cx, props(p)))
    } else if is::<StepperKind>(kind) {
        Box::new(toggle::StepperControl::new(cx, props(p)))
    } else if is::<SliderKind>(kind) {
        Box::new(value::SliderControl::new(cx, props(p)))
    } else if is::<DatePickerKind>(kind) {
        Box::new(date::DateControl::new(cx, props(p)))
    } else if is::<ColorWellKind>(kind) {
        Box::new(date::ColorWellControl::new(cx, props(p)))
    } else if is::<ProgressKind>(kind) {
        Box::new(display::ProgressControl::new(cx, props(p)))
    } else if is::<LabelKind>(kind) {
        Box::new(display::LabelControl::new(cx, props(p)))
    } else if is::<LinkKind>(kind) {
        Box::new(display::LinkControl::new(cx, props(p)))
    } else {
        return None;
    };
    Some(control)
}

pub(crate) struct TargetIvars {
    on_action: Box<dyn Fn(&NSControl)>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements and this type does not implement Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = TargetIvars]
    pub(crate) struct NdControlTarget;

    unsafe impl NSObjectProtocol for NdControlTarget {}

    impl NdControlTarget {
        #[unsafe(method(changed:))]
        fn changed(&self, sender: &NSControl) {
            (self.ivars().on_action)(sender);
        }
    }
);

impl NdControlTarget {
    pub fn new(mtm: MainThreadMarker, on_action: impl Fn(&NSControl) + 'static) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(TargetIvars { on_action: Box::new(on_action) });
        unsafe { msg_send![super(this), init] }
    }

    /// Points `control`'s target and action here. AppKit holds the target weakly; the control's
    /// owner keeps it alive.
    pub fn wire(&self, control: &NSControl) {
        unsafe {
            control.setTarget(Some(self));
            control.setAction(Some(sel!(changed:)));
        }
    }
}

/// Sends `control`'s action to its target the way `NSAccessibilityPressAction` does: no
/// pressed highlight, no tracking loop. What `perform` calls after setting a value.
pub(crate) fn send_action(control: &NSControl) -> Result<(), ActionError> {
    let Some(mtm) = MainThreadMarker::new() else {
        return Err(ActionError::NotActionable("off the main thread".into()));
    };
    if !control.isEnabled() {
        return Err(ActionError::NotActionable("the control is disabled".into()));
    }
    let Some(action) = control.action() else {
        return Err(ActionError::NotActionable("the control has no action".into()));
    };
    let target = control.target();
    let sender: &AnyObject = control;
    let sent =
        unsafe { NSApplication::sharedApplication(mtm).sendAction_to_from(action, target.as_deref(), Some(sender)) };
    if sent { Ok(()) } else { Err(ActionError::NotActionable("no target took the control's action".into())) }
}

/// `enabled` and `tooltip`, applied when they changed.
pub(crate) fn apply_common(
    control: Option<&NSControl>,
    view: &NSView,
    old: Option<(bool, &Option<SharedString>)>,
    enabled: bool,
    tooltip: &Option<SharedString>,
) {
    if let Some(control) = control
        && old.is_none_or(|(e, _)| e != enabled)
    {
        control.setEnabled(enabled);
    }
    if old.is_none_or(|(_, t)| t != tooltip) {
        view.setToolTip(tooltip.as_deref().map(NSString::from_str).as_deref());
    }
}

/// The accessibility label a control's props name it with, when its own title does not.
pub(crate) fn set_accessibility_label(view: &NSView, label: Option<&str>) {
    view.setAccessibilityLabel(label.map(NSString::from_str).as_deref());
}

/// The control's own size from AppKit.
pub(crate) fn fitting(view: &NSView) -> Size {
    let s = view.fittingSize();
    Size { width: s.width as f32, height: s.height as f32 }
}
