//! Tier 1 behaviour of the forms family on `TestBackend`: widget kind and props at the seam,
//! the controlled round trip, the semantic node, and what automation's actions produce.

mod common;

use common::*;
use nd_components::forms::*;
use nd_components::{Icon, Mapping};
use nd_ui::Role;
use nd_ui::prelude::*;
use nd_ui::semantics::{ActionError, SemanticAction};

#[derive(Default)]
struct Text {
    text: String,
    changes: u32,
    submits: Vec<String>,
    icon_clicks: Vec<IconEdge>,
}

fn text_view(s: &Text, cx: &mut Context<Host<Text>>) -> AnyElement {
    v_flex()
        .child(
            text_field("name")
                .text(s.text.clone())
                .placeholder("Full name")
                .label("Name")
                .leading_icon(Icon::User, "Account")
                .trailing_icon(Icon::X, "Clear")
                .on_icon_click(cx.listener(|h, e: &FieldIconClicked, _, cx| {
                    h.s.icon_clicks.push(e.edge);
                    cx.notify();
                }))
                .on_change(cx.listener(|h, e: &TextChanged, _, cx| {
                    h.s.text = e.text.to_string();
                    h.s.changes += 1;
                    cx.notify();
                }))
                .on_submit(cx.listener(|h, e: &TextSubmitted, _, cx| {
                    h.s.submits.push(e.text.to_string());
                    cx.notify();
                })),
        )
        .into_any_element()
}

#[test]
fn text_field_reports_kind_props_and_semantics() {
    let mut rig = Rig::new(Text { text: "Ada".into(), ..Text::default() }, text_view);
    assert!(rig.kind_is::<TextFieldKind>("name"));
    let props = rig.props::<TextFieldProps>("name");
    assert_eq!((props.text.as_ref(), props.field, props.enabled), ("Ada", FieldStyle::Plain, true));
    assert_eq!(props.placeholder.as_deref(), Some("Full name"));
    assert_eq!(props.leading.as_ref().map(|i| (i.icon, i.label.as_ref())), Some((Icon::User, "Account")));
    assert_eq!(props.trailing.as_ref().map(|i| i.icon), Some(Icon::X));

    let node = rig.sem("name");
    assert_eq!((node.role, node.name.as_str(), node.value.as_deref()), (Role::TextField, "Name", Some("Ada")));
    assert_eq!(node.placeholder.as_deref(), Some("Full name"));
    assert!(node.enabled);

    rig.emit("name", FieldIconClicked { edge: IconEdge::Trailing });
    assert_eq!(rig.state().icon_clicks, [IconEdge::Trailing]);
    rig.emit("name", TextSubmitted { text: "Ada".into() });
    assert_eq!(rig.state().submits, ["Ada"]);
}

#[test]
fn text_field_edit_round_trips_with_one_update_and_feeding_back_changes_nothing() {
    let mut rig = Rig::new(Text::default(), text_view);
    rig.fresh();
    rig.emit("name", TextChanged { text: "Grace".into() });
    assert_eq!(rig.state().text, "Grace");
    assert_eq!(rig.updates("name"), 1, "the new text reaches the backend once");
    assert_eq!(rig.sem("name").value.as_deref(), Some("Grace"));

    rig.fresh();
    rig.emit("name", TextChanged { text: "Grace".into() });
    assert_eq!(rig.state().changes, 2, "the handler ran again");
    assert_eq!(rig.all_updates(), 0, "an equal value fed back sets nothing");
    rig.rerender();
    assert_eq!(rig.all_updates(), 0);
}

#[test]
fn text_field_set_value_and_type_come_through_respond() {
    let mut rig = Rig::new(Text { text: "ab".into(), ..Text::default() }, text_view);
    rig.perform("name", SemanticAction::Type("c".into())).unwrap();
    assert_eq!(rig.state().text, "abc");
    rig.set_value("name", "xyz").unwrap();
    assert_eq!(rig.state().text, "xyz");
    assert_eq!(rig.sem("name").value.as_deref(), Some("xyz"));
    assert!(matches!(rig.perform("name", SemanticAction::Click), Err(ActionError::Unsupported(_))));
}

