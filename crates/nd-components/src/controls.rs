//! Button variants, icon buttons, the control `Mapping` and `separator`. The form controls
//! live in [`crate::forms`].

use std::panic::Location;

use nd_ui::prelude::*;
use nd_ui::{Bezel, ButtonVariant, Key, NativeElement, StyleRefinement};

use crate::icon::Icon;

/// Button variants beyond the label.
pub trait ButtonExt: Sized {
    fn icon(self, icon: Icon) -> Self;
    /// Clicking dispatches `action` from the focused node, as a menu item or key binding would.
    fn dispatch(self, action: impl nd_ui::Action) -> Self;
    fn tooltip(self, tooltip: impl Into<SharedString>) -> Self;
    /// macOS bezel; ignored elsewhere.
    fn bezel(self, bezel: Bezel) -> Self;
    /// The window's default action (Return).
    fn primary(self) -> Self;
    /// Filled with the accent colour.
    fn accent(self) -> Self;
    fn outline(self) -> Self;
    fn destructive(self) -> Self;
    /// Filled with the accent colour: the prominent style (`.accent()` under its HIG name).
    fn prominent(self) -> Self;
    /// No bezel until hovered or pressed.
    fn borderless(self) -> Self;
    /// The toolbar's own button style.
    fn toolbar_style(self) -> Self;
}

impl ButtonExt for Button {
    fn icon(self, icon: Icon) -> Self {
        self.icon_name(icon.lucide())
    }

    fn dispatch(self, action: impl nd_ui::Action) -> Self {
        self.on_click(move |_: &ClickEvent, window, cx| {
            cx.dispatch_action(window.id(), &*action.boxed_clone());
        })
    }

    fn tooltip(mut self, tooltip: impl Into<SharedString>) -> Self {
        self.props_mut().tooltip = Some(tooltip.into());
        self
    }

    fn bezel(self, bezel: Bezel) -> Self {
        Button::bezel(self, bezel)
    }

    fn primary(self) -> Self {
        self.variant(ButtonVariant::Primary)
    }

    fn accent(self) -> Self {
        self.variant(ButtonVariant::Accent)
    }

    fn outline(self) -> Self {
        self.variant(ButtonVariant::Outline)
    }

    fn destructive(self) -> Self {
        self.variant(ButtonVariant::Destructive)
    }

    fn prominent(self) -> Self {
        self.variant(ButtonVariant::Accent)
    }

    fn borderless(self) -> Self {
        Button::bezel(self, Bezel::Borderless)
    }

    fn toolbar_style(self) -> Self {
        Button::bezel(self, Bezel::Toolbar)
    }
}

/// A button showing only `icon`; the platform's toolbar or borderless style.
pub fn icon_button(id: impl Into<SharedString>, icon: Icon) -> Button {
    button(id, "").icon(icon).bezel(Bezel::Borderless)
}

/// How a control is presented, where the platform has more than one control for it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Mapping {
    #[default]
    Automatic,
    PopUp,
    Segmented,
    Radio,
}

/// A separator line: the platform's own (`NSBox` separator on macOS).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct SeparatorProps {
    pub vertical: bool,
}

/// Widget kind of `separator`.
pub struct SeparatorKind;

pub struct Separator(NativeElement<SeparatorProps>);

#[track_caller]
pub fn separator() -> Separator {
    Separator(NativeElement::new::<SeparatorKind>(Key::Site(Location::caller()), SeparatorProps::default()))
}

impl Separator {
    pub fn vertical(mut self) -> Self {
        self.0.props_mut().vertical = true;
        self
    }
}

impl Styled for Separator {
    fn style(&mut self) -> &mut StyleRefinement {
        self.0.style()
    }
}

impl IntoElement for Separator {
    fn into_any_element(self) -> AnyElement {
        let vertical = self.0.props().vertical;
        let el = if vertical { self.0.h_full() } else { self.0.w_full() };
        el.into_any_element()
    }
}
