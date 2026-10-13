//! Drawn pickers. `date_picker`: the `Field` and `Stepper` styles are a field holding the value
//! as ISO text (`2026-10-13`, `09:30`, `2026-10-13 09:30`), committed on Return or when focus
//! leaves; `Stepper` adds day (or minute) arrows and Up/Down. `Graphical` is a month grid with
//! arrow-key navigation. `color_well`: a swatch opening a popover with a palette and a hex field.
//! Every change reports the normalised value; what is shown is the prop's.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use nd_components::forms::*;
use nd_core::NodeId;
use nd_gpui::gpui::{
    self, App, CursorStyle, ElementId, FocusHandle, FontWeight, Hsla, InteractiveElement, IntoElement, KeyDownEvent,
    MouseDownEvent, ParentElement, Rgba, SharedString, StatefulInteractiveElement, Styled, Window, div, px,
};

use super::glyph::{Glyph, glyph};
use super::text::{Commit, Editor, Field, line, popover, prepare, value_field};
use super::toggle::{pressable, ring};
use super::{Sink, with_style};
use crate::render::Build;

fn date_text(p: &DatePickerProps) -> String {
    let v = p.value;
    let date = format!("{:04}-{:02}-{:02}", v.year, v.month, v.day);
    let time = format!("{:02}:{:02}", v.hour, v.minute);
    match p.mode {
        DatePickerMode::Date => date,
        DatePickerMode::Time => time,
        DatePickerMode::DateTime => format!("{date} {time}"),
    }
}

/// Days since 1970-01-01 (proleptic Gregorian).
fn days(y: i32, m: u8, d: u8) -> i64 {
    let y = i64::from(y) - i64::from(m <= 2);
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (i64::from(m) + 9) % 12;
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil(z: i64) -> (i32, u8, u8) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u8;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
    ((yoe + era * 400 + i64::from(m <= 2)) as i32, m, d)
}

fn add_days(v: CivilDateTime, n: i64) -> CivilDateTime {
    let (year, month, day) = civil(days(v.year, v.month, v.day) + n);
    CivilDateTime { year, month, day, ..v }
}

fn add_minutes(v: CivilDateTime, n: i64) -> CivilDateTime {
    let total = i64::from(v.hour) * 60 + i64::from(v.minute) + n;
    let day = add_days(v, total.div_euclid(24 * 60));
    let minutes = total.rem_euclid(24 * 60);
    CivilDateTime { hour: (minutes / 60) as u8, minute: (minutes % 60) as u8, ..day }
}

fn days_in_month(year: i32, month: u8) -> u8 {
    let (ny, nm) = if month == 12 { (year + 1, 1) } else { (year, month + 1) };
    (days(ny, nm, 1) - days(year, month, 1)) as u8
}

/// One step of the stepper style: a day, or a minute when only the time shows.
fn step(p: &DatePickerProps, n: i64) -> CivilDateTime {
    match p.mode {
        DatePickerMode::Time => add_minutes(p.value, n),
        _ => add_days(p.value, n),
    }
}

fn changed(sink: &Sink, p: &DatePickerProps, next: CivilDateTime, cx: &mut App) {
    let next = p.normalise(next);
    if next != p.value {
        sink.send(DateChanged { value: next }, cx);
    }
}