#[test]
fn disabled_and_read_only_fields_refuse_text() {
    fn view(s: &Text, cx: &mut Context<Host<Text>>) -> AnyElement {
        let on_change = cx.listener(|h, e: &TextChanged, _, cx| {
            h.s.text = e.text.to_string();
            cx.notify();
        });
        v_flex()
            .child(text_field("off").text(s.text.clone()).disabled(true).on_change(on_change))
            .child(text_field("ro").text("fixed").read_only(true))
            .into_any_element()
    }
    let mut rig = Rig::new(Text::default(), view);
    assert!(!rig.sem("off").enabled);
    assert!(refused(rig.set_value("off", "x")));
    assert!(refused(rig.perform("off", SemanticAction::Type("x".into()))));
    assert_eq!(rig.state().text, "");
    assert!(rig.sem("ro").enabled);
    assert!(refused(rig.set_value("ro", "x")));
    assert!(!rig.props::<TextFieldProps>("ro").editable);
}

#[test]
fn secure_field_never_exposes_its_text() {
    fn view(s: &Text, cx: &mut Context<Host<Text>>) -> AnyElement {
        secure_field("pw")
            .text(s.text.clone())
            .placeholder("Password")
            .label("Password")
            .on_change(cx.listener(|h, e: &TextChanged, _, cx| {
                h.s.text = e.text.to_string();
                cx.notify();
            }))
            .into_any_element()
    }
    let mut rig = Rig::new(Text { text: "hunter2".into(), ..Text::default() }, view);
    assert_eq!(rig.props::<TextFieldProps>("pw").field, FieldStyle::Secure);
    let node = rig.sem("pw");
    assert_eq!((node.role, node.value, node.placeholder.as_deref()), (Role::TextField, None, Some("Password")));
    rig.perform("pw", SemanticAction::Type("!".into())).unwrap();
    assert_eq!(rig.state().text, "hunter2!", "typing still reaches the field");
    assert_eq!(rig.sem("pw").value, None);
}

#[test]
fn text_area_takes_several_lines() {
    fn view(s: &Text, cx: &mut Context<Host<Text>>) -> AnyElement {
        text_area("notes")
            .text(s.text.clone())
            .placeholder("Notes")
            .label("Notes")
            .min_lines(5)
            .on_change(cx.listener(|h, e: &TextChanged, _, cx| {
                h.s.text = e.text.to_string();
                cx.notify();
            }))
            .into_any_element()
    }
    let mut rig = Rig::new(Text::default(), view);
    assert!(rig.kind_is::<TextAreaKind>("notes"));
    assert_eq!(rig.props::<TextAreaProps>("notes").min_lines, 5);
    assert_eq!(rig.sem("notes").role, Role::TextArea);
    rig.set_value("notes", "one\ntwo").unwrap();
    assert_eq!(rig.state().text, "one\ntwo");
    rig.fresh();
    rig.emit("notes", TextChanged { text: "one\ntwo".into() });
    assert_eq!(rig.all_updates(), 0);
}

#[derive(Default)]
struct Combo {
    text: String,
    picked: Option<usize>,
}

#[test]
fn combo_box_fills_its_text_and_reports_the_pick() {
    fn view(s: &Combo, cx: &mut Context<Host<Combo>>) -> AnyElement {
        combo_box("tz", ["UTC", "Europe/Madrid", "Atlantic/Canary"])
            .text(s.text.clone())
            .placeholder("Time zone")
            .label("Time zone")
            .completes(false)
            .on_change(cx.listener(|h, e: &TextChanged, _, cx| {
                h.s.text = e.text.to_string();
                cx.notify();
            }))
            .on_select(cx.listener(|h, e: &SelectChanged, _, cx| {
                h.s.picked = Some(e.index);
                cx.notify();
            }))
            .into_any_element()
    }
    let mut rig = Rig::new(Combo::default(), view);
    assert!(rig.kind_is::<ComboBoxKind>("tz"));
    let props = rig.props::<ComboBoxProps>("tz");
    assert_eq!((props.options.len(), props.completes), (3, false));
    assert_eq!(rig.sem("tz").role, Role::ComboBox);
    assert_eq!(rig.sem("tz").placeholder.as_deref(), Some("Time zone"));

    rig.emit("tz", TextChanged { text: "Europe/Madrid".into() });
    rig.emit("tz", SelectChanged { index: 1 });
    assert_eq!((rig.state().text.as_str(), rig.state().picked), ("Europe/Madrid", Some(1)));
    assert_eq!(rig.sem("tz").value.as_deref(), Some("Europe/Madrid"));
    rig.set_value("tz", "custom").unwrap();
    assert_eq!(rig.state().text, "custom", "the text is free");
}

