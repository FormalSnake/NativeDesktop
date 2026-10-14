//! `date_picker` as `NSDatePicker` and `color_well` as `NSColorWell`.
//!
//! Civil values are the wall clock: they convert through a Gregorian calendar in the current
//! time zone, so a non-Gregorian system calendar still shows its own dates in the picker while
//! `CivilDateTime` keeps ISO years. The colour well uses `.minimal`, the swatch SwiftUI's
//! `ColorPicker` draws inline in a settings row; it opens the shared colour panel.

use std::cell::RefCell;
use std::rc::Rc;

use nd_components::{
    CivilDateTime, ColorChanged, ColorWellProps, DateChanged, DatePickerMode, DatePickerProps, DatePickerStyle,
};
use nd_core::Size;
use nd_ui::backend::Props;
use nd_ui::events::Synced;
use nd_ui::prelude::{Hsla, Rgba};
use nd_ui::semantics::{ActionError, Probe, SemanticAction, WidgetSemantics};
use objc2::rc::Retained;
use objc2_app_kit::{
    NSColor, NSColorSpace, NSColorWell, NSColorWellStyle, NSDatePicker, NSDatePickerElementFlags, NSDatePickerStyle,
    NSView,
};
use objc2_foundation::{NSCalendar, NSCalendarIdentifierGregorian, NSCalendarUnit, NSDate, NSDateComponents};

use super::{Ctx, FormControl, NdControlTarget, apply_common, fitting, props, send_action, set_accessibility_label};
use crate::events::defer;
use crate::look::Color;

fn gregorian() -> Option<Retained<NSCalendar>> {
    let calendar = NSCalendar::calendarWithIdentifier(unsafe { NSCalendarIdentifierGregorian })?;
    calendar.setTimeZone(&NSCalendar::currentCalendar().timeZone());
    Some(calendar)
}

fn to_date(value: CivilDateTime) -> Option<Retained<NSDate>> {
    let parts = NSDateComponents::new();
    parts.setYear(value.year as isize);
    parts.setMonth(value.month as isize);
    parts.setDay(value.day as isize);
    parts.setHour(value.hour as isize);
    parts.setMinute(value.minute as isize);
    parts.setSecond(value.second as isize);
    gregorian()?.dateFromComponents(&parts)
}

fn civil(date: &NSDate) -> Option<CivilDateTime> {
    let units = NSCalendarUnit::Year
        | NSCalendarUnit::Month
        | NSCalendarUnit::Day
        | NSCalendarUnit::Hour
        | NSCalendarUnit::Minute
        | NSCalendarUnit::Second;
    let parts = gregorian()?.components_fromDate(units, date);
    Some(CivilDateTime {
        year: parts.year() as i32,
        month: parts.month() as u8,
        day: parts.day() as u8,
        hour: parts.hour() as u8,
        minute: parts.minute() as u8,
        second: parts.second() as u8,
    })
}

pub(crate) struct DateControl {
    picker: Retained<NSDatePicker>,
    _target: Retained<NdControlTarget>,
    value: Rc<RefCell<Synced<CivilDateTime>>>,
    props: DatePickerProps,
    cx: Ctx,
}

impl DateControl {
    pub fn new(cx: Ctx, props: &DatePickerProps) -> Self {
        let value: Rc<RefCell<Synced<CivilDateTime>>> = Rc::default();
        let reported = value.clone();
        let send = cx.clone();
        let target = NdControlTarget::new(cx.mtm, move |sender| {
            let Some(held) = sender.downcast_ref::<NSDatePicker>().and_then(|p| civil(&p.dateValue())) else { return };
            reported.borrow_mut().report(held);
            send.send(DateChanged { value: held });
        });
        let picker = NSDatePicker::new(cx.mtm);
        target.wire(&picker);
        let this = Self { picker, _target: target, value, props: props.clone(), cx };
        this.apply(None, props);
        this
    }

    fn held(&self) -> Option<CivilDateTime> {
        civil(&self.picker.dateValue())
    }

