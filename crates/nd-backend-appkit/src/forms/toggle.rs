//! `checkbox` as an `NSButton` of the switch type, `switch` as `NSSwitch` and `stepper` as
//! `NSStepper`, with an `NSTextField` beside it when the props ask for a value field.

use std::cell::RefCell;
use std::rc::Rc;

use nd_components::{CheckboxProps, StepperChanged, StepperProps, SwitchProps, Toggled, number_text};
use nd_core::Size;
use nd_ui::backend::Props;
use nd_ui::events::Synced;
use nd_ui::prelude::Hsla;
use nd_ui::semantics::{ActionError, Probe, SemanticAction};
use objc2::rc::Retained;
use objc2_app_kit::{
    NSButton, NSButtonType, NSControl, NSControlStateValue, NSControlStateValueMixed, NSControlStateValueOff,
    NSControlStateValueOn, NSLayoutAttribute, NSLayoutConstraint, NSStackView, NSStepper, NSSwitch, NSTextField,
    NSUserInterfaceLayoutOrientation, NSView,
};
use objc2_foundation::{NSNumber, NSNumberFormatter, NSNumberFormatterStyle, NSString};

use super::{Ctx, FormControl, NdControlTarget, apply_common, fitting, props, send_action, set_accessibility_label};
use crate::look::Color;

fn parse_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "on" | "yes" | "checked" => Some(true),
        "false" | "0" | "off" | "no" | "unchecked" => Some(false),
        _ => None,
    }
}

/// Sets `control` to the state a toggle action asks for, then sends its own action.
fn toggle_perform(
    control: &NSControl,
    state: NSControlStateValue,
    set: impl Fn(NSControlStateValue),
    action: &SemanticAction,
) -> Option<Result<(), ActionError>> {
    let next = match action {
        SemanticAction::Click => {
            if state == NSControlStateValueOn {
                NSControlStateValueOff
            } else {
                NSControlStateValueOn
            }
        }
        SemanticAction::SetValue(value) => match parse_bool(value) {
            Some(true) => NSControlStateValueOn,
            Some(false) => NSControlStateValueOff,
            None => return Some(Err(ActionError::NotActionable(format!("\"{value}\" is not on or off")))),
        },
        _ => return None,
    };
    set(next);
    Some(send_action(control))
}

pub(crate) struct CheckboxControl {
    button: Retained<NSButton>,
    _target: Retained<NdControlTarget>,
    props: CheckboxProps,
    state: Rc<RefCell<Synced<NSControlStateValue>>>,
}

impl CheckboxControl {
    pub fn new(cx: Ctx, props: &CheckboxProps) -> Self {
        let state: Rc<RefCell<Synced<NSControlStateValue>>> = Rc::default();
        let reported = state.clone();
        let mtm = cx.mtm;
        let target = NdControlTarget::new(mtm, move |sender| {
            let Some(button) = sender.downcast_ref::<NSButton>() else { return };
            // A click on a mixed box checks it, as the semantics say; AppKit's own cycle would
            // land on mixed again.
            let next =
                if button.state() == NSControlStateValueOff { NSControlStateValueOff } else { NSControlStateValueOn };
            button.setState(next);
            reported.borrow_mut().report(next);
            cx.send(Toggled { checked: next == NSControlStateValueOn });
        });
        let button = NSButton::new(mtm);
        button.setButtonType(NSButtonType::Switch);
        target.wire(&button);
        let mut this = Self { button, _target: target, props: props.clone(), state };
        this.apply(None, props);
        this.props = props.clone();
        this
    }

    fn apply(&self, old: Option<&CheckboxProps>, new: &CheckboxProps) {
        let button = &self.button;
        if old.is_none_or(|o| o.label != new.label) {
            button.setTitle(&NSString::from_str(&new.label));
        }
        apply_common(Some(button), button, old.map(|o| (o.enabled, &o.tooltip)), new.enabled, &new.tooltip);
        button.setAllowsMixedState(new.mixed);
        let want = if new.mixed {
            NSControlStateValueMixed
        } else if new.checked {
            NSControlStateValueOn
        } else {
            NSControlStateValueOff
        };
        self.state.borrow_mut().apply(&want, |s| {
            button.setState(*s);
            button.state()
        });
    }
}

impl FormControl for CheckboxControl {
    fn view(&self) -> &NSView {
        &self.button
    }

    fn update(&mut self, old: &Props, new: &Props) -> bool {
        let (old, new): (&CheckboxProps, &CheckboxProps) = (props(old), props(new));
        self.apply(Some(old), new);
        self.props = new.clone();
        true
    }

    fn measure(&self, _width: Option<f32>) -> Size {
        fitting(&self.button)
    }

