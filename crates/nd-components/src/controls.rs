//! Controls the chrome and the override layers need now: button variants, icon buttons, the
//! picker (`select`) and the slider. The rest of the control set lands with `nd-components`'
//! M6 families on the same pattern: a leaf whose widget kind each backend maps.

use std::ops::RangeInclusive;
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

#[derive(Clone, Debug, PartialEq)]
pub struct SelectProps {
    pub options: Vec<SharedString>,
    pub selected: usize,
    pub mapping: Mapping,
    pub style: StyleRefinement,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SelectChanged {
    pub index: usize,
}

/// One of several options: a pop-up button by default, a segmented control or radio group
/// through [`Select::native`]. Controlled: `selected` is the app's.
pub struct Select(NativeElement<SelectProps>);

/// Widget kind of `select`.
pub struct SelectKind;

pub fn select(id: impl Into<SharedString>, options: impl IntoIterator<Item = impl Into<SharedString>>) -> Select {
    let props = SelectProps {
        options: options.into_iter().map(Into::into).collect(),
        selected: 0,
        mapping: Mapping::Automatic,
        style: StyleRefinement::default(),
    };
    Select(NativeElement::new::<SelectKind>(Key::Id(id.into()), props))
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

    pub fn on_change(self, f: impl Fn(&SelectChanged, &mut Window, &mut App) + 'static) -> Self {
        Select(self.0.on(f))
    }
}

impl Styled for Select {
    fn style(&mut self) -> &mut StyleRefinement {
        self.0.style()
    }
}

impl FluentBuilder for Select {}

impl IntoElement for Select {
    fn into_any_element(mut self) -> AnyElement {
        let style = self.0.style().clone();
        self.0.props_mut().style = style;
        self.0.into_any_element()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SliderProps {
    pub value: f32,
    pub min: f32,
    pub max: f32,
    /// The framework-drawn slider instead of the native control.
    pub drawn: bool,
    pub style: StyleRefinement,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SliderChanged {
    pub value: f32,
}

/// Widget kind of `slider`.
pub struct SliderKind;

pub struct Slider(NativeElement<SliderProps>);

pub fn slider(id: impl Into<SharedString>, value: f32) -> Slider {
    let props = SliderProps { value, min: 0., max: 1., drawn: false, style: StyleRefinement::default() };
    Slider(NativeElement::new::<SliderKind>(Key::Id(id.into()), props))
}

impl Slider {
    pub fn range(mut self, range: RangeInclusive<f32>) -> Self {
        let props = self.0.props_mut();
        props.min = *range.start();
        props.max = *range.end();
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

impl Styled for Slider {
    fn style(&mut self) -> &mut StyleRefinement {
        self.0.style()
    }
}

impl FluentBuilder for Slider {}

impl IntoElement for Slider {
    fn into_any_element(mut self) -> AnyElement {
        let style = self.0.style().clone();
        self.0.props_mut().style = style;
        self.0.into_any_element()
    }
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
