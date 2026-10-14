//! Numeric values: `slider` (`NSSlider`) and `stepper` (`NSStepper`, with its value in a field
//! beside it on request). Controlled; a control that snaps the value to its step reports the
//! snapped value, which the app can feed back without a second set.

use std::ops::RangeInclusive;

use nd_ui::prelude::*;
use nd_ui::semantics::{Response, SemanticAction, Semantics, WidgetSemantics};
use nd_ui::{Key, Role, StyleRefinement};

use super::{guard, leaf};

#[derive(Clone, Debug, PartialEq)]
pub struct SliderProps {
    pub value: f32,
    pub min: f32,
    pub max: f32,
    /// Values snap to multiples of `step` from `min`.
    pub step: Option<f32>,
    /// Tick marks under the track; with a `step`, values snap to them.
    pub ticks: usize,
    pub vertical: bool,
    /// The framework-drawn slider instead of the native control.
    pub drawn: bool,
    pub label: Option<SharedString>,
    pub enabled: bool,
    pub tooltip: Option<SharedString>,
    pub style: StyleRefinement,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SliderChanged {
    pub value: f32,
}

/// Widget kind of `slider`.
pub struct SliderKind;

impl SliderProps {
    /// `value` clamped to the range and snapped to the step, as the native control would hold it.
    pub fn normalise(&self, value: f32) -> f32 {
        let (lo, hi) = if self.min <= self.max { (self.min, self.max) } else { (self.max, self.min) };
        let v = value.clamp(lo, hi);
        match self.step.filter(|s| *s > 0.) {
            Some(step) => (lo + ((v - lo) / step).round() * step).clamp(lo, hi),
            None => v,
        }
    }
}

/// Formats a number without trailing zeros (`0.5`, `3`), the form automation reads back.
pub fn number_text(value: f64, digits: Option<u8>) -> String {
    match digits {
        Some(d) => format!("{value:.*}", d as usize),
        None => {
            let s = format!("{value:.6}");
            let s = s.trim_end_matches('0').trim_end_matches('.');
            if s == "-0" { "0".to_owned() } else { s.to_owned() }
        }
    }
}

fn parse_number(value: &str) -> Option<f64> {
    value.trim().trim_end_matches('%').parse::<f64>().ok().filter(|v| v.is_finite())
}

impl WidgetSemantics for SliderProps {
    fn semantics(&self) -> Semantics {
        let name = self.label.clone().unwrap_or_default();
        Semantics::new(Role::Slider, name.to_string()).value(number_text(self.value.into(), None)).enabled(self.enabled)
    }

    fn respond(&self, action: &SemanticAction) -> Option<Response> {
        let SemanticAction::SetValue(value) = action else { return None };
        if let Some(refused) = guard(self.enabled) {
            return Some(refused);
        }
        Some(match parse_number(value) {
            Some(v) => Ok(Box::new(SliderChanged { value: self.normalise(v as f32) })),
            None => super::not_taken(&format!("\"{value}\" is not a number")),
        })
    }
}

pub struct Slider(NativeElement<SliderProps>);

pub fn slider(id: impl Into<SharedString>, value: f32) -> Slider {
    let props = SliderProps {
        value,
        min: 0.,
        max: 1.,
        step: None,
        ticks: 0,
        vertical: false,
        drawn: false,
        label: None,
        enabled: true,
        tooltip: None,
        style: StyleRefinement::default(),
    };
    Slider(NativeElement::described::<SliderKind>(Key::Id(id.into()), props))
}

impl Slider {
    pub fn range(mut self, range: RangeInclusive<f32>) -> Self {
        let props = self.0.props_mut();
        props.min = *range.start();
        props.max = *range.end();
        self
    }

    pub fn step(mut self, step: f32) -> Self {
        self.0.props_mut().step = Some(step);
        self
    }

    pub fn ticks(mut self, ticks: usize) -> Self {
        self.0.props_mut().ticks = ticks;
        self
    }

    pub fn vertical(mut self) -> Self {
        self.0.props_mut().vertical = true;
        self
    }

    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.0.props_mut().label = Some(label.into());
        self
    }

    /// The framework-drawn slider on every OS instead of the native control: it takes every
    /// style override (height, radius, colours) the native one cannot express.
    pub fn drawn(mut self) -> Self {
        self.0.props_mut().drawn = true;
        self
    }

    pub fn on_change(self, f: impl Fn(&SliderChanged, &mut Window, &mut App) + 'static) -> Self {
        Slider(self.0.on(f))
    }
}

leaf!(Slider, SliderProps);

#[derive(Clone, Debug, PartialEq)]
pub struct StepperProps {
    pub value: f64,
    pub min: f64,
    pub max: f64,
    pub step: f64,
    /// Past `max` goes to `min` and back.
    pub wraps: bool,
    /// Shows the value in an editable field beside the arrows, with this many decimals.
    pub field: Option<u8>,
    pub label: Option<SharedString>,
    pub enabled: bool,
    pub tooltip: Option<SharedString>,
    pub style: StyleRefinement,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StepperChanged {
    pub value: f64,
}

/// Widget kind of `stepper`.
pub struct StepperKind;

impl StepperProps {
    /// `value` as the stepper holds it: clamped, or wrapped when `wraps`.
    pub fn normalise(&self, value: f64) -> f64 {
        if self.wraps && self.max > self.min {
            if value > self.max {
                return self.min;
            }
            if value < self.min {
                return self.max;
            }
        }
        value.clamp(self.min.min(self.max), self.max.max(self.min))
    }
}

impl WidgetSemantics for StepperProps {
    fn semantics(&self) -> Semantics {
        let name = self.label.clone().unwrap_or_default();
        Semantics::new(Role::Incrementor, name.to_string())
            .value(number_text(self.value, self.field))
            .enabled(self.enabled)
    }

    fn respond(&self, action: &SemanticAction) -> Option<Response> {
        let SemanticAction::SetValue(value) = action else { return None };
        if let Some(refused) = guard(self.enabled) {
            return Some(refused);
        }
        Some(match parse_number(value) {
            Some(v) => Ok(Box::new(StepperChanged { value: self.normalise(v) })),
            None => super::not_taken(&format!("\"{value}\" is not a number")),
        })
    }
}

pub struct Stepper(NativeElement<StepperProps>);

/// Up and down arrows changing `value` by a step.
pub fn stepper(id: impl Into<SharedString>, value: f64) -> Stepper {
    let props = StepperProps {
        value,
        min: 0.,
        max: 100.,
        step: 1.,
        wraps: false,
        field: None,
        label: None,
        enabled: true,
        tooltip: None,
        style: StyleRefinement::default(),
    };
    Stepper(NativeElement::described::<StepperKind>(Key::Id(id.into()), props))
}

impl Stepper {
    pub fn range(mut self, range: RangeInclusive<f64>) -> Self {
        let props = self.0.props_mut();
        props.min = *range.start();
        props.max = *range.end();
        self
    }

    pub fn step(mut self, step: f64) -> Self {
        self.0.props_mut().step = step;
        self
    }

    pub fn wraps(mut self, wraps: bool) -> Self {
        self.0.props_mut().wraps = wraps;
        self
    }

    /// The value in an editable field beside the arrows, with `digits` decimals.
    pub fn with_field(mut self, digits: u8) -> Self {
        self.0.props_mut().field = Some(digits);
        self
    }

    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.0.props_mut().label = Some(label.into());
        self
    }

    pub fn on_change(self, f: impl Fn(&StepperChanged, &mut Window, &mut App) + 'static) -> Self {
        Stepper(self.0.on(f))
    }
}

leaf!(Stepper, StepperProps);
