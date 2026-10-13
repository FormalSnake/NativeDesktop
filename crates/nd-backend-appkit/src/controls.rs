//! `select` (`NSPopUpButton`, or `NSSegmentedControl` through `Mapping::Segmented`), `slider`
//! (`NSSlider`, or nd's drawn slider through `.drawn()`) and `separator` (`NSBox`). Controlled
//! values go through [`Synced`] so an app that feeds a reported value back sets nothing
//! (3.13 rule 2).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use nd_components::{Mapping, SelectChanged, SelectProps, SliderChanged, SliderProps};
use nd_core::Size;
use nd_ui::events::{Emit, Synced};
use nd_ui::prelude::Hsla;
use objc2::rc::Retained;
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSBox, NSBoxType, NSColor, NSControl, NSEvent, NSPopUpButton, NSResponder, NSSegmentedControl, NSSlider, NSView,
    NSViewLayerContentsRedrawPolicy,
};
use objc2_foundation::{MainThreadMarker, NSArray, NSObject, NSObjectProtocol, NSRect, NSString};
use objc2_quartz_core::CALayer;

use crate::events::Wake;
use crate::look::{Color, length};

pub struct TargetIvars {
    on_action: Box<dyn Fn(&NSControl)>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements and this type does not implement Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = TargetIvars]
    pub struct NdControlTarget;

    unsafe impl NSObjectProtocol for NdControlTarget {}

    impl NdControlTarget {
        #[unsafe(method(changed:))]
        fn changed(&self, sender: &NSControl) {
            (self.ivars().on_action)(sender);
        }
    }
);

impl NdControlTarget {
    fn new(mtm: MainThreadMarker, on_action: impl Fn(&NSControl) + 'static) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(TargetIvars { on_action: Box::new(on_action) });
        unsafe { msg_send![super(this), init] }
    }
}

fn wire(control: &NSControl, target: &NdControlTarget) {
    unsafe {
        control.setTarget(Some(target));
        control.setAction(Some(sel!(changed:)));
    }
}

enum SelectView {
    PopUp(Retained<NSPopUpButton>),
    Segmented(Retained<NSSegmentedControl>),
}

pub(crate) struct SelectNative {
    view: SelectView,
    _target: Retained<NdControlTarget>,
    props: SelectProps,
    selected: Rc<RefCell<Synced<usize>>>,
}

fn segmented(mapping: Mapping) -> bool {
    matches!(mapping, Mapping::Segmented | Mapping::Radio)
}

impl SelectNative {
    pub fn new(mtm: MainThreadMarker, emit: Emit, wake: Wake, props: &SelectProps) -> Self {
        let selected: Rc<RefCell<Synced<usize>>> = Rc::default();
        let reported = selected.clone();
        let target = NdControlTarget::new(mtm, move |sender| {
            let index = if let Some(s) = sender.downcast_ref::<NSSegmentedControl>() {
                s.selectedSegment()
            } else if let Some(p) = sender.downcast_ref::<NSPopUpButton>() {
                p.indexOfSelectedItem()
            } else {
                return;
            };
            if index >= 0 {
                reported.borrow_mut().report(index as usize);
                emit.emit(SelectChanged { index: index as usize });
                wake.wake();
            }
        });
        let titles: Vec<Retained<NSString>> = props.options.iter().map(|o| NSString::from_str(o)).collect();
        let view = if segmented(props.mapping) {
            let control = NSSegmentedControl::new(mtm);
            control.setSegmentCount(titles.len() as isize);
            for (i, title) in titles.iter().enumerate() {
                control.setLabel_forSegment(title, i as isize);
            }
            wire(&control, &target);
            SelectView::Segmented(control)
        } else {
            let control = NSPopUpButton::initWithFrame_pullsDown(NSPopUpButton::alloc(mtm), NSRect::ZERO, false);
            control.addItemsWithTitles(&NSArray::from_retained_slice(&titles));
            wire(&control, &target);
            SelectView::PopUp(control)
        };
        let this = Self { view, _target: target, props: props.clone(), selected };
        this.select(props.selected);
        this
    }

    pub fn view(&self) -> &NSView {
        match &self.view {
            SelectView::PopUp(v) => v,
            SelectView::Segmented(v) => v,
        }
    }

    /// False when the mapping changed: the backend recreates the control.
    pub fn can_update(&self, props: &SelectProps) -> bool {
        segmented(self.props.mapping) == segmented(props.mapping) && self.props.options == props.options
    }

