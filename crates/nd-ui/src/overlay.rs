//! Overlays (ARCHITECTURE.md 6.3): popovers, floating panels and tooltips. An overlay is open
//! while it is in the tree; it never takes room in its parent's layout, and its parent is its
//! anchor, so it closes with the anchor's subtree.
//!
//! Dismissal is the app's: the backend or nd reports [`Dismissed`] (Esc, a press outside, the app
//! losing activation) and the app drops the overlay from its next render. Rules every backend
//! keeps:
//! - Input inside an overlay never reaches what lies under it: clicks, scrolls and hover, over
//!   native views and native surfaces (WKWebView, CEF, GPU surfaces) included.
//! - A press outside a `LightDismiss` overlay closes it and is consumed, on every OS: the element
//!   under the pointer sees nothing. `Modal` overlays consume outside presses and stay open.
//!   `Passive` overlays (tooltips) take no input at all.
//! - Esc closes the innermost non-passive overlay before any key binding sees it.
//! - Closing hands focus back to the node that had it when the overlay opened.

use std::panic::Location;

use nd_gpui::gpui::prelude::FluentBuilder;
use nd_gpui::gpui::{SharedString, StyleRefinement, Styled};

use crate::app::App;
use crate::backend::{NodeKind, WidgetKind};
use crate::container::ContainerElement;
use crate::element::{AnyElement, DispatchProps, InteractiveElement, IntoElement, Key, ParentElement};
use crate::window::Window;

/// The widget kind of every overlay node; [`OverlayProps::kind`] says which one.
pub struct Overlay;

/// What the overlay is, which decides how each platform presents it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum OverlayKind {
    /// Anchored to its parent: `NSPopover` on macOS, a drawn popover in the overlay layer on GPUI.
    #[default]
    Popover,
    /// A floating panel over the window's content (command palette, sheet-like panel), centred
    /// near the top of the window.
    Panel,
    /// A hint next to its parent; never takes input.
    Tooltip,
}

/// How an overlay treats input outside it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Modality {
    /// Outside presses are consumed and the overlay stays open.
    Modal,
    /// An outside press closes the overlay and is consumed.
    #[default]
    LightDismiss,
    /// Takes no input; what is under it gets everything.
    Passive,
}

/// Where an anchored overlay goes relative to its anchor. Panels ignore it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Placement {
    #[default]
    Below,
    Above,
    Leading,
    Trailing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DismissReason {
    Escape,
    /// A press outside the overlay, in any window of the app.
    OutsidePress,
    /// The app lost activation.
    Deactivated,
}

/// The overlay asks to close. Delivered to its `on_dismiss` handler.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dismissed {
    pub reason: DismissReason,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OverlayProps {
    pub kind: OverlayKind,
    pub modality: Modality,
    pub placement: Placement,
    /// The overlay's own style: padding, size limits and background of its content.
    pub style: StyleRefinement,
}

pub(crate) fn is_overlay(kind: NodeKind) -> bool {
    matches!(kind, NodeKind::Widget(w) if w == WidgetKind::of::<Overlay>())
}

/// An overlay element. Children are its content; `.on_dismiss(..)` hears the close requests.
pub struct OverlayElement(ContainerElement<OverlayProps>);

fn overlay(key: Key, kind: OverlayKind, modality: Modality, placement: Placement) -> OverlayElement {
    let props = OverlayProps { kind, modality, placement, style: StyleRefinement::default() };
    let mut element = ContainerElement::new::<Overlay>(key, props);
    // Out of the parent's flow: the overlay is sized by its content and placed by the backend.
    element.style().position = Some(nd_gpui::gpui::Position::Absolute);
    OverlayElement(element)
}

/// A popover anchored below its parent; light dismiss.
pub fn popover(id: impl Into<SharedString>) -> OverlayElement {
    overlay(Key::Id(id.into()), OverlayKind::Popover, Modality::LightDismiss, Placement::Below)
}

/// A floating panel over the window's content; modal until the app closes it or Esc.
pub fn panel(id: impl Into<SharedString>) -> OverlayElement {
    overlay(Key::Id(id.into()), OverlayKind::Panel, Modality::Modal, Placement::Below)
}

/// A tooltip above its parent; passive.
#[track_caller]
pub fn tooltip() -> OverlayElement {
    overlay(Key::Site(Location::caller()), OverlayKind::Tooltip, Modality::Passive, Placement::Above)
}

impl OverlayElement {
    pub fn modality(mut self, modality: Modality) -> Self {
        self.0.props_mut().modality = modality;
        self
    }

    pub fn placement(mut self, placement: Placement) -> Self {
        self.0.props_mut().placement = placement;
        self
    }

    pub fn props(&self) -> &OverlayProps {
        self.0.props()
    }

    pub fn on_dismiss(self, handler: impl Fn(&Dismissed, &mut Window, &mut App) + 'static) -> Self {
        OverlayElement(self.0.on(handler))
    }
}

impl Styled for OverlayElement {
    fn style(&mut self) -> &mut StyleRefinement {
        self.0.style()
    }
}

impl FluentBuilder for OverlayElement {}

impl ParentElement for OverlayElement {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.0.extend(elements);
    }
}

impl InteractiveElement for OverlayElement {
    fn dispatch_props(&mut self) -> &mut DispatchProps {
        self.0.dispatch_props()
    }
}

impl IntoElement for OverlayElement {
    fn into_any_element(mut self) -> AnyElement {
        let style = self.0.style().clone();
        self.0.props_mut().style = style;
        self.0.into_any_element()
    }
}