    fn probe(&self, probe: &mut Probe) {
        let state = self.button.state();
        probe.enabled = Some(self.button.isEnabled());
        probe.checked = Some(state == NSControlStateValueOn);
        probe.value = Some(
            if state == NSControlStateValueMixed {
                "mixed"
            } else if state == NSControlStateValueOn {
                "true"
            } else {
                "false"
            }
            .to_owned(),
        );
    }

    fn perform(&self, action: &SemanticAction) -> Option<Result<(), ActionError>> {
        toggle_perform(&self.button, self.button.state(), |s| self.button.setState(s), action)
    }

    fn set_accent(&self, accent: Option<Hsla>) {
        self.button.setContentTintColor(accent.map(|a| Color::from(a).ns_color()).as_deref());
    }
}

pub(crate) struct SwitchControl {
    switch: Retained<NSSwitch>,
    _target: Retained<NdControlTarget>,
    state: Rc<RefCell<Synced<NSControlStateValue>>>,
}

impl SwitchControl {
    pub fn new(cx: Ctx, props: &SwitchProps) -> Self {
        let state: Rc<RefCell<Synced<NSControlStateValue>>> = Rc::default();
        let reported = state.clone();
        let mtm = cx.mtm;
        let target = NdControlTarget::new(mtm, move |sender| {
            let Some(switch) = sender.downcast_ref::<NSSwitch>() else { return };
            let on = switch.state() == NSControlStateValueOn;
            reported.borrow_mut().report(switch.state());
            cx.send(Toggled { checked: on });
        });
        let switch = NSSwitch::new(mtm);
        target.wire(&switch);
        let this = Self { switch, _target: target, state };
        this.apply(None, props);
        this
    }

    fn apply(&self, old: Option<&SwitchProps>, new: &SwitchProps) {
        let switch = &self.switch;
        apply_common(Some(switch), switch, old.map(|o| (o.enabled, &o.tooltip)), new.enabled, &new.tooltip);
        if old.is_none_or(|o| o.label != new.label) {
            set_accessibility_label(switch, new.label.as_deref());
        }
        let want = if new.checked { NSControlStateValueOn } else { NSControlStateValueOff };
        self.state.borrow_mut().apply(&want, |s| {
            switch.setState(*s);
            switch.state()
        });
    }
}

impl FormControl for SwitchControl {
    fn view(&self) -> &NSView {
        &self.switch
    }

    fn update(&mut self, old: &Props, new: &Props) -> bool {
        self.apply(Some(props(old)), props(new));
        true
    }

    fn measure(&self, _width: Option<f32>) -> Size {
        fitting(&self.switch)
    }

    fn probe(&self, probe: &mut Probe) {
        let on = self.switch.state() == NSControlStateValueOn;
        probe.enabled = Some(self.switch.isEnabled());
        probe.checked = Some(on);
        probe.value = Some(on.to_string());
    }

    fn perform(&self, action: &SemanticAction) -> Option<Result<(), ActionError>> {
        toggle_perform(&self.switch, self.switch.state(), |s| self.switch.setState(s), action)
    }
}

pub(crate) struct StepperControl {
    root: Retained<NSView>,
    stepper: Retained<NSStepper>,
    field: Option<Field>,
    _target: Retained<NdControlTarget>,
    props: Rc<RefCell<StepperProps>>,
    value: Rc<RefCell<Synced<f64>>>,
    cx: Ctx,
}

struct Field {
    text: Retained<NSTextField>,
    formatter: Retained<NSNumberFormatter>,
    width: Retained<NSLayoutConstraint>,
}

