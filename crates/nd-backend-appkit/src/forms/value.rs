//! `slider` as `NSSlider` (or nd's drawn slider through `.drawn()`, which takes style
//! overrides the native one cannot express). The stepper lives in `toggle.rs`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use nd_components::{SliderChanged, SliderProps};
use nd_core::Size;
use nd_ui::backend::Props;
use nd_ui::events::{Emit, Synced};
use nd_ui::prelude::Hsla;
use nd_ui::semantics::{ActionError, Probe, SemanticAction};
use objc2::rc::Retained;
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSColor, NSEvent, NSResponder, NSSlider, NSView, NSViewLayerContentsRedrawPolicy};
use objc2_foundation::{MainThreadMarker, NSRect};
use objc2_quartz_core::CALayer;

use super::{Ctx, FormControl, NdControlTarget, apply_common, fitting, props, send_action, set_accessibility_label};
use crate::events::Wake;
use crate::look::{Color, length};

pub struct DrawnIvars {
    value: Cell<f32>,
    min: Cell<f32>,
    max: Cell<f32>,
    radius: Cell<Option<f32>>,
    track: RefCell<Option<Hsla>>,
    accent: Cell<Option<Hsla>>,
    emit: RefCell<Option<(Emit, Wake)>>,
    synced: RefCell<Option<Rc<RefCell<Synced<f32>>>>>,
    fill: RefCell<Option<Retained<CALayer>>>,
    knob: RefCell<Option<Retained<CALayer>>>,
}

define_class!(
    // SAFETY: NSView has no subclassing requirements beyond calling super, and this type does
    // not implement Drop.
    #[unsafe(super(NSView, NSResponder))]
    #[thread_kind = MainThreadOnly]
    #[ivars = DrawnIvars]
    pub struct NdDrawnSlider;

    impl NdDrawnSlider {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(wantsUpdateLayer))]
        fn wants_update_layer(&self) -> bool {
            true
        }

        // Colours resolve here, under the view's effective appearance.
        #[unsafe(method(updateLayer))]
        fn update_layer(&self) {
            self.paint();
        }

        #[unsafe(method(layout))]
        fn layout(&self) {
            let _: () = unsafe { msg_send![super(self), layout] };
            self.paint();
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            self.track_to(event);
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, event: &NSEvent) {
            self.track_to(event);
        }
    }
);

impl NdDrawnSlider {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(DrawnIvars {
            value: Cell::new(0.),
            min: Cell::new(0.),
            max: Cell::new(1.),
            radius: Cell::new(None),
            track: RefCell::new(None),
            accent: Cell::new(None),
            emit: RefCell::new(None),
            synced: RefCell::new(None),
            fill: RefCell::new(None),
            knob: RefCell::new(None),
        });
        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] };
        this.setWantsLayer(true);
        this.setLayerContentsRedrawPolicy(NSViewLayerContentsRedrawPolicy::OnSetNeedsDisplay);
        if let Some(layer) = this.layer() {
            let fill = CALayer::new();
            let knob = CALayer::new();
            layer.addSublayer(&fill);
            layer.addSublayer(&knob);
            *this.ivars().fill.borrow_mut() = Some(fill);
            *this.ivars().knob.borrow_mut() = Some(knob);
        }
        this
    }

    fn fraction(&self) -> f32 {
        let (min, max) = (self.ivars().min.get(), self.ivars().max.get());
        if max > min { ((self.ivars().value.get() - min) / (max - min)).clamp(0., 1.) } else { 0. }
    }

    fn paint(&self) {
        let Some(layer) = self.layer() else { return };
        let bounds = self.bounds();
        let height = bounds.size.height;
        let radius = self.ivars().radius.get().map_or(height / 2., |r| f64::from(r).min(height / 2.));
        let track =
            self.ivars().track.borrow().map_or_else(NSColor::quaternaryLabelColor, |c| Color::from(c).ns_color());
        let accent = self.ivars().accent.get().map_or_else(NSColor::controlAccentColor, |c| Color::from(c).ns_color());
        layer.setBackgroundColor(Some(&track.CGColor()));
        layer.setCornerRadius(radius);
        layer.setMasksToBounds(true);
        // The knob sits inside the track, inset by the track's own gap so both ends round alike.
        let inset = (height * 0.1).round();
        let knob_size = height - 2. * inset;
        let travel = (bounds.size.width - knob_size - 2. * inset).max(0.);
        let x = inset + travel * f64::from(self.fraction());
        if let Some(fill) = self.ivars().fill.borrow().as_ref() {
            fill.setBackgroundColor(Some(&accent.CGColor()));
            fill.setFrame(NSRect::new(bounds.origin, objc2_foundation::NSSize::new(x + knob_size + inset, height)));
            fill.setCornerRadius(radius);
        }
        if let Some(knob) = self.ivars().knob.borrow().as_ref() {
            knob.setBackgroundColor(Some(&NSColor::alternateSelectedControlTextColor().CGColor()));
            knob.setFrame(NSRect::new(
                objc2_foundation::NSPoint::new(x, inset),
                objc2_foundation::NSSize::new(knob_size, knob_size),
            ));
            knob.setCornerRadius((radius - inset).max(0.));
        }
    }

    fn track_to(&self, event: &NSEvent) {
        let at = self.convertPoint_fromView(event.locationInWindow(), None);
        let width = self.bounds().size.width;
        if width <= 0. {
            return;
        }
        let fraction = (at.x / width).clamp(0., 1.) as f32;
        let (min, max) = (self.ivars().min.get(), self.ivars().max.get());
        let value = min + (max - min) * fraction;
        self.ivars().value.set(value);
        if let Some(synced) = self.ivars().synced.borrow().as_ref() {
            synced.borrow_mut().report(value);
        }
        self.paint();
        if let Some((emit, wake)) = self.ivars().emit.borrow().as_ref() {
            emit.emit(SliderChanged { value });
            wake.wake();
        }
    }
}