    fn apply(&self, old: Option<&DatePickerProps>, new: &DatePickerProps) {
        let p = &self.picker;
        if old.is_none_or(|o| o.mode != new.mode) {
            p.setDatePickerElements(match new.mode {
                DatePickerMode::Date => NSDatePickerElementFlags::YearMonthDay,
                DatePickerMode::Time => NSDatePickerElementFlags::HourMinuteSecond,
                DatePickerMode::DateTime => {
                    NSDatePickerElementFlags::YearMonthDay | NSDatePickerElementFlags::HourMinuteSecond
                }
            });
        }
        if old.is_none_or(|o| o.presentation != new.presentation) {
            p.setDatePickerStyle(match new.presentation {
                DatePickerStyle::Field => NSDatePickerStyle::TextField,
                DatePickerStyle::Stepper => NSDatePickerStyle::TextFieldAndStepper,
                DatePickerStyle::Graphical => NSDatePickerStyle::ClockAndCalendar,
            });
        }
        if old.is_none_or(|o| o.min != new.min || o.max != new.max) {
            p.setMinDate(new.min.and_then(to_date).as_deref());
            p.setMaxDate(new.max.and_then(to_date).as_deref());
            // New bounds can move the value the picker holds; record that before comparing.
            if let Some(held) = self.held()
                && self.value.borrow().value().is_some()
            {
                self.value.borrow_mut().report(held);
            }
        }
        apply_common(Some(p), p, old.map(|o| (o.enabled, &o.tooltip)), new.enabled, &new.tooltip);
        if old.is_none_or(|o| o.label != new.label) {
            set_accessibility_label(p, new.label.as_deref());
        }
        let normalised = self.value.borrow_mut().apply(&new.value, |v| {
            let clamped = new.normalise(*v);
            if let Some(date) = to_date(clamped) {
                p.setDateValue(&date);
            }
            self.held().unwrap_or(clamped)
        });
        if let Some(value) = normalised {
            // The update runs inside `programmatic`, which drops events; the app still has to
            // learn the value the picker holds.
            let cx = self.cx.clone();
            defer(move || cx.send(DateChanged { value }));
        }
    }
}

impl FormControl for DateControl {
    fn view(&self) -> &NSView {
        &self.picker
    }

    fn update(&mut self, old: &Props, new: &Props) -> bool {
        let (old, new): (&DatePickerProps, &DatePickerProps) = (props(old), props(new));
        self.apply(Some(old), new);
        self.props = new.clone();
        true
    }

    fn measure(&self, _width: Option<f32>) -> Size {
        fitting(&self.picker)
    }

    fn probe(&self, probe: &mut Probe) {
        probe.enabled = Some(self.picker.isEnabled());
        if let Some(held) = self.held() {
            let full = held.to_string();
            probe.value = Some(match self.props.mode {
                DatePickerMode::Date => full[..10].to_owned(),
                DatePickerMode::Time => full[11..].to_owned(),
                DatePickerMode::DateTime => full,
            });
        }
    }

    fn perform(&self, action: &SemanticAction) -> Option<Result<(), ActionError>> {
        let SemanticAction::SetValue(_) = action else { return None };
        let current = self.held()?;
        let mut props = self.props.clone();
        props.value = current;
        let event = match props.respond(action)? {
            Ok(event) => event,
            Err(e) => return Some(Err(e)),
        };
        let value = event.downcast_ref::<DateChanged>()?.value;
        if let Some(date) = to_date(value) {
            self.picker.setDateValue(&date);
        }
        Some(send_action(&self.picker))
    }
}

fn ns_color(color: Hsla) -> Retained<NSColor> {
    Color::from(color).ns_color()
}

fn hsla_of(color: &NSColor) -> Option<Hsla> {
    let c = color.colorUsingColorSpace(&NSColorSpace::sRGBColorSpace())?;
    Some(
        Rgba {
            r: c.redComponent() as f32,
            g: c.greenComponent() as f32,
            b: c.blueComponent() as f32,
            a: c.alphaComponent() as f32,
        }
        .into(),
    )
}