type Bound = Entity<TextFieldState>;

fn bound_view(s: &Bound, cx: &mut Context<Host<Bound>>) -> AnyElement {
    v_flex()
        .child(search_field("find", s).label("Find"))
        .child(text_field("plain").bind(s).label("Mirror"))
        .child(button("reset", "Reset").on_click(cx.listener(|h, _: &ClickEvent, window, cx| {
            h.s.update(cx, |state, cx| state.set_text("", window, cx));
        })))
        .into_any_element()
}

/// The view that keeps the entity observes it, so a change made anywhere renders the fields again.
fn bound_rig(state: TextFieldState) -> Rig<Bound> {
    let mut rig = Rig::build(|cx| cx.new(|_| state), bound_view);
    rig.win.root().clone().update(&mut rig.cx, |host, cx| {
        cx.observe(&host.s, |_, _, cx| cx.notify()).detach();
    });
    rig
}

#[test]
fn bound_fields_read_the_entity_and_edits_update_it() {
    let mut rig = bound_rig(TextFieldState::default().placeholder("Search notes"));
    assert!(rig.kind_is::<TextFieldKind>("find"));
    let find = rig.sem("find");
    assert_eq!((find.role, find.placeholder.as_deref()), (Role::SearchField, Some("Search notes")));
    assert_eq!(rig.props::<TextFieldProps>("find").field, FieldStyle::Search);

    rig.emit("find", TextChanged { text: "rust".into() });
    let state = rig.state().clone();
    assert_eq!(state.read(&rig.cx).text().as_ref(), "rust");
    assert_eq!(state.read(&rig.cx).last_event(), Some(TextEvent::Edited));
    assert_eq!(rig.sem("find").value.as_deref(), Some("rust"));
    assert_eq!(rig.sem("plain").value.as_deref(), Some("rust"), "every field bound to the entity follows it");

    rig.emit("find", TextSubmitted { text: "rust".into() });
    assert_eq!(state.read(&rig.cx).last_event(), Some(TextEvent::Submitted));
    assert_eq!(state.read(&rig.cx).text().as_ref(), "rust", "submitting keeps the text");
}

#[test]
fn set_text_records_replaced_and_reaches_the_field() {
    let mut rig = bound_rig(TextFieldState::default());
    rig.emit("find", TextChanged { text: "draft".into() });
    assert_eq!(rig.sem("find").value.as_deref(), Some("draft"));
    rig.perform("reset", SemanticAction::Click).unwrap();
    let state = rig.state().clone();
    assert_eq!(state.read(&rig.cx).last_event(), Some(TextEvent::Replaced));
    assert_eq!(state.read(&rig.cx).text().as_ref(), "");
    assert_eq!(rig.sem("find").value.as_deref(), Some(""));
}

#[derive(Default)]
struct Toggles {
    remember: bool,
    all: bool,
    wifi: bool,
    log: Vec<bool>,
}

fn toggle_view(s: &Toggles, cx: &mut Context<Host<Toggles>>) -> AnyElement {
    v_flex()
        .child(checkbox("remember", "Remember me").checked(s.remember).on_toggle(cx.listener(
            |h, e: &Toggled, _, cx| {
                h.s.remember = e.checked;
                h.s.log.push(e.checked);
                cx.notify();
            },
        )))
        .child(checkbox("all", "Select all").checked(s.all).mixed(!s.all).on_toggle(cx.listener(
            |h, e: &Toggled, _, cx| {
                h.s.all = e.checked;
                cx.notify();
            },
        )))
        .child(checkbox("locked", "Locked").checked(true).disabled(true))
        .child(switch("wifi").label("Wi-Fi").checked(s.wifi).on_toggle(cx.listener(|h, e: &Toggled, _, cx| {
            h.s.wifi = e.checked;
            cx.notify();
        })))
        .child(switch("airplane").label("Airplane mode").disabled(true))
        .into_any_element()
}