enum View {
    Native { slider: Retained<NSSlider>, _target: Retained<NdControlTarget> },
    Drawn(Retained<NdDrawnSlider>),
}

pub(crate) struct SliderControl {
    view: View,
    value: Rc<RefCell<Synced<f32>>>,
    props: Rc<RefCell<SliderProps>>,
}

impl SliderControl {
    pub fn new(cx: Ctx, props: &SliderProps) -> Self {
        let value: Rc<RefCell<Synced<f32>>> = Rc::default();
        let live = Rc::new(RefCell::new(props.clone()));
        let view = if props.drawn {
            let view = NdDrawnSlider::new(cx.mtm);
            *view.ivars().emit.borrow_mut() = Some((cx.emit.clone(), cx.wake.clone()));
            *view.ivars().synced.borrow_mut() = Some(value.clone());
            View::Drawn(view)
        } else {
            let reported = value.clone();
            let send = cx.clone();
            let steps = live.clone();
            let target = NdControlTarget::new(cx.mtm, move |sender| {
                // The knob can sit between steps while dragging; the app is told the snapped value.
                let v = steps.borrow().normalise(sender.doubleValue() as f32);
                sender.setDoubleValue(v.into());
                if reported.borrow().value() == Some(&v) {
                    return;
                }
                reported.borrow_mut().report(v);
                send.send(SliderChanged { value: v });
            });
            let slider = NSSlider::new(cx.mtm);
            slider.setContinuous(true);
            target.wire(&slider);
            View::Native { slider, _target: target }
        };
        let this = Self { view, value, props: live };
        this.apply(None, props);
        this
    }

    fn apply(&self, old: Option<&SliderProps>, props: &SliderProps) {
        match &self.view {
            View::Native { slider, .. } => {
                slider.setMinValue(props.min.into());
                slider.setMaxValue(props.max.into());
                if old.is_none_or(|o| o.vertical != props.vertical) {
                    slider.setVertical(props.vertical);
                }
                // Tick marks as asked; with a step and no ticks, one per step (up to a count that
                // stays legible) so the knob snaps to them. Other steps snap in the action.
                let steps = props.step.filter(|s| *s > 0.).map(|s| ((props.max - props.min) / s).round() as isize + 1);
                let ticks = match steps {
                    _ if props.ticks > 0 => props.ticks as isize,
                    Some(n) if n <= 64 => n,
                    _ => 0,
                };
                slider.setNumberOfTickMarks(ticks);
                slider.setAllowsTickMarkValuesOnly(ticks > 0 && steps == Some(ticks));
                apply_common(Some(slider), slider, old.map(|o| (o.enabled, &o.tooltip)), props.enabled, &props.tooltip);
                if old.is_none_or(|o| o.label != props.label) {
                    set_accessibility_label(slider, props.label.as_deref());
                }
                self.value.borrow_mut().apply(&props.value, |v| {
                    slider.setDoubleValue((*v).into());
                    slider.doubleValue() as f32
                });
            }
            View::Drawn(view) => {
                let ivars = view.ivars();
                ivars.min.set(props.min);
                ivars.max.set(props.max);
                ivars.radius.set(props.style.corner_radii.top_left.map(length));
                *ivars.track.borrow_mut() =
                    props.style.background.as_ref().and_then(|f| f.color()).and_then(|b| b.as_solid());
                apply_common(None, view, old.map(|o| (o.enabled, &o.tooltip)), props.enabled, &props.tooltip);
                self.value.borrow_mut().apply(&props.value, |v| {
                    ivars.value.set(*v);
                    *v
                });
                view.setNeedsDisplay(true);
            }
        }
    }
}

impl FormControl for SliderControl {
    fn view(&self) -> &NSView {
        match &self.view {
            View::Native { slider, .. } => slider,
            View::Drawn(v) => v,
        }
    }

    fn update(&mut self, old: &Props, new: &Props) -> bool {
        let (old, new): (&SliderProps, &SliderProps) = (props(old), props(new));
        if old.drawn != new.drawn {
            return false;
        }
        *self.props.borrow_mut() = new.clone();
        self.apply(Some(old), new);
        true
    }

    fn measure(&self, _width: Option<f32>) -> Size {
        match &self.view {
            View::Native { slider, .. } => fitting(slider),
            // A drawn slider takes the size its style gives it.
            View::Drawn(_) => Size::default(),
        }
    }

    fn probe(&self, probe: &mut Probe) {
        if let View::Native { slider, .. } = &self.view {
            probe.enabled = Some(slider.isEnabled());
            probe.value = Some(nd_components::forms::number_text(slider.doubleValue(), None));
        }
    }

    fn perform(&self, action: &SemanticAction) -> Option<Result<(), ActionError>> {
        let View::Native { slider, .. } = &self.view else { return None };
        let SemanticAction::SetValue(value) = action else { return None };
        let Ok(v) = value.trim().parse::<f64>() else {
            return Some(Err(ActionError::NotActionable(format!("\"{value}\" is not a number"))));
        };
        slider.setDoubleValue(f64::from(self.props.borrow().normalise(v as f32)));
        Some(send_action(slider))
    }

    fn set_accent(&self, accent: Option<Hsla>) {
        match &self.view {
            View::Native { slider, .. } => {
                slider.setTrackFillColor(accent.map(|a| Color::from(a).ns_color()).as_deref())
            }
            View::Drawn(view) => {
                view.ivars().accent.set(accent);
                view.setNeedsDisplay(true);
            }
        }
    }
}