pub(crate) fn date_picker(
    b: &Build,
    id: NodeId,
    p: &DatePickerProps,
    window: &mut Window,
    cx: &mut App,
) -> gpui::AnyElement {
    if p.presentation == DatePickerStyle::Graphical {
        return calendar(b, id, p, window, cx);
    }
    let parse = {
        let p = p.clone();
        move |text: &str| -> Option<Box<dyn std::any::Any>> {
            Some(Box::new(DateChanged { value: p.normalise(CivilDateTime::parse(text, p.value)?) }))
        }
    };
    let (ctl, field) = value_field(b, id, date_text(p).into(), p.enabled, parse, window, cx);
    let look = ctl.look;
    let name = p.label.clone().or_else(|| p.tooltip.clone()).unwrap_or_default();
    let role = match p.mode {
        DatePickerMode::Date => gpui::Role::DateInput,
        DatePickerMode::Time => gpui::Role::TimeInput,
        DatePickerMode::DateTime => gpui::Role::DateTimeInput,
    };
    let mut field = field.role(role).aria_label(name).aria_value(SharedString::from(date_text(p)));
    if p.presentation == DatePickerStyle::Stepper {
        let arrow = |key: &'static str, g: Glyph, label: &'static str, n: i64| {
            let mut a = div()
                .id(key)
                .role(gpui::Role::Button)
                .aria_label(label)
                .flex()
                .items_center()
                .justify_center()
                .w(look.icon_size + px(6.))
                .flex_1()
                .rounded(look.radius)
                .child(glyph(g, look.icon_size - px(4.), look.text, look.glyph_stroke));
            if p.enabled {
                let (sink, p) = (ctl.sink.clone(), p.clone());
                a = a.occlude().cursor(CursorStyle::PointingHand).hover(move |s| s.bg(look.fill_hover)).on_click(
                    move |_, window, cx| {
                        sink.take_focus(window, cx);
                        changed(&sink, &p, step(&p, n), cx);
                    },
                );
            }
            a
        };
        field = field.pr(px(2.)).child(
            div()
                .flex()
                .flex_col()
                .h_full()
                .py(px(2.))
                .child(arrow("later", Glyph::ChevronUp, "Later", 1))
                .child(arrow("earlier", Glyph::ChevronDown, "Earlier", -1)),
        );
        if p.enabled {
            let (sink, p2) = (ctl.sink.clone(), p.clone());
            field = field.on_key_down(move |event: &KeyDownEvent, window: &mut Window, cx: &mut App| {
                let n = match event.keystroke.key.as_str() {
                    "up" => 1,
                    "down" => -1,
                    _ => return,
                };
                cx.stop_propagation();
                changed(&sink, &p2, step(&p2, n), cx);
                window.refresh();
            });
        }
    }
    with_style(field, &p.style).into_any_element()
}

/// The month the calendar shows, until the value moves.
#[derive(Default)]
pub(crate) struct Month {
    shown: Option<(i32, u8)>,
    value: Option<CivilDateTime>,
}

const WEEKDAYS: [&str; 7] = ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"];
const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

fn calendar(b: &Build, id: NodeId, p: &DatePickerProps, window: &mut Window, cx: &mut App) -> gpui::AnyElement {
    let (ctl, month) = b.ctl(id, p.enabled, window, cx, Month::default);
    let look = ctl.look;
    let (year, m) = {
        let mut s = month.borrow_mut();
        if s.value != Some(p.value) {
            *s = Month { shown: Some((p.value.year, p.value.month)), value: Some(p.value) };
        }
        s.shown.unwrap_or((p.value.year, p.value.month))
    };
    let flip = |key: &'static str, g: Glyph, label: &'static str, delta: i32| {
        let mut button = div()
            .id(key)
            .role(gpui::Role::Button)
            .aria_label(label)
            .flex()
            .items_center()
            .justify_center()
            .size(look.day_size)
            .rounded(look.radius)
            .child(glyph(g, look.icon_size, look.text, look.glyph_stroke));
        if p.enabled {
            let month = month.clone();
            button = button.cursor(CursorStyle::PointingHand).hover(move |s| s.bg(look.fill_hover)).on_click(
                move |_, window, _| {
                    let mut s = month.borrow_mut();
                    let (y, m) = s.shown.unwrap_or((year, m));
                    let index = y * 12 + i32::from(m) - 1 + delta;
                    s.shown = Some((index.div_euclid(12), (index.rem_euclid(12) + 1) as u8));
                    window.refresh();
                },
            );
        }
        button
    };
    let header = div()
        .flex()
        .flex_row()
        .items_center()
        .child(flip("previous", Glyph::ChevronLeft, "Previous month", -1))
        .child(
            div()
                .flex_1()
                .flex()
                .justify_center()
                .font_weight(FontWeight::BOLD)
                .child(format!("{} {year}", MONTHS[usize::from(m - 1)])),
        )
        .child(flip("next", Glyph::ChevronRight, "Next month", 1));
    let cell =
        |content: gpui::AnyElement| div().size(look.day_size).flex().items_center().justify_center().child(content);
    let mut grid = div().flex().flex_col().child(div().flex().flex_row().children(WEEKDAYS.iter().map(|d| {
        cell(
            div()
                .text_color(look.secondary_text)
                .text_size(look.type_of(TextStyle::Caption1).0)
                .child(*d)
                .into_any_element(),
        )
    })));
    let first = days(year, m, 1);
    let lead = (first + 3).rem_euclid(7);
    let count = i64::from(days_in_month(year, m));
    for week in 0..6 {
        let mut row = div().flex().flex_row();
        for weekday in 0..7 {
            let n = week * 7 + weekday - lead;
            if n < 0 || n >= count {
                row = row.child(cell(div().into_any_element()));
                continue;
            }
            let day = (n + 1) as u8;
            let date = CivilDateTime { year, month: m, day, ..p.value };
            let selected = (p.value.year, p.value.month, p.value.day) == (year, m, day);
            let allowed = p.normalise(date) == date;
            let mut d = div()
                .id(ElementId::Integer(n as u64))
                .role(gpui::Role::Button)
                .aria_label(SharedString::from(format!("{} {day}", MONTHS[usize::from(m - 1)])))
                .aria_selected(selected)
                .size(look.day_size)
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .child(day.to_string());
            d = if selected {
                d.bg(look.accent).text_color(look.on_accent).font_weight(FontWeight::BOLD)
            } else if !allowed {
                d.text_color(look.secondary_text.opacity(0.6))
            } else {
                d
            };
            if p.enabled && allowed && !selected {
                let (sink, p) = (ctl.sink.clone(), p.clone());
                d = d.cursor(CursorStyle::PointingHand).hover(move |s| s.bg(look.fill_hover)).on_click(
                    move |_, window, cx| {
                        sink.take_focus(window, cx);
                        changed(&sink, &p, date, cx);
                    },
                );
            }
            row = row.child(d);
        }
        grid = grid.child(row);
    }
    let name = p.label.clone().or_else(|| p.tooltip.clone()).unwrap_or_default();
    let mut body = div()
        .flex()
        .flex_col()
        .gap(px(4.))
        .p(look.popover_inset)
        .rounded(look.popover_radius)
        .bg(look.popover)
        .border_1()
        .border_color(look.border)
        .text_size(look.text_size)
        .text_color(look.text)
        .child(header)
        .child(grid);
    if p.mode == DatePickerMode::DateTime {
        body = body.child(
            div()
                .flex()
                .justify_center()
                .text_color(look.secondary_text)
                .child(format!("{:02}:{:02}", p.value.hour, p.value.minute)),
        );
    }
    let mut picker = div()
        .id(ElementId::Integer(id.0))
        .role(gpui::Role::DateInput)
        .aria_label(name)
        .aria_value(SharedString::from(date_text(p)))
        .flex_none()
        .child(ring(look, ctl.ring(), look.popover_radius, body));
    if !p.enabled {
        return with_style(picker.opacity(look.disabled_opacity), &p.style).into_any_element();
    }
    let (sink, p2) = (ctl.sink.clone(), p.clone());
    picker = pressable(picker, &ctl).cursor(CursorStyle::Arrow).on_key_down(
        move |event: &KeyDownEvent, _: &mut Window, cx: &mut App| {
            let n = match event.keystroke.key.as_str() {
                "left" => -1,
                "right" => 1,
                "up" => -7,
                "down" => 7,
                _ => return,
            };
            cx.stop_propagation();
            changed(&sink, &p2, add_days(p2.value, n), cx);
        },
    );
    with_style(picker, &p.style).into_any_element()
}