#[test]
fn checkbox_reports_kind_props_state_and_toggles() {
    let mut rig = Rig::new(Toggles::default(), toggle_view);
    assert!(rig.kind_is::<CheckboxKind>("remember"));
    assert_eq!(rig.props::<CheckboxProps>("remember").label.as_ref(), "Remember me");
    let node = rig.sem("remember");
    assert_eq!((node.role, node.name.as_str(), node.value.as_deref()), (Role::Checkbox, "Remember me", Some("false")));
    assert_eq!(node.checked, Some(false));

    rig.fresh();
    rig.emit("remember", Toggled { checked: true });
    assert!(rig.state().remember);
    assert_eq!(rig.updates("remember"), 1);
    assert_eq!(rig.sem("remember").checked, Some(true));
    assert_eq!(rig.sem("remember").value.as_deref(), Some("true"));

    rig.fresh();
    rig.emit("remember", Toggled { checked: true });
    assert_eq!(rig.all_updates(), 0, "an equal state fed back sets nothing");

    rig.perform("remember", SemanticAction::Click).unwrap();
    assert!(!rig.state().remember, "click flips it");
    rig.set_value("remember", "on").unwrap();
    rig.set_value("remember", "NO").unwrap();
    assert_eq!(rig.state().log, [true, true, false, true, false]);
    assert!(refused(rig.set_value("remember", "maybe")));
}

#[test]
fn mixed_checkbox_checks_on_click() {
    let mut rig = Rig::new(Toggles::default(), toggle_view);
    let node = rig.sem("all");
    assert_eq!((node.value.as_deref(), node.checked), (Some("mixed"), Some(false)));
    rig.perform("all", SemanticAction::Click).unwrap();
    assert!(rig.state().all);
    assert_eq!(rig.sem("all").value.as_deref(), Some("true"));
}

#[test]
fn disabled_toggles_refuse_and_report_disabled() {
    let mut rig = Rig::new(Toggles::default(), toggle_view);
    for id in ["locked", "airplane"] {
        assert!(!rig.sem(id).enabled, "{id}");
        assert!(refused(rig.perform(id, SemanticAction::Click)), "{id}");
        assert!(refused(rig.set_value(id, "true")), "{id}");
    }
}

#[test]
fn switch_reports_kind_name_and_toggles() {
    let mut rig = Rig::new(Toggles::default(), toggle_view);
    assert!(rig.kind_is::<SwitchKind>("wifi"));
    let node = rig.sem("wifi");
    assert_eq!((node.role, node.name.as_str(), node.checked), (Role::Switch, "Wi-Fi", Some(false)));
    rig.emit("wifi", Toggled { checked: true });
    assert!(rig.state().wifi);
    assert_eq!(rig.sem("wifi").checked, Some(true));
    rig.perform("wifi", SemanticAction::Click).unwrap();
    assert!(!rig.state().wifi);
    rig.set_value("wifi", "1").unwrap();
    assert!(rig.state().wifi);
    assert!(refused(rig.set_value("wifi", "sideways")));
}

#[derive(Default)]
struct Choice {
    pop: usize,
    seg: usize,
    radio: usize,
}

fn choice_view(s: &Choice, cx: &mut Context<Host<Choice>>) -> AnyElement {
    v_flex()
        .child(select("pop", ["Name", "Date", "Size"]).selected(s.pop).label("Sort by").on_change(cx.listener(
            |h, e: &SelectChanged, _, cx| {
                h.s.pop = e.index;
                cx.notify();
            },
        )))
        .child(segmented("seg", ["Day", "Week", "Month"]).selected(s.seg).label("Range").on_change(cx.listener(
            |h, e: &SelectChanged, _, cx| {
                h.s.seg = e.index;
                cx.notify();
            },
        )))
        .child(radio_group("radio", ["Small", "Large"]).selected(s.radio).horizontal().label("Size").on_change(
            cx.listener(|h, e: &SelectChanged, _, cx| {
                h.s.radio = e.index;
                cx.notify();
            }),
        ))
        .child(select("off", ["A", "B"]).disabled(true))
        .child(select("forced", ["A", "B"]).native(Mapping::Segmented))
        .into_any_element()
}

