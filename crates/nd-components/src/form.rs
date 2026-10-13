//! `form()`: settings rows, as intent. On macOS a SwiftUI `Form` in the grouped style, so rows
//! take System Settings' metrics and restyle with the OS (ARCHITECTURE.md 3.6, 3.9); each
//! `form_row` holds one control, which the form places. GPUI backends draw grouped rows with
//! the skin's tokens.

use std::panic::Location;

use nd_ui::prelude::*;
use nd_ui::{ContainerElement, Key, StyleRefinement};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FormProps {}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FormSectionProps {
    pub title: Option<SharedString>,
    pub footer: Option<SharedString>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FormRowProps {
    pub label: SharedString,
    /// Secondary text under the label.
    pub help: Option<SharedString>,
}

pub struct Form(ContainerElement<FormProps>);
pub struct FormSection(ContainerElement<FormSectionProps>);
pub struct FormRow(ContainerElement<FormRowProps>);

/// A grouped form. Children are [`form_section`]s.
#[track_caller]
pub fn form() -> Form {
    Form(ContainerElement::new::<Form>(Key::Site(Location::caller()), FormProps::default()).size_full())
}

/// A group of rows under an optional title.
#[track_caller]
pub fn form_section() -> FormSection {
    FormSection(ContainerElement::new::<FormSection>(Key::Site(Location::caller()), FormSectionProps::default()))
}

/// One row: `label` on the leading side, its single child control on the trailing side.
#[track_caller]
pub fn form_row(label: impl Into<SharedString>) -> FormRow {
    let props = FormRowProps { label: label.into(), help: None };
    // A row, so the control keeps its natural size unless its own style sets one.
    FormRow(ContainerElement::new::<FormRow>(Key::Site(Location::caller()), props).flex().flex_row().items_start())
}

impl FormSection {
    pub fn title(mut self, title: impl Into<SharedString>) -> Self {
        self.0.props_mut().title = Some(title.into());
        self
    }

    pub fn footer(mut self, footer: impl Into<SharedString>) -> Self {
        self.0.props_mut().footer = Some(footer.into());
        self
    }
}

impl FormRow {
    pub fn help(mut self, help: impl Into<SharedString>) -> Self {
        self.0.props_mut().help = Some(help.into());
        self
    }

    pub fn id(mut self, id: impl Into<SharedString>) -> Self {
        *self.0.key_mut() = Key::Id(id.into());
        self
    }
}

macro_rules! container {
    ($ty:ident) => {
        impl Styled for $ty {
            fn style(&mut self) -> &mut StyleRefinement {
                self.0.style()
            }
        }

        impl ParentElement for $ty {
            fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
                self.0.extend(elements);
            }
        }

        impl FluentBuilder for $ty {}

        impl IntoElement for $ty {
            fn into_any_element(self) -> AnyElement {
                self.0.into_any_element()
            }
        }
    };
}

container!(Form);
container!(FormSection);
container!(FormRow);
