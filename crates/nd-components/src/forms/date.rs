//! Pickers: `date_picker` (`NSDatePicker`) and `color_well` (`NSColorWell`). Dates are civil
//! (wall clock, no zone), so the app decides what zone a value means; the native picker shows
//! them in the user's calendar and locale.

use std::fmt;

use nd_ui::prelude::*;
use nd_ui::semantics::{Response, SemanticAction, Semantics, WidgetSemantics};
use nd_ui::{Key, Role, StyleRefinement};

use super::{guard, leaf};

/// A wall-clock date and time. Orders chronologically.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CivilDateTime {
    pub year: i32,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

impl CivilDateTime {
    pub const fn date(year: i32, month: u8, day: u8) -> Self {
        Self { year, month, day, hour: 0, minute: 0, second: 0 }
    }

    pub const fn at(self, hour: u8, minute: u8, second: u8) -> Self {
        Self { hour, minute, second, ..self }
    }

    /// Parses `2026-10-13`, `2026-10-13T09:30`, `2026-10-13 09:30:15` or `09:30` (time only,
    /// on `base`'s date).
    pub fn parse(text: &str, base: CivilDateTime) -> Option<Self> {
        let text = text.trim();
        let (date, time) = match text.split_once(['T', ' ']) {
            Some((d, t)) => (Some(d), Some(t)),
            None if text.contains(':') => (None, Some(text)),
            None => (Some(text), None),
        };
        let mut out = base;
        if let Some(date) = date {
            let mut parts = date.splitn(3, '-');
            out.year = parts.next()?.parse().ok()?;
            out.month = parts.next()?.parse().ok().filter(|m| (1..=12).contains(m))?;
            out.day = parts.next()?.parse().ok().filter(|d| (1..=31).contains(d))?;
            if time.is_none() {
                (out.hour, out.minute, out.second) = (0, 0, 0);
            }
        }
        if let Some(time) = time {
            let mut parts = time.split(':');
            out.hour = parts.next()?.parse().ok().filter(|h| *h < 24)?;
            out.minute = parts.next()?.parse().ok().filter(|m| *m < 60)?;
            out.second = parts.next().map_or(Some(0), |s| s.parse().ok().filter(|s| *s < 60))?;
        }
        Some(out)
    }
}

impl fmt::Display for CivilDateTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        )
    }
}

/// Which parts of the value the picker edits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum DatePickerMode {
    #[default]
    Date,
    Time,
    DateTime,
}

/// How the picker presents itself.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum DatePickerStyle {
    /// A compact field the user edits in place; the platform's default.
    #[default]
    Field,
    /// The field with stepper arrows.
    Stepper,
    /// A calendar month (and clock face for time).
    Graphical,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DatePickerProps {
    pub value: CivilDateTime,
    pub mode: DatePickerMode,
    pub presentation: DatePickerStyle,
    pub min: Option<CivilDateTime>,
    pub max: Option<CivilDateTime>,
    pub label: Option<SharedString>,
    pub enabled: bool,
    pub tooltip: Option<SharedString>,
    pub style: StyleRefinement,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DateChanged {
    pub value: CivilDateTime,
}

/// Widget kind of `date_picker`.
pub struct DatePickerKind;

impl DatePickerProps {
    /// `value` clamped into `min..=max`, as the picker holds it.
    pub fn normalise(&self, value: CivilDateTime) -> CivilDateTime {
        let value = self.min.map_or(value, |min| value.max(min));
        self.max.map_or(value, |max| value.min(max))
    }

    fn text(&self) -> String {
        let full = self.value.to_string();
        match self.mode {
            DatePickerMode::Date => full[..10].to_owned(),
            DatePickerMode::Time => full[11..].to_owned(),
            DatePickerMode::DateTime => full,
        }
    }
}

impl WidgetSemantics for DatePickerProps {
    fn semantics(&self) -> Semantics {
        let name = self.label.clone().unwrap_or_default();
        Semantics::new(Role::DatePicker, name.to_string()).value(self.text()).enabled(self.enabled)
    }

    fn respond(&self, action: &SemanticAction) -> Option<Response> {
        let SemanticAction::SetValue(value) = action else { return None };
        if let Some(refused) = guard(self.enabled) {
            return Some(refused);
        }
        Some(match CivilDateTime::parse(value, self.value) {
            Some(v) => Ok(Box::new(DateChanged { value: self.normalise(v) })),
            None => super::not_taken(&format!("\"{value}\" is not a date (YYYY-MM-DD, HH:MM or both)")),
        })
    }
}

pub struct DatePicker(NativeElement<DatePickerProps>);

pub fn date_picker(id: impl Into<SharedString>, value: CivilDateTime) -> DatePicker {
    let props = DatePickerProps {
        value,
        mode: DatePickerMode::Date,
        presentation: DatePickerStyle::Field,
        min: None,
        max: None,
        label: None,
        enabled: true,
        tooltip: None,
        style: StyleRefinement::default(),
    };
    DatePicker(NativeElement::described::<DatePickerKind>(Key::Id(id.into()), props))
}

impl DatePicker {
    pub fn mode(mut self, mode: DatePickerMode) -> Self {
        self.0.props_mut().mode = mode;
        self
    }

    pub fn presentation(mut self, presentation: DatePickerStyle) -> Self {
        self.0.props_mut().presentation = presentation;
        self
    }

    pub fn min(mut self, min: CivilDateTime) -> Self {
        self.0.props_mut().min = Some(min);
        self
    }

    pub fn max(mut self, max: CivilDateTime) -> Self {
        self.0.props_mut().max = Some(max);
        self
    }

    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.0.props_mut().label = Some(label.into());
        self
    }

    pub fn on_change(self, f: impl Fn(&DateChanged, &mut Window, &mut App) + 'static) -> Self {
        DatePicker(self.0.on(f))
    }
}