#[test]
fn select_family_shares_a_kind_and_picks_the_role_from_the_mapping() {
    let rig = Rig::new(Choice::default(), choice_view);
    for id in ["pop", "seg", "radio"] {
        assert!(rig.kind_is::<SelectKind>(id), "{id}");
    }
    assert_eq!(rig.props::<SelectProps>("pop").mapping, Mapping::Automatic);
    assert_eq!(rig.props::<SelectProps>("seg").mapping, Mapping::Segmented);
    assert!(rig.props::<SelectProps>("radio").horizontal);
    let roles: Vec<_> = ["pop", "seg", "radio", "forced"].map(|id| rig.sem(id).role).into();
    assert_eq!(roles, [Role::PopUpButton, Role::SegmentedControl, Role::RadioGroup, Role::SegmentedControl]);
    assert_eq!(rig.sem("pop").value.as_deref(), Some("Name"));
    assert_eq!(rig.sem("seg").name, "Range");
}

#[test]
fn select_event_round_trips_and_normalises_nothing_extra() {
    let mut rig = Rig::new(Choice::default(), choice_view);
    rig.fresh();
    rig.emit("pop", SelectChanged { index: 2 });
    assert_eq!(rig.state().pop, 2);
    assert_eq!(rig.updates("pop"), 1);
    assert_eq!(rig.sem("pop").value.as_deref(), Some("Size"));
    rig.fresh();
    rig.emit("pop", SelectChanged { index: 2 });
    assert_eq!(rig.all_updates(), 0);
}

#[test]
fn select_set_value_takes_a_title_in_any_case_or_an_index() {
    let mut rig = Rig::new(Choice::default(), choice_view);
    rig.set_value("pop", "Date").unwrap();
    assert_eq!(rig.state().pop, 1);
    rig.set_value("pop", "size").unwrap();
    assert_eq!(rig.state().pop, 2);
    rig.set_value("seg", "2").unwrap();
    assert_eq!(rig.state().seg, 2);
    rig.set_value("radio", "Large").unwrap();
    assert_eq!(rig.sem("radio").value.as_deref(), Some("Large"));
    assert!(refused(rig.set_value("pop", "Colour")), "an unknown option");
    assert!(refused(rig.set_value("pop", "3")), "an index past the end");
    assert_eq!(rig.state().pop, 2);
    assert!(refused(rig.set_value("off", "A")), "a disabled select");
    assert!(!rig.sem("off").enabled);
}

#[derive(Default)]
struct Num {
    vol: f32,
    wrap: f64,
    clamp: f64,
    changes: u32,
}

fn num_view(s: &Num, cx: &mut Context<Host<Num>>) -> AnyElement {
    v_flex()
        .child(slider("vol", s.vol).range(0.0..=1.0).step(0.25).ticks(5).label("Volume").on_change(cx.listener(
            |h, e: &SliderChanged, _, cx| {
                h.s.vol = e.value;
                h.s.changes += 1;
                cx.notify();
            },
        )))
        .child(slider("free", 0.3).label("Free").disabled(true))
        .child(stepper("wrap", s.wrap).range(0.0..=10.0).wraps(true).with_field(1).label("Wrapping").on_change(
            cx.listener(|h, e: &StepperChanged, _, cx| {
                h.s.wrap = e.value;
                cx.notify();
            }),
        ))
        .child(stepper("clamp", s.clamp).range(0.0..=10.0).step(2.).label("Clamped").on_change(cx.listener(
            |h, e: &StepperChanged, _, cx| {
                h.s.clamp = e.value;
                cx.notify();
            },
        )))
        .child(stepper("off", 1.).disabled(true))
        .into_any_element()
}

#[test]
fn slider_reports_kind_props_and_semantics() {
    let rig = Rig::new(Num { vol: 0.5, ..Num::default() }, num_view);
    assert!(rig.kind_is::<SliderKind>("vol"));
    let props = rig.props::<SliderProps>("vol");
    assert_eq!((props.min, props.max, props.step, props.ticks), (0., 1., Some(0.25), 5));
    assert!(!props.drawn && !props.vertical);
    let node = rig.sem("vol");
    assert_eq!((node.role, node.name.as_str(), node.value.as_deref()), (Role::Slider, "Volume", Some("0.5")));
}