    pub fn update(&mut self, props: &SelectProps) {
        self.select(props.selected);
        self.props = props.clone();
    }

    fn select(&self, index: usize) {
        let view = &self.view;
        self.selected.borrow_mut().apply(&index, |i| {
            match view {
                SelectView::PopUp(p) => p.selectItemAtIndex(*i as isize),
                SelectView::Segmented(s) => s.setSelectedSegment(*i as isize),
            }
            *i
        });
    }

    pub fn set_accent(&self, accent: Option<Hsla>) {
        match &self.view {
            SelectView::Segmented(s) => {
                s.setSelectedSegmentBezelColor(accent.map(|a| Color::from(a).ns_color()).as_deref())
            }
            SelectView::PopUp(p) => p.setBezelColor(accent.map(|a| Color::from(a).ns_color()).as_deref()),
        }
    }

    pub fn measure(&self) -> Size {
        let s = self.view().fittingSize();
        Size { width: s.width as f32, height: s.height as f32 }
    }
}

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

enum SliderView {
    Native { slider: Retained<NSSlider>, _target: Retained<NdControlTarget> },
    Drawn(Retained<NdDrawnSlider>),
}

pub(crate) struct SliderNative {
    view: SliderView,
    value: Rc<RefCell<Synced<f32>>>,
}

impl SliderNative {
    pub fn new(mtm: MainThreadMarker, emit: Emit, wake: Wake, props: &SliderProps) -> Self {
        let value: Rc<RefCell<Synced<f32>>> = Rc::default();
        let view = if props.drawn {
            let view = NdDrawnSlider::new(mtm);
            *view.ivars().emit.borrow_mut() = Some((emit, wake));
            *view.ivars().synced.borrow_mut() = Some(value.clone());
            SliderView::Drawn(view)
        } else {
            let reported = value.clone();
            let target = NdControlTarget::new(mtm, move |sender| {
                let v = sender.doubleValue() as f32;
                reported.borrow_mut().report(v);
                emit.emit(SliderChanged { value: v });
                wake.wake();
            });
            let slider = NSSlider::new(mtm);
            slider.setContinuous(true);
            wire(&slider, &target);
            SliderView::Native { slider, _target: target }
        };
        let this = Self { view, value };
        this.apply(props);
        this
    }

    pub fn view(&self) -> &NSView {
        match &self.view {
            SliderView::Native { slider: v, .. } => v,
            SliderView::Drawn(v) => v,
        }
    }

    pub fn apply(&self, props: &SliderProps) {
        match &self.view {
            SliderView::Native { slider, .. } => {
                slider.setMinValue(props.min.into());
                slider.setMaxValue(props.max.into());
                self.value.borrow_mut().apply(&props.value, |v| {
                    slider.setDoubleValue((*v).into());
                    *v
                });
            }
            SliderView::Drawn(view) => {
                let ivars = view.ivars();
                ivars.min.set(props.min);
                ivars.max.set(props.max);
                ivars.radius.set(props.style.corner_radii.top_left.map(length));
                *ivars.track.borrow_mut() =
                    props.style.background.as_ref().and_then(|f| f.color()).and_then(|b| b.as_solid());
                self.value.borrow_mut().apply(&props.value, |v| {
                    ivars.value.set(*v);
                    *v
                });
                view.setNeedsDisplay(true);
            }
        }
    }

    pub fn set_accent(&self, accent: Option<Hsla>) {
        match &self.view {
            SliderView::Native { slider, .. } => {
                slider.setTrackFillColor(accent.map(|a| Color::from(a).ns_color()).as_deref())
            }
            SliderView::Drawn(view) => {
                view.ivars().accent.set(accent);
                view.setNeedsDisplay(true);
            }
        }
    }

    pub fn measure(&self) -> Size {
        match &self.view {
            SliderView::Native { slider: v, .. } => {
                let s = v.fittingSize();
                Size { width: s.width as f32, height: s.height as f32 }
            }
            // A drawn slider takes the size its style gives it.
            SliderView::Drawn(_) => Size::default(),
        }
    }
}

/// `NSBox` of the separator type.
pub(crate) fn separator(mtm: MainThreadMarker) -> Retained<NSBox> {
    let b = NSBox::new(mtm);
    b.setBoxType(NSBoxType::Separator);
    b
}
