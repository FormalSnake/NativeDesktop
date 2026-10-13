//! Tier 1 behaviour of the date and colour pickers.

mod common;

use common::*;
use nd_components::forms::*;
use nd_ui::Role;
use nd_ui::prelude::*;
use nd_ui::semantics::{ActionError, SemanticAction};

struct Dates {
    date: CivilDateTime,
    time: CivilDateTime,
    both: CivilDateTime,
}

fn dates() -> Dates {
    Dates {
        date: CivilDateTime::date(2026, 10, 13),
        time: CivilDateTime::date(2026, 10, 13).at(9, 30, 0),
        both: CivilDateTime::date(2026, 10, 13).at(9, 30, 0),
    }
}

fn date_view(s: &Dates, cx: &mut Context<Host<Dates>>) -> AnyElement {
    v_flex()
        .child(
            date_picker("date", s.date)
                .label("Due")
                .min(CivilDateTime::date(2026, 1, 1))
                .max(CivilDateTime::date(2026, 12, 31))
                .on_change(cx.listener(|h, e: &DateChanged, _, cx| {
                    h.s.date = e.value;
                    cx.notify();
                })),
        )
        .child(
            date_picker("time", s.time)
                .mode(DatePickerMode::Time)
                .presentation(DatePickerStyle::Stepper)
                .label("Alarm")
                .on_change(cx.listener(|h, e: &DateChanged, _, cx| {
                    h.s.time = e.value;
                    cx.notify();
                })),
        )
        .child(
            date_picker("both", s.both)
                .mode(DatePickerMode::DateTime)
                .presentation(DatePickerStyle::Graphical)
                .label("Starts")
                .on_change(cx.listener(|h, e: &DateChanged, _, cx| {
                    h.s.both = e.value;
                    cx.notify();
                })),
        )
        .child(date_picker("off", s.date).disabled(true))
        .into_any_element()
}

#[test]
fn date_picker_reports_kind_props_and_a_value_per_mode() {
    let rig = Rig::new(dates(), date_view);
    assert!(rig.kind_is::<DatePickerKind>("date"));
    let props = rig.props::<DatePickerProps>("date");
    assert_eq!((props.mode, props.presentation), (DatePickerMode::Date, DatePickerStyle::Field));
    assert_eq!(props.min, Some(CivilDateTime::date(2026, 1, 1)));
    assert_eq!(rig.props::<DatePickerProps>("time").presentation, DatePickerStyle::Stepper);
    assert_eq!(rig.props::<DatePickerProps>("both").presentation, DatePickerStyle::Graphical);

    let node = rig.sem("date");
    assert_eq!((node.role, node.name.as_str(), node.value.as_deref()), (Role::DatePicker, "Due", Some("2026-10-13")));
    assert_eq!(rig.sem("time").value.as_deref(), Some("09:30:00"));
    assert_eq!(rig.sem("both").value.as_deref(), Some("2026-10-13T09:30:00"));
}

#[test]
fn date_event_round_trips_with_one_update() {
    let mut rig = Rig::new(dates(), date_view);
    let next = CivilDateTime::date(2026, 11, 2);
    rig.fresh();
    rig.emit("date", DateChanged { value: next });
    assert_eq!(rig.state().date, next);
    assert_eq!(rig.updates("date"), 1);
    assert_eq!(rig.sem("date").value.as_deref(), Some("2026-11-02"));
    rig.fresh();
    rig.emit("date", DateChanged { value: next });
    assert_eq!(rig.all_updates(), 0);
}

#[test]
fn date_set_value_clamps_into_min_and_max() {
    let mut rig = Rig::new(dates(), date_view);
    rig.set_value("date", "2026-10-20").unwrap();
    assert_eq!(rig.state().date, CivilDateTime::date(2026, 10, 20));
    rig.set_value("date", "2027-03-01").unwrap();
    assert_eq!(rig.state().date, CivilDateTime::date(2026, 12, 31), "past the maximum");
    rig.set_value("date", "2025-05-05").unwrap();
    assert_eq!(rig.state().date, CivilDateTime::date(2026, 1, 1), "before the minimum");
    rig.fresh();
    rig.set_value("date", "2020-01-01").unwrap();
    assert_eq!(rig.all_updates(), 0, "the clamped value fed back is the value already held");
}

#[test]
fn time_and_date_time_pickers_parse_their_parts() {
    let mut rig = Rig::new(dates(), date_view);
    rig.set_value("time", "10:45").unwrap();
    assert_eq!(rig.state().time, CivilDateTime::date(2026, 10, 13).at(10, 45, 0), "the date stays");
    rig.set_value("time", "23:59:30").unwrap();
    assert_eq!(rig.sem("time").value.as_deref(), Some("23:59:30"));
    rig.set_value("both", "2026-12-24T18:15").unwrap();
    assert_eq!(rig.sem("both").value.as_deref(), Some("2026-12-24T18:15:00"));
    rig.set_value("both", "2026-12-25 08:00:05").unwrap();
    assert_eq!(rig.state().both, CivilDateTime::date(2026, 12, 25).at(8, 0, 5));
}