#[test]
fn slider_event_round_trips_with_one_update() {
    let mut rig = Rig::new(Num::default(), num_view);
    rig.fresh();
    rig.emit("vol", SliderChanged { value: 0.75 });
    assert_eq!(rig.state().vol, 0.75);
    assert_eq!(rig.updates("vol"), 1);
    assert_eq!(rig.sem("vol").value.as_deref(), Some("0.75"));
    rig.fresh();
    rig.emit("vol", SliderChanged { value: 0.75 });
    assert_eq!(rig.all_updates(), 0);
}

#[test]
fn slider_set_value_snaps_to_the_step_and_clamps() {
    let mut rig = Rig::new(Num::default(), num_view);
    rig.set_value("vol", "0.74").unwrap();
    assert_eq!(rig.state().vol, 0.75, "snapped to the nearest step");
    rig.fresh();
    rig.set_value("vol", "0.74").unwrap();
    assert_eq!(rig.all_updates(), 0, "the snapped value fed back is the value already held");
    rig.set_value("vol", "7").unwrap();
    assert_eq!(rig.state().vol, 1.);
    rig.set_value("vol", "-3").unwrap();
    assert_eq!(rig.state().vol, 0.);
    rig.set_value("vol", " 0.5 ").unwrap();
    assert_eq!(rig.state().vol, 0.5);
}

#[test]
fn slider_refuses_text_and_a_disabled_control() {
    let mut rig = Rig::new(Num { vol: 0.5, ..Num::default() }, num_view);
    let before = rig.state().changes;
    assert!(refused(rig.set_value("vol", "abc")));
    assert!(refused(rig.set_value("vol", "NaN")));
    assert!(refused(rig.set_value("vol", "")));
    assert_eq!(rig.state().changes, before);
    assert!(refused(rig.set_value("free", "0.5")));
    assert!(!rig.sem("free").enabled);
    assert!(matches!(rig.perform("vol", SemanticAction::Click), Err(ActionError::Unsupported(_))));
}

#[test]
fn stepper_reports_kind_props_and_the_field_digits() {
    let rig = Rig::new(Num { wrap: 3., clamp: 4., ..Num::default() }, num_view);
    assert!(rig.kind_is::<StepperKind>("wrap"));
    let props = rig.props::<StepperProps>("wrap");
    assert_eq!((props.min, props.max, props.wraps, props.field), (0., 10., true, Some(1)));
    let node = rig.sem("wrap");
    assert_eq!((node.role, node.name.as_str(), node.value.as_deref()), (Role::Incrementor, "Wrapping", Some("3.0")));
    assert_eq!(rig.sem("clamp").value.as_deref(), Some("4"));
}

#[test]
fn stepper_wraps_or_clamps_what_automation_sets() {
    let mut rig = Rig::new(Num { wrap: 3., clamp: 4., ..Num::default() }, num_view);
    rig.set_value("wrap", "11").unwrap();
    assert_eq!(rig.state().wrap, 0., "past the maximum wraps to the minimum");
    rig.set_value("wrap", "-1").unwrap();
    assert_eq!(rig.state().wrap, 10., "below the minimum wraps to the maximum");
    rig.set_value("wrap", "7.5").unwrap();
    assert_eq!(rig.state().wrap, 7.5);
    rig.set_value("clamp", "99").unwrap();
    assert_eq!(rig.state().clamp, 10.);
    rig.set_value("clamp", "-4").unwrap();
    assert_eq!(rig.state().clamp, 0.);
    rig.fresh();
    rig.emit("clamp", StepperChanged { value: 0. });
    assert_eq!(rig.all_updates(), 0);
    rig.emit("clamp", StepperChanged { value: 2. });
    assert_eq!(rig.updates("clamp"), 1);
}

#[test]
fn stepper_refuses_text_and_a_disabled_control() {
    let mut rig = Rig::new(Num::default(), num_view);
    assert!(refused(rig.set_value("wrap", "many")));
    assert!(refused(rig.set_value("off", "3")));
    assert!(!rig.sem("off").enabled);
    assert_eq!(rig.state().wrap, 0.);
}
