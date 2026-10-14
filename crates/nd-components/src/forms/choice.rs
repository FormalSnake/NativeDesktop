//! One of several options: `select` (a pop-up button), `segmented` and `radio_group`. One
//! widget kind; the mapping picks the native control (`NSPopUpButton`, `NSSegmentedControl`,
//! a column of radio `NSButton`s), so `.native(Mapping::..)` switches presentation without the
//! app changing anything else. Controlled: `selected` is the app's.

use nd_ui::prelude::*;
use nd_ui::semantics::{Response, SemanticAction, Semantics, WidgetSemantics};
use nd_ui::{Key, Role, StyleRefinement};

use super::{guard, leaf};
use crate::controls::Mapping;

#[derive(Clone, Debug, PartialEq)]
pub struct SelectProps {
    pub options: Vec<SharedString>,
    pub selected: usize,
    pub mapping: Mapping,
    /// Radio buttons side by side instead of stacked.
    pub horizontal: bool,
    /// Accessible name when no visible label names the control.
    pub label: Option<SharedString>,
    pub enabled: bool,
    pub tooltip: Option<SharedString>,
    pub style: StyleRefinement,
}

/// The option the user picked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SelectChanged {
    pub index: usize,
}

/// Widget kind of `select`, `segmented` and `radio_group`.
pub struct SelectKind;

/// The index `value` names among `options`: the option's title (exact, then ignoring case),
/// or its index.
pub fn option_index(options: &[SharedString], value: &str) -> Option<usize> {
    options
        .iter()
        .position(|o| o.as_ref() == value)
        .or_else(|| options.iter().position(|o| o.eq_ignore_ascii_case(value)))
        .or_else(|| value.trim().parse::<usize>().ok().filter(|i| *i < options.len()))
}

impl WidgetSemantics for SelectProps {
    fn semantics(&self) -> Semantics {
        let role = match self.mapping {
            Mapping::Automatic | Mapping::PopUp => Role::PopUpButton,
            Mapping::Segmented => Role::SegmentedControl,
            Mapping::Radio => Role::RadioGroup,
        };
        let name = self.label.clone().unwrap_or_default();
        let mut s = Semantics::new(role, name.to_string()).enabled(self.enabled);
        if let Some(option) = self.options.get(self.selected) {
            s = s.value(option.to_string());
        }
        s
    }

    fn respond(&self, action: &SemanticAction) -> Option<Response> {
        let SemanticAction::SetValue(value) = action else { return None };
        if let Some(refused) = guard(self.enabled) {
            return Some(refused);
        }
        Some(match option_index(&self.options, value) {
            Some(index) => Ok(Box::new(SelectChanged { index })),
            None => super::not_taken(&format!("no option \"{value}\"")),
        })
    }
}

/// One of several options: a pop-up button by default, a segmented control or radio group
/// through [`Select::native`].
pub struct Select(NativeElement<SelectProps>);

fn build(id: SharedString, options: Vec<SharedString>, mapping: Mapping) -> Select {
    let props = SelectProps {
        options,
        selected: 0,
        mapping,
        horizontal: false,
        label: None,
        enabled: true,
        tooltip: None,
        style: StyleRefinement::default(),
    };
    Select(NativeElement::described::<SelectKind>(Key::Id(id), props))
}

pub fn select(id: impl Into<SharedString>, options: impl IntoIterator<Item = impl Into<SharedString>>) -> Select {
    build(id.into(), options.into_iter().map(Into::into).collect(), Mapping::Automatic)
}

/// A segmented control: the options side by side, one selected.
pub fn segmented(id: impl Into<SharedString>, options: impl IntoIterator<Item = impl Into<SharedString>>) -> Select {
    build(id.into(), options.into_iter().map(Into::into).collect(), Mapping::Segmented)
}

/// Radio buttons, one per option, stacked unless `.horizontal()`.
pub fn radio_group(id: impl Into<SharedString>, options: impl IntoIterator<Item = impl Into<SharedString>>) -> Select {
    build(id.into(), options.into_iter().map(Into::into).collect(), Mapping::Radio)
}

impl Select {
    pub fn selected(mut self, index: usize) -> Self {
        self.0.props_mut().selected = index;
        self
    }

    pub fn native(mut self, mapping: Mapping) -> Self {
        self.0.props_mut().mapping = mapping;
        self
    }

    pub fn horizontal(mut self) -> Self {
        self.0.props_mut().horizontal = true;
        self
    }

    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.0.props_mut().label = Some(label.into());
        self
    }

    pub fn on_change(self, f: impl Fn(&SelectChanged, &mut Window, &mut App) + 'static) -> Self {
        Select(self.0.on(f))
    }
}

leaf!(Select, SelectProps);