#[test]
fn date_picker_refuses_bad_dates_and_a_disabled_control() {
    let mut rig = Rig::new(dates(), date_view);
    for bad in ["tomorrow", "2026-13-01", "2026-02-40", "25:00", "10:61", ""] {
        assert!(refused(rig.set_value("date", bad)), "{bad:?}");
    }
    assert_eq!(rig.state().date, CivilDateTime::date(2026, 10, 13));
    assert!(refused(rig.set_value("off", "2026-10-14")));
    assert!(!rig.sem("off").enabled);
    assert!(matches!(rig.perform("date", SemanticAction::Click), Err(ActionError::Unsupported(_))));
}

#[test]
fn civil_date_time_orders_and_prints_chronologically() {
    let a = CivilDateTime::date(2026, 10, 13).at(9, 30, 0);
    let b = CivilDateTime::date(2026, 10, 13).at(9, 30, 1);
    assert!(a < b && b < CivilDateTime::date(2026, 10, 14));
    assert_eq!(a.to_string(), "2026-10-13T09:30:00");
    assert_eq!(CivilDateTime::parse("07:05", a), Some(a.at(7, 5, 0)));
    assert_eq!(CivilDateTime::parse("2027-01-02", a), Some(CivilDateTime::date(2027, 1, 2)));
}

struct Colours {
    solid: Hsla,
    alpha: Hsla,
}

fn rgb_of(r: f32, g: f32, b: f32, a: f32) -> Hsla {
    Rgba { r, g, b, a }.into()
}

fn colour_view(s: &Colours, cx: &mut Context<Host<Colours>>) -> AnyElement {
    v_flex()
        .child(color_well("solid", s.solid).label("Tint").on_change(cx.listener(|h, e: &ColorChanged, _, cx| {
            h.s.solid = e.color;
            cx.notify();
        })))
        .child(color_well("alpha", s.alpha).alpha(true).label("Overlay").on_change(cx.listener(
            |h, e: &ColorChanged, _, cx| {
                h.s.alpha = e.color;
                cx.notify();
            },
        )))
        .child(color_well("off", s.solid).disabled(true))
        .into_any_element()
}

fn colours() -> Colours {
    Colours { solid: rgb_of(1., 0., 0., 1.), alpha: rgb_of(0., 0., 1., 0.5) }
}

#[test]
fn color_well_reports_kind_props_and_hex() {
    let rig = Rig::new(colours(), colour_view);
    assert!(rig.kind_is::<ColorWellKind>("solid"));
    assert!(!rig.props::<ColorWellProps>("solid").alpha);
    assert!(rig.props::<ColorWellProps>("alpha").alpha);
    let node = rig.sem("solid");
    assert_eq!((node.role, node.name.as_str(), node.value.as_deref()), (Role::ColorWell, "Tint", Some("#ff0000")));
    assert_eq!(rig.sem("alpha").value.as_deref(), Some("#0000ff80"));
}

#[test]
fn color_event_round_trips_with_one_update() {
    let mut rig = Rig::new(colours(), colour_view);
    rig.fresh();
    rig.emit("solid", ColorChanged { color: rgb_of(0., 1., 0., 1.) });
    assert_eq!(rig.updates("solid"), 1);
    assert_eq!(rig.sem("solid").value.as_deref(), Some("#00ff00"));
    rig.fresh();
    rig.emit("solid", ColorChanged { color: rgb_of(0., 1., 0., 1.) });
    assert_eq!(rig.all_updates(), 0);
}

#[test]
fn color_set_value_drops_alpha_unless_the_well_offers_it() {
    let mut rig = Rig::new(colours(), colour_view);
    rig.set_value("solid", "#00ff0080").unwrap();
    assert_eq!(rig.sem("solid").value.as_deref(), Some("#00ff00"), "no opacity control, so opaque");
    rig.set_value("alpha", "#00ff0080").unwrap();
    assert_eq!(rig.sem("alpha").value.as_deref(), Some("#00ff0080"));
    rig.set_value("alpha", "#336699").unwrap();
    assert_eq!(rig.sem("alpha").value.as_deref(), Some("#336699"));
}

#[test]
fn color_well_refuses_bad_colours_and_a_disabled_control() {
    let mut rig = Rig::new(colours(), colour_view);
    for bad in ["red", "ff0000", "#f00", "#gg0000", "#ff00000", ""] {
        assert!(refused(rig.set_value("solid", bad)), "{bad:?}");
    }
    assert_eq!(rig.sem("solid").value.as_deref(), Some("#ff0000"));
    assert!(refused(rig.set_value("off", "#000000")));
    assert!(!rig.sem("off").enabled);
}