/// `#rrggbb`, or `#rrggbbaa` when not opaque, as `nd-components` reads it back.
fn hex(color: Hsla) -> String {
    let c = Rgba::from(color);
    let byte = |v: f32| (v.clamp(0., 1.) * 255.).round() as u8;
    let (r, g, b, a) = (byte(c.r), byte(c.g), byte(c.b), byte(c.a));
    if a == 255 { format!("#{r:02x}{g:02x}{b:02x}") } else { format!("#{r:02x}{g:02x}{b:02x}{a:02x}") }
}

fn parse_hex(text: &str) -> Option<Hsla> {
    let text = text.trim();
    let digits = text.strip_prefix('#').unwrap_or(text);
    let channel = |i: usize| u8::from_str_radix(digits.get(i..i + 2)?, 16).ok().map(|v| f32::from(v) / 255.);
    let a = match digits.len() {
        6 => 1.,
        8 => channel(6)?,
        _ => return None,
    };
    Some(Rgba { r: channel(0)?, g: channel(2)?, b: channel(4)?, a }.into())
}

/// The GNOME palette's middle shades plus light and dark, as the popover offers them.
const PALETTE: [u32; 10] =
    [0x3584e4, 0x33d17a, 0xf6d32d, 0xff7800, 0xe01b24, 0x9141ac, 0x986a44, 0xffffff, 0x9a9996, 0x241f31];

/// The open popover and its hex field.
pub(crate) struct Well {
    open: bool,
    alpha: Rc<Cell<bool>>,
    hex: Rc<RefCell<Editor>>,
    hex_focus: Option<FocusHandle>,
}