impl StepperControl {
    pub fn new(cx: Ctx, props: &StepperProps) -> Self {
        let live = Rc::new(RefCell::new(props.clone()));
        let value: Rc<RefCell<Synced<f64>>> = Rc::default();
        let stepper = NSStepper::new(cx.mtm);
        let field = props.field.map(|_| {
            let text = NSTextField::new(cx.mtm);
            text.setAlignment(objc2_app_kit::NSTextAlignment::Right);
            // The action fires on Return and when editing ends, so a typed value commits either way.
            if let Some(cell) = text.cell() {
                cell.setSendsActionOnEndEditing(true);
            }
            let formatter = NSNumberFormatter::new();
            formatter.setNumberStyle(NSNumberFormatterStyle::DecimalStyle);
            formatter.setUsesGroupingSeparator(false);
            let width = text.widthAnchor().constraintEqualToConstant(0.);
            width.setActive(true);
            text.setFormatter(Some(&formatter));
            Field { text, formatter, width }
        });
        let send = cx.clone();
        let owner = cx.clone();
        let (s, f, l, v) = (stepper.clone(), field.as_ref().map(|f| f.text.clone()), live.clone(), value.clone());
        let target = NdControlTarget::new(cx.mtm, move |sender| {
            let raw = sender.downcast_ref::<NSTextField>().map_or_else(|| s.doubleValue(), |t| t.doubleValue());
            let next = l.borrow().normalise(raw);
            s.setDoubleValue(next);
            if let Some(f) = &f {
                f.setDoubleValue(next);
            }
            if v.borrow().value() == Some(&next) {
                return;
            }
            v.borrow_mut().report(next);
            send.send(StepperChanged { value: next });
        });
        target.wire(&stepper);
        if let Some(f) = &field {
            target.wire(&f.text);
        }
        let root: Retained<NSView> = match &field {
            Some(f) => {
                let stack = NSStackView::new(cx.mtm);
                stack.setOrientation(NSUserInterfaceLayoutOrientation::Horizontal);
                stack.setAlignment(NSLayoutAttribute::CenterY);
                stack.addArrangedSubview(&f.text);
                stack.addArrangedSubview(&stepper);
                stack.into_super()
            }
            None => Retained::into_super(Retained::into_super(stepper.clone())),
        };
        let this = Self { root, stepper, field, _target: target, props: live, value, cx: owner };
        this.apply(None, props);
        this
    }

    fn apply(&self, old: Option<&StepperProps>, new: &StepperProps) {
        let stepper = &self.stepper;
        stepper.setMinValue(new.min.min(new.max));
        stepper.setMaxValue(new.max.max(new.min));
        stepper.setIncrement(new.step);
        stepper.setValueWraps(new.wraps);
        stepper.setAutorepeat(true);
        apply_common(Some(stepper), &self.root, old.map(|o| (o.enabled, &o.tooltip)), new.enabled, &new.tooltip);
        if old.is_none_or(|o| o.label != new.label) {
            set_accessibility_label(stepper, new.label.as_deref());
        }
        let next = new.normalise(new.value);
        let held = self.value.borrow_mut().apply(&next, |v| {
            stepper.setDoubleValue(*v);
            stepper.doubleValue()
        });
        let held = held.unwrap_or(next);
        if held != new.value && self.value.borrow().value() == Some(&held) && old.is_none_or(|o| o.value != new.value) {
            self.cx.send_normalised(StepperChanged { value: held });
        }
        if let Some(field) = &self.field {
            field.text.setEnabled(new.enabled);
            if old.is_none_or(|o| (o.field, o.min, o.max) != (new.field, new.min, new.max)) {
                self.fit_field(field, new);
            }
            if let Some(held) = self.value.borrow().value() {
                field.text.setDoubleValue(*held);
            }
            if old.is_none_or(|o| o.label != new.label) {
                set_accessibility_label(&field.text, new.label.as_deref());
            }
        }
    }

    /// Sizes the field for the widest value it can show, so it keeps its width as the value changes.
    fn fit_field(&self, field: &Field, props: &StepperProps) {
        let digits = usize::from(props.field.unwrap_or(0));
        field.formatter.setMinimumFractionDigits(digits);
        field.formatter.setMaximumFractionDigits(digits);
        let widest = [props.min, props.max]
            .into_iter()
            .filter_map(|v| field.formatter.stringFromNumber(&NSNumber::new_f64(v)))
            .max_by_key(|s| s.len());
        if let Some(widest) = widest {
            field.text.setStringValue(&widest);
            field.width.setConstant(0.);
            field.width.setConstant(field.text.fittingSize().width);
        }
    }
}

impl FormControl for StepperControl {
    fn view(&self) -> &NSView {
        &self.root
    }

    fn update(&mut self, old: &Props, new: &Props) -> bool {
        let (old, new): (&StepperProps, &StepperProps) = (props(old), props(new));
        if old.field.is_some() != new.field.is_some() {
            return false;
        }
        *self.props.borrow_mut() = new.clone();
        self.apply(Some(old), new);
        true
    }

    fn measure(&self, _width: Option<f32>) -> Size {
        fitting(&self.root)
    }

    fn probe(&self, probe: &mut Probe) {
        probe.enabled = Some(self.stepper.isEnabled());
        probe.value = Some(number_text(self.stepper.doubleValue(), self.props.borrow().field));
    }

    fn perform(&self, action: &SemanticAction) -> Option<Result<(), ActionError>> {
        let SemanticAction::SetValue(value) = action else { return None };
        let Some(v) = value.trim().parse::<f64>().ok().filter(|v| v.is_finite()) else {
            return Some(Err(ActionError::NotActionable(format!("\"{value}\" is not a number"))));
        };
        self.stepper.setDoubleValue(self.props.borrow().normalise(v));
        Some(send_action(&self.stepper))
    }
}
