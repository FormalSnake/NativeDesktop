//! Checkbox, switch, radio group, stepper and slider on the real AppKit controls.

use nd_backend_appkit::objc2::Message;
use nd_backend_appkit::objc2::rc::Retained;
use nd_backend_appkit::objc2::runtime::AnyObject;
use nd_backend_appkit::objc2_app_kit::{
    NSAccessibility, NSApplication, NSButton, NSControl, NSControlStateValueMixed, NSControlStateValueOff,
    NSControlStateValueOn, NSSegmentedControl, NSSlider, NSStackView, NSStepper, NSSwitch, NSTextField,
    NSUserInterfaceLayoutOrientation, NSView,
};
use nd_backend_appkit::objc2_foundation::{MainThreadMarker, NSNumberFormatter};
use nd_components::{
    Mapping, SelectChanged, SliderChanged, StepperChanged, Toggled, checkbox, radio_group, slider, stepper, switch,
};
use nd_ui::Role;
use nd_ui::prelude::*;
use nd_ui::semantics::SemanticAction;

use super::{Checks, Rig};

struct Form {
    tip: bool,
    checked: bool,
    mixed: bool,
    on: bool,
    selected: usize,
    horizontal: bool,
    mapping: Mapping,
    stepped: f64,
    capped: f64,
    level: f32,
    log: Vec<String>,
}

impl Form {
    fn count(&self, key: &str) -> usize {
        self.log.iter().filter(|l| l.starts_with(key)).count()
    }

    fn last(&self, key: &str) -> Option<&String> {
        self.log.iter().rev().find(|l| l.starts_with(key))
    }
}

impl Render for Form {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tip = if self.tip { "one" } else { "two" };
        v_flex()
            .size_full()
            .gap_2()
            .p(px(12.))
            .child(checkbox("cb", "Check").checked(self.checked).mixed(self.mixed).tooltip(tip).on_toggle(cx.listener(
                |this, e: &Toggled, _, cx| {
                    this.log.push(format!("cb:{}", e.checked));
                    cx.notify();
                    this.checked = e.checked;
                    this.mixed = false;
                },
            )))
            .child(switch("sw").label("Wi-Fi").checked(self.on).tooltip(tip).on_toggle(cx.listener(
                |this, e: &Toggled, _, cx| {
                    this.log.push(format!("sw:{}", e.checked));
                    cx.notify();
                    // The app refuses to turn it off; the tooltip change is what makes the
                    // refusal reach the control, since equal props produce no update.
                    this.on = true;
                    this.tip = !this.tip;
                },
            )))
            .child({
                let mut group = radio_group("rg", ["Alpha", "Beta", "Gamma"])
                    .native(self.mapping)
                    .selected(self.selected)
                    .label("Letters")
                    .tooltip(tip);
                if self.horizontal {
                    group = group.horizontal();
                }
                group.on_change(cx.listener(|this, e: &SelectChanged, _, cx| {
                    this.log.push(format!("rg:{}", e.index));
                    cx.notify();
                    this.selected = e.index;
                }))
            })
            .child(
                stepper("st", self.stepped)
                    .range(0.0..=10.0)
                    .step(0.5)
                    .with_field(2)
                    .label("Amount")
                    .tooltip(tip)
                    .on_change(cx.listener(|this, e: &StepperChanged, _, cx| {
                        this.log.push(format!("st:{}", e.value));
                        cx.notify();
                        this.stepped = e.value;
                    })),
            )
            .child(stepper("cap", self.capped).range(0.0..=10.0).tooltip(tip).on_change(cx.listener(
                |this, e: &StepperChanged, _, cx| {
                    this.log.push(format!("cap:{}", e.value));
                    cx.notify();
                    // The app caps the value below the stepper's own maximum.
                    this.capped = e.value.min(5.);
                },
            )))
            .child(slider("sl", self.level).step(0.25).tooltip(tip).on_change(cx.listener(
                |this, e: &SliderChanged, _, cx| {
                    this.log.push(format!("sl:{}", e.value));
                    cx.notify();
                    this.level = e.value;
                },
            )))
            .child(slider("sv", 0.5).ticks(3).vertical())
    }
}

