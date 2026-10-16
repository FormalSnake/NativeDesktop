//! Icons by Lucide name. Backends map the name to the platform set (SF Symbols on macOS) or
//! draw the vendored Lucide shape; see `nd-icons`. `icon` shows one on its own, outside a
//! button or a row.

use std::panic::Location;

pub use nd_icons::{Icon, register};
use nd_ui::prelude::*;
use nd_ui::semantics::{Semantics, WidgetSemantics};
use nd_ui::{Key, Role, StyleRefinement};

use crate::forms::leaf;

#[derive(Clone, Debug, PartialEq)]
pub struct IconProps {
    pub icon: Icon,
    /// The square it is drawn in, in points.
    pub size: f32,
    /// What it says to a screen reader; `None` leaves it out of the accessibility tree's names
    /// (a mark beside text that already says it).
    pub label: Option<SharedString>,
    pub enabled: bool,
    pub tooltip: Option<SharedString>,
    /// `text_color` tints it; without one it takes the secondary text colour.
    pub style: StyleRefinement,
}

/// Widget kind of `icon`.
pub struct IconKind;

impl WidgetSemantics for IconProps {
    fn semantics(&self) -> Semantics {
        Semantics::new(Role::Image, self.label.clone().unwrap_or_default().to_string()).enabled(self.enabled)
    }
}

pub struct IconView(NativeElement<IconProps>);

/// `icon` on its own in a `size` point square: an SF Symbol on macOS, the Lucide shape
/// elsewhere, tinted with the element's text colour.
#[track_caller]
pub fn icon(icon: Icon, size: f32) -> IconView {
    let props = IconProps { icon, size, label: None, enabled: true, tooltip: None, style: StyleRefinement::default() };
    let el = NativeElement::described::<IconKind>(Key::Site(Location::caller()), props).size(px(size)).flex_none();
    IconView(el)
}

impl IconView {
    pub fn id(mut self, id: impl Into<SharedString>) -> Self {
        *self.0.key_mut() = Key::Id(id.into());
        self
    }

    /// Names the icon for screen readers and automation.
    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.0.props_mut().label = Some(label.into());
        self
    }
}

leaf!(IconView, IconProps);
