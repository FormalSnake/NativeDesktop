//! On/off controls: `checkbox` (an `NSButton` of the checkbox type on AppKit) and `switch`
//! (`NSSwitch`). Controlled: `checked` is the app's, user toggles arrive as [`Toggled`].

use nd_ui::prelude::*;
use nd_ui::semantics::{Response, SemanticAction, Semantics, WidgetSemantics};
use nd_ui::{Key, Role, StyleRefinement};

use super::{guard, leaf};

/// The control's new state after the user toggled it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Toggled {
    pub checked: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CheckboxProps {
    pub label: SharedString,
    pub checked: bool,
    /// The mixed state of a checkbox standing for several others; a click checks it.
    pub mixed: bool,
    pub enabled: bool,
    pub tooltip: Option<SharedString>,
    pub style: StyleRefinement,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SwitchProps {
    pub checked: bool,
    /// Accessible name; a switch shows no title of its own (its row's label names it).
    pub label: Option<SharedString>,
    pub enabled: bool,
    pub tooltip: Option<SharedString>,
    pub style: StyleRefinement,
}

/// Widget kind of `checkbox`.
pub struct CheckboxKind;
/// Widget kind of `switch`.
pub struct SwitchKind;

fn parse_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "on" | "yes" | "checked" => Some(true),
        "false" | "0" | "off" | "no" | "unchecked" => Some(false),
        _ => None,
    }
}

fn respond_toggle(checked: bool, enabled: bool, action: &SemanticAction) -> Option<Response> {
    let next = match action {
        SemanticAction::Click => !checked,
        SemanticAction::SetValue(v) => match parse_bool(v) {
            Some(b) => b,
            None => return Some(super::not_taken(&format!("\"{v}\" is not on or off"))),
        },
        _ => return None,
    };
    if let Some(refused) = guard(enabled) {
        return Some(refused);
    }
    Some(Ok(Box::new(Toggled { checked: next })))
}

impl WidgetSemantics for CheckboxProps {
    fn semantics(&self) -> Semantics {
        let value = if self.mixed {
            "mixed"
        } else if self.checked {
            "true"
        } else {
            "false"
        };
        Semantics::new(Role::Checkbox, self.label.to_string()).value(value).checked(self.checked).enabled(self.enabled)
    }

    fn respond(&self, action: &SemanticAction) -> Option<Response> {
        // A mixed checkbox checks on click, as AppKit's allowsMixedState cycle does from mixed.
        respond_toggle(self.checked && !self.mixed, self.enabled, action)
    }
}

impl WidgetSemantics for SwitchProps {
    fn semantics(&self) -> Semantics {
        let name = self.label.clone().unwrap_or_default();
        Semantics::new(Role::Switch, name.to_string())
            .value(if self.checked { "true" } else { "false" })
            .checked(self.checked)
            .enabled(self.enabled)
    }

    fn respond(&self, action: &SemanticAction) -> Option<Response> {
        respond_toggle(self.checked, self.enabled, action)
    }
}

pub struct Checkbox(NativeElement<CheckboxProps>);
pub struct Switch(NativeElement<SwitchProps>);

/// A checkbox with its title beside it.
pub fn checkbox(id: impl Into<SharedString>, label: impl Into<SharedString>) -> Checkbox {
    let props = CheckboxProps {
        label: label.into(),
        checked: false,
        mixed: false,
        enabled: true,
        tooltip: None,
        style: StyleRefinement::default(),
    };
    Checkbox(NativeElement::described::<CheckboxKind>(Key::Id(id.into()), props))
}

/// An on/off switch. Give it a `.label(..)` when no row title names it.
pub fn switch(id: impl Into<SharedString>) -> Switch {
    let props =
        SwitchProps { checked: false, label: None, enabled: true, tooltip: None, style: StyleRefinement::default() };
    Switch(NativeElement::described::<SwitchKind>(Key::Id(id.into()), props))
}

impl Checkbox {
    pub fn checked(mut self, checked: bool) -> Self {
        self.0.props_mut().checked = checked;
        self
    }

    pub fn mixed(mut self, mixed: bool) -> Self {
        self.0.props_mut().mixed = mixed;
        self
    }

    pub fn on_toggle(self, f: impl Fn(&Toggled, &mut Window, &mut App) + 'static) -> Self {
        Checkbox(self.0.on(f))
    }
}

impl Switch {
    pub fn checked(mut self, checked: bool) -> Self {
        self.0.props_mut().checked = checked;
        self
    }

    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.0.props_mut().label = Some(label.into());
        self
    }

    pub fn on_toggle(self, f: impl Fn(&Toggled, &mut Window, &mut App) + 'static) -> Self {
        Switch(self.0.on(f))
    }
}

leaf!(Checkbox, CheckboxProps);
leaf!(Switch, SwitchProps);