leaf!(DatePicker, DatePickerProps);

#[derive(Clone, Debug, PartialEq)]
pub struct ColorWellProps {
    pub color: Hsla,
    /// Offers an opacity control in the picker.
    pub alpha: bool,
    pub label: Option<SharedString>,
    pub enabled: bool,
    pub tooltip: Option<SharedString>,
    pub style: StyleRefinement,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColorChanged {
    pub color: Hsla,
}

/// Widget kind of `color_well`.
pub struct ColorWellKind;

/// `#rrggbb`, or `#rrggbbaa` when not opaque.
pub(crate) fn hex(color: Hsla) -> String {
    let c = Rgba::from(color);
    let byte = |v: f32| (v.clamp(0., 1.) * 255.).round() as u8;
    let (r, g, b, a) = (byte(c.r), byte(c.g), byte(c.b), byte(c.a));
    if a == 255 { format!("#{r:02x}{g:02x}{b:02x}") } else { format!("#{r:02x}{g:02x}{b:02x}{a:02x}") }
}

pub(crate) fn parse_hex(text: &str) -> Option<Hsla> {
    let digits = text.trim().strip_prefix('#')?;
    let channel = |i: usize| u8::from_str_radix(digits.get(i..i + 2)?, 16).ok().map(|v| f32::from(v) / 255.);
    let (r, g, b) = (channel(0)?, channel(2)?, channel(4)?);
    let a = match digits.len() {
        6 => 1.,
        8 => channel(6)?,
        _ => return None,
    };
    Some(Rgba { r, g, b, a }.into())
}

impl WidgetSemantics for ColorWellProps {
    fn semantics(&self) -> Semantics {
        let name = self.label.clone().unwrap_or_default();
        Semantics::new(Role::ColorWell, name.to_string()).value(hex(self.color)).enabled(self.enabled)
    }

    fn respond(&self, action: &SemanticAction) -> Option<Response> {
        let SemanticAction::SetValue(value) = action else { return None };
        if let Some(refused) = guard(self.enabled) {
            return Some(refused);
        }
        Some(match parse_hex(value) {
            Some(mut color) => {
                if !self.alpha {
                    color.a = 1.;
                }
                Ok(Box::new(ColorChanged { color }))
            }
            None => super::not_taken(&format!("\"{value}\" is not a #rrggbb colour")),
        })
    }
}

pub struct ColorWell(NativeElement<ColorWellProps>);

pub fn color_well(id: impl Into<SharedString>, color: Hsla) -> ColorWell {
    let props = ColorWellProps {
        color,
        alpha: false,
        label: None,
        enabled: true,
        tooltip: None,
        style: StyleRefinement::default(),
    };
    ColorWell(NativeElement::described::<ColorWellKind>(Key::Id(id.into()), props))
}

impl ColorWell {
    pub fn alpha(mut self, alpha: bool) -> Self {
        self.0.props_mut().alpha = alpha;
        self
    }

    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.0.props_mut().label = Some(label.into());
        self
    }

    pub fn on_change(self, f: impl Fn(&ColorChanged, &mut Window, &mut App) + 'static) -> Self {
        ColorWell(self.0.on(f))
    }
}

leaf!(ColorWell, ColorWellProps);