/// Sends the control's own action to its own target, as a click does after changing its state.
fn fire(control: &NSControl) {
    let mtm = MainThreadMarker::new().expect("main thread");
    let sender: &AnyObject = control;
    let action = control.action().expect("control action");
    unsafe {
        NSApplication::sharedApplication(mtm).sendAction_to_from(action, control.target().as_deref(), Some(sender));
    }
}

fn as_button(view: &NSView) -> Retained<NSButton> {
    view.downcast_ref::<NSButton>().expect("an NSButton").retain()
}

fn buttons(stack: &NSView) -> Vec<Retained<NSButton>> {
    let stack = stack.downcast_ref::<NSStackView>().expect("an NSStackView");
    stack.arrangedSubviews().iter().filter_map(|v| v.downcast_ref::<NSButton>().map(|b| b.retain())).collect()
}

pub fn run(rig: &Rig, c: &mut Checks) {
    let opened = rig.open("toggles", |_| Form {
        tip: false,
        checked: false,
        mixed: false,
        on: false,
        selected: 1,
        horizontal: false,
        mapping: Mapping::Radio,
        stepped: 1.5,
        capped: 2.,
        level: 0.5,
        log: Vec::new(),
    });
    let win = opened.id();
    let count = |key: &str| rig.read(&opened, |f| f.count(key));
    // A silent native change followed by an unrelated prop update: if the control were set
    // again with the value the app already holds, the change would be undone.
    let retip = || {
        rig.update(&opened, |f, cx| {
            f.tip = !f.tip;
            cx.notify();
        });
        rig.pump();
    };

    // checkbox
    let view = rig.view(win, "cb");
    let cb = as_button(&view);
    c.check(
        cb.title().to_string() == "Check" && cb.state() == NSControlStateValueOff,
        "checkbox: NSButton with its title, off",
    );
    let node = rig.node(win, "cb").expect("cb node");
    c.check(
        node.role == Role::Checkbox && node.checked == Some(false) && node.enabled,
        "checkbox: role, checked, enabled",
    );
    rig.update(&opened, |f, cx| {
        f.mixed = true;
        cx.notify();
    });
    rig.pump();
    c.check(cb.allowsMixedState() && cb.state() == NSControlStateValueMixed, "checkbox: mixed shows the mixed state");
    c.check(rig.node(win, "cb").and_then(|n| n.value).as_deref() == Some("mixed"), "checkbox: mixed value");
    rig.update(&opened, |f, cx| {
        f.mixed = false;
        cx.notify();
    });
    rig.pump();
    c.check(!cb.allowsMixedState() && cb.state() == NSControlStateValueOff, "checkbox: mixed cleared");
    cb.setState(NSControlStateValueOn);
    fire(&cb);
    rig.pump();
    c.check(
        count("cb") == 1 && rig.read(&opened, |f| f.last("cb").cloned()).as_deref() == Some("cb:true"),
        "checkbox: user toggle emits Toggled once",
    );
    cb.setState(NSControlStateValueOff);
    retip();
    c.check(cb.state() == NSControlStateValueOff && count("cb") == 1, "checkbox: echo sets nothing");
    c.check(
        rig.perform(win, "cb", SemanticAction::Click).is_ok() && count("cb") == 2,
        "checkbox: Click acts through the control",
    );
    c.check(
        cb.state() == NSControlStateValueOn && rig.node(win, "cb").and_then(|n| n.checked) == Some(true),
        "checkbox: Click result",
    );
    c.check(
        rig.perform(win, "cb", SemanticAction::SetValue("off".into())).is_ok() && cb.state() == NSControlStateValueOff,
        "checkbox: SetValue",
    );

    // switch
    let view = rig.view(win, "sw");
    let sw = view.downcast_ref::<NSSwitch>().expect("an NSSwitch").retain();
    c.check(
        sw.accessibilityLabel().is_some_and(|l| l.to_string() == "Wi-Fi"),
        "switch: NSSwitch with its accessibility label",
    );
    c.check(
        rig.node(win, "sw").is_some_and(|n| n.role == Role::Switch && n.checked == Some(false)),
        "switch: role and checked",
    );
    sw.setState(NSControlStateValueOn);
    fire(&sw);
    rig.pump();
    c.check(count("sw") == 1 && sw.state() == NSControlStateValueOn, "switch: user toggle emits Toggled once");
    sw.setState(NSControlStateValueOff);
    fire(&sw);
    rig.pump();
    rig.pump();
    c.check(
        count("sw") == 2 && sw.state() == NSControlStateValueOn,
        "switch: a handler that refuses the change neither loops nor emits twice",
    );
    c.check(rig.perform(win, "sw", SemanticAction::Click).is_ok() && count("sw") == 3, "switch: Click reaches the app");

    // radio group
    let stack = rig.view(win, "rg");
    let radios = buttons(&stack);
    let titles: Vec<String> = radios.iter().map(|b| b.title().to_string()).collect();
    c.check(titles == ["Alpha", "Beta", "Gamma"], "radio: one radio NSButton per option");
    c.check(
        radios.iter().map(|b| b.state()).collect::<Vec<_>>() == [0, 1, 0],
        "radio: selected option reaches the control",
    );
    let node = rig.node(win, "rg").expect("rg node");
    c.check(node.role == Role::RadioGroup && node.value.as_deref() == Some("Beta"), "radio: role and value");
    c.check(
        stack
            .downcast_ref::<NSStackView>()
            .is_some_and(|s| s.orientation() == NSUserInterfaceLayoutOrientation::Vertical),
        "radio: stacked",
    );
    rig.update(&opened, |f, cx| {
        f.horizontal = true;
        cx.notify();
    });
    rig.pump();
    c.check(
        stack
            .downcast_ref::<NSStackView>()
            .is_some_and(|s| s.orientation() == NSUserInterfaceLayoutOrientation::Horizontal),
        "radio: horizontal",
    );
    unsafe { radios[2].performClick(None) };
    rig.pump();
    c.check(
        count("rg") == 1 && rig.read(&opened, |f| f.last("rg").cloned()).as_deref() == Some("rg:2"),
        "radio: click emits SelectChanged once",
    );
    c.check(radios.iter().map(|b| b.state()).collect::<Vec<_>>() == [0, 0, 1], "radio: the group is exclusive");
    radios[1].setState(NSControlStateValueOn);
    retip();
    c.check(radios[1].state() == NSControlStateValueOn && count("rg") == 1, "radio: echo sets nothing");
    unsafe { radios[2].performClick(None) };
    rig.pump();
    c.check(count("rg") == 1, "radio: clicking the chosen option again emits nothing");
    c.check(rig.perform(win, "rg", SemanticAction::SetValue("Alpha".into())).is_ok(), "radio: SetValue acts");
    c.check(
        count("rg") == 2 && radios.iter().map(|b| b.state()).collect::<Vec<_>>() == [1, 0, 0],
        "radio: SetValue result",
    );
    rig.update(&opened, |f, cx| {
        f.mapping = Mapping::Segmented;
        cx.notify();
    });
    rig.pump();
    c.check(
        rig.view(win, "rg").downcast_ref::<NSSegmentedControl>().is_some(),
        "radio: switching the mapping recreates as segmented",
    );
    rig.update(&opened, |f, cx| {
        f.mapping = Mapping::Radio;
        cx.notify();
    });
    rig.pump();
    c.check(buttons(&rig.view(win, "rg")).len() == 3, "radio: and back to a radio group");

    // stepper with a field
    let stack = rig.view(win, "st");
    let parts = stack.downcast_ref::<NSStackView>().expect("stepper with a field is a stack").arrangedSubviews();
    let field = parts.objectAtIndex(0).downcast_ref::<NSTextField>().expect("the field").retain();
    let st = parts.objectAtIndex(1).downcast_ref::<NSStepper>().expect("the stepper").retain();
    c.check(
        st.minValue() == 0. && st.maxValue() == 10. && st.increment() == 0.5,
        "stepper: range and step reach the control",
    );
    c.check(field.doubleValue() == 1.5 && st.doubleValue() == 1.5, "stepper: value shown in both");
    c.check(
        field.formatter().and_then(|f| f.downcast_ref::<NSNumberFormatter>().map(|f| f.minimumFractionDigits()))
            == Some(2),
        "stepper: field formats with its digits",
    );
    c.check(
        rig.node(win, "st").is_some_and(|n| n.role == Role::Incrementor && n.value.as_deref() == Some("1.50")),
        "stepper: role and value",
    );
    st.setDoubleValue(2.5);
    fire(&st);
    rig.pump();
    c.check(count("st") == 1 && field.doubleValue() == 2.5, "stepper: arrows emit once and update the field");
    st.setDoubleValue(7.);
    retip();
    c.check(st.doubleValue() == 7. && count("st") == 1, "stepper: echo sets nothing");
    field.setDoubleValue(50.);
    fire(&field);
    rig.pump();
    c.check(
        count("st") == 2
            && rig.read(&opened, |f| f.last("st").cloned()).as_deref() == Some("st:10")
            && st.doubleValue() == 10.,
        "stepper: a typed value past max emits the clamped value once",
    );
    rig.update(&opened, |f, cx| {
        f.stepped = 99.;
        cx.notify();
    });
    rig.pump();
    c.check(st.doubleValue() == 10. && count("st") == 2, "stepper: an app value past max is clamped without an event");
    c.check(
        rig.perform(win, "st", SemanticAction::SetValue("4".into())).is_ok() && count("st") == 3,
        "stepper: SetValue reaches the app",
    );
    c.check(
        rig.node(win, "st").and_then(|n| n.value).as_deref() == Some("4.00") && field.doubleValue() == 4.,
        "stepper: SetValue result",
    );

    // stepper without a field, and a handler that rewrites the value
    let cap = rig.view(win, "cap").downcast_ref::<NSStepper>().expect("a bare NSStepper").retain();
    cap.setDoubleValue(8.);
    fire(&cap);
    rig.pump();
    rig.pump();
    c.check(
        count("cap") == 1 && cap.doubleValue() == 5.,
        "stepper: a handler that rewrites the value neither loops nor emits twice",
    );

    // slider
    let sl = rig.view(win, "sl").downcast_ref::<NSSlider>().expect("an NSSlider").retain();
    c.check(sl.numberOfTickMarks() == 5 && sl.allowsTickMarkValuesOnly(), "slider: a step becomes snapping tick marks");
    sl.setDoubleValue(0.37);
    fire(&sl);
    rig.pump();
    let said = rig.read(&opened, |f| f.last("sl").cloned());
    c.check(
        count("sl") == 1 && said.as_deref().is_some_and(|s| s == "sl:0.25" || s == "sl:0.5"),
        "slider: a value between steps emits one snapped event",
    );
    let snapped = sl.doubleValue();
    sl.setDoubleValue(0.75);
    retip();
    c.check(sl.doubleValue() == 0.75 && snapped != 0.75 && count("sl") == 1, "slider: echo sets nothing");
    c.check(rig.perform(win, "sl", SemanticAction::SetValue("0.6".into())).is_ok(), "slider: SetValue acts");
    c.check(
        rig.read(&opened, |f| f.last("sl").cloned()).as_deref() == Some("sl:0.5") && count("sl") == 2,
        "slider: SetValue snaps to the step",
    );
    let sv = rig.view(win, "sv").downcast_ref::<NSSlider>().expect("an NSSlider").retain();
    c.check(
        sv.isVertical() && sv.numberOfTickMarks() == 3 && !sv.allowsTickMarkValuesOnly(),
        "slider: vertical with ticks and no snapping",
    );

    rig.close(opened);
}