/// Within half a byte per channel: what an sRGB round trip through `NSColor` may move.
fn same_colour(a: Hsla, b: Hsla) -> bool {
    let (a, b) = (Rgba::from(a), Rgba::from(b));
    let tolerance = 0.5 / 255.;
    (a.r - b.r).abs() <= tolerance
        && (a.g - b.g).abs() <= tolerance
        && (a.b - b.b).abs() <= tolerance
        && (a.a - b.a).abs() <= tolerance
}

fn hex(color: Hsla) -> String {
    let c = Rgba::from(color);
    let byte = |v: f32| (v.clamp(0., 1.) * 255.).round() as u8;
    let (r, g, b, a) = (byte(c.r), byte(c.g), byte(c.b), byte(c.a));
    if a == 255 { format!("#{r:02x}{g:02x}{b:02x}") } else { format!("#{r:02x}{g:02x}{b:02x}{a:02x}") }
}

pub(crate) struct ColorWellControl {
    well: Retained<NSColorWell>,
    _target: Retained<NdControlTarget>,
    color: Rc<RefCell<Synced<Hsla>>>,
    props: ColorWellProps,
    cx: Ctx,
}

impl ColorWellControl {
    pub fn new(cx: Ctx, props: &ColorWellProps) -> Self {
        let color: Rc<RefCell<Synced<Hsla>>> = Rc::default();
        let reported = color.clone();
        let send = cx.clone();
        let target = NdControlTarget::new(cx.mtm, move |sender| {
            let Some(held) = sender.downcast_ref::<NSColorWell>().and_then(|w| hsla_of(&w.color())) else { return };
            reported.borrow_mut().report(held);
            send.send(ColorChanged { color: held });
        });
        let well = NSColorWell::colorWellWithStyle(NSColorWellStyle::Minimal, cx.mtm);
        target.wire(&well);
        let this = Self { well, _target: target, color, props: props.clone(), cx };
        this.apply(None, props);
        this
    }

    fn apply(&self, old: Option<&ColorWellProps>, new: &ColorWellProps) {
        let w = &self.well;
        if old.is_none_or(|o| o.alpha != new.alpha) {
            w.setSupportsAlpha(new.alpha);
        }
        apply_common(Some(w), w, old.map(|o| (o.enabled, &o.tooltip)), new.enabled, &new.tooltip);
        if old.is_none_or(|o| o.label != new.label) {
            set_accessibility_label(w, new.label.as_deref());
        }
        // Without alpha support the well drops opacity, so the value it can hold is opaque.
        let wanted = if new.alpha { new.color } else { Hsla { a: 1., ..new.color } };
        let normalised = self.color.borrow_mut().apply(&new.color, |_| {
            w.setColor(&ns_color(wanted));
            match hsla_of(&w.color()) {
                Some(held) if !same_colour(held, wanted) => held,
                _ => wanted,
            }
        });
        if let Some(color) = normalised {
            let cx = self.cx.clone();
            defer(move || cx.send(ColorChanged { color }));
        }
    }
}

impl Drop for ColorWellControl {
    fn drop(&mut self) {
        // The colour panel is shared; it must not keep a well that is going away as its target.
        if self.well.isActive() {
            self.well.deactivate();
        }
    }
}

impl FormControl for ColorWellControl {
    fn view(&self) -> &NSView {
        &self.well
    }

    fn update(&mut self, old: &Props, new: &Props) -> bool {
        let (old, new): (&ColorWellProps, &ColorWellProps) = (props(old), props(new));
        self.apply(Some(old), new);
        self.props = new.clone();
        true
    }

    fn measure(&self, _width: Option<f32>) -> Size {
        fitting(&self.well)
    }

    fn probe(&self, probe: &mut Probe) {
        probe.enabled = Some(self.well.isEnabled());
        probe.value = hsla_of(&self.well.color()).map(hex);
    }

    fn perform(&self, action: &SemanticAction) -> Option<Result<(), ActionError>> {
        let SemanticAction::SetValue(_) = action else { return None };
        let event = match self.props.respond(action)? {
            Ok(event) => event,
            Err(e) => return Some(Err(e)),
        };
        let color = event.downcast_ref::<ColorChanged>()?.color;
        self.well.setColor(&ns_color(color));
        Some(send_action(&self.well))
    }
}