impl Well {
    fn new() -> Self {
        let alpha = Rc::new(Cell::new(false));
        let keep = alpha.clone();
        let parse = move |text: &str| -> Option<Box<dyn std::any::Any>> {
            let mut color = parse_hex(text)?;
            if !keep.get() {
                color.a = 1.;
            }
            Some(Box::new(ColorChanged { color }))
        };
        Self {
            open: false,
            alpha,
            hex: Rc::new(RefCell::new(Editor::new(false, Commit::Parse(Rc::new(parse))))),
            hex_focus: None,
        }
    }
}

pub(crate) fn color_well(
    b: &Build,
    id: NodeId,
    p: &ColorWellProps,
    window: &mut Window,
    cx: &mut App,
) -> gpui::AnyElement {
    let (ctl, well) = b.ctl(id, p.enabled, window, cx, Well::new);
    let look = ctl.look;
    let hex_focus = well.borrow_mut().hex_focus.get_or_insert_with(|| cx.focus_handle()).clone();
    well.borrow().alpha.set(p.alpha);
    let open = well.borrow().open && p.enabled;
    let name = p.label.clone().or_else(|| p.tooltip.clone()).unwrap_or_default();
    let mut swatch = div()
        .id(ElementId::Integer(id.0))
        .role(gpui::Role::ColorWell)
        .aria_label(name)
        .aria_value(SharedString::from(hex(p.color)))
        .aria_expanded(open)
        .flex()
        .items_center()
        .justify_center()
        .w(look.control_height + look.swatch_size)
        .h(look.control_height)
        .rounded(look.radius)
        .bg(look.fill)
        .border_2()
        .border_color(if ctl.ring() { look.focus_ring } else { gpui::transparent_black() })
        .child(
            div()
                .w(look.control_height + look.swatch_size - px(12.))
                .h(look.swatch_size)
                .rounded(look.radius - px(2.))
                .bg(p.color)
                .border_1()
                .border_color(look.knob_edge),
        );
    if !p.enabled {
        return with_style(swatch.opacity(look.disabled_opacity), &p.style).into_any_element();
    }
    let toggle = {
        let (well, sink) = (well.clone(), ctl.sink.clone());
        move |event: &gpui::ClickEvent, window: &mut Window, cx: &mut App| {
            if !matches!(event, gpui::ClickEvent::Keyboard(_)) {
                sink.take_focus(window, cx);
            }
            let mut w = well.borrow_mut();
            w.open = !w.open;
            drop(w);
            window.refresh();
        }
    };
    swatch = pressable(swatch, &ctl).hover(move |s| s.bg(look.fill_hover)).on_click(toggle);
    let mut root = div().flex().flex_col().child(with_style(swatch, &p.style));
    if open {
        let part = ctl.part(hex_focus, window);
        let f = Field {
            multiline: false,
            secure: false,
            editable: true,
            enabled: true,
            text: hex(p.color).into(),
            placeholder: None,
            options: Vec::new(),
            completes: false,
        };
        let editor = well.borrow().hex.clone();
        prepare(&part, &editor, &f, cx);
        let close = {
            let well = well.clone();
            move |_: &MouseDownEvent, window: &mut Window, _: &mut App| {
                well.borrow_mut().open = false;
                window.refresh();
            }
        };
        let escape = {
            let well = well.clone();
            move |event: &KeyDownEvent, window: &mut Window, cx: &mut App| {
                if event.keystroke.key == "escape" {
                    well.borrow_mut().open = false;
                    cx.stop_propagation();
                    window.refresh();
                }
            }
        };
        let swatches = PALETTE.iter().enumerate().map(|(i, rgb)| {
            let color: Hsla = gpui::rgb(*rgb).into();
            let (sink, well) = (ctl.sink.clone(), well.clone());
            let current = hex(color) == hex(Hsla { a: 1., ..p.color });
            div()
                .id(ElementId::Integer(i as u64))
                .role(gpui::Role::Button)
                .aria_label(SharedString::from(hex(color)))
                .child(ring(
                    look,
                    current,
                    look.swatch_size / 2.,
                    div().size(look.swatch_size).rounded_full().bg(color).border_1().border_color(look.knob_edge),
                ))
                .cursor(CursorStyle::PointingHand)
                .on_click(move |_, window, cx| {
                    well.borrow_mut().open = false;
                    sink.send(ColorChanged { color }, cx);
                    window.refresh();
                })
        });
        let palette = div().flex().flex_row().flex_wrap().w(px(5. * 30.)).children(swatches);
        let hex_field = line(&part, &editor, &f).w(px(5. * 30.)).aria_label("Hex").on_key_down(escape);
        root = root.child(popover(look, close, [palette.into_any_element(), hex_field.mt(px(6.)).into_any_element()]));
    }
    root.into_any_element()
}
