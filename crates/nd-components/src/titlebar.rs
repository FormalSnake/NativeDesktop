//! `titlebar()`: a custom title row (ARCHITECTURE.md 3.6, 5.4) for windows opened with
//! `Titlebar::Custom`. It reserves the OS window controls' side and size as leading and
//! trailing insets the backend measures from the OS (traffic lights on the left on macOS,
//! caption buttons on the right on Windows, GNOME's `button-layout`, nothing on Hyprland), and
//! its empty area drags the window.

use std::panic::Location;

use nd_ui::prelude::*;
use nd_ui::{ContainerElement, Key, NativeElement, StyleRefinement, WindowControls};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum InsetEdge {
    #[default]
    Leading,
    Trailing,
}

/// A measured leaf: as wide as the OS controls on its edge need, as tall as the OS title bar.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct TitlebarInsetProps {
    pub edge: InsetEdge,
    /// The controls sit in an embedded slot in this row, so they need no inset.
    pub controls_in_row: bool,
}

/// Widget kind of the insets.
pub struct TitlebarInset;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TitlebarProps {
    pub style: StyleRefinement,
}

pub struct Titlebar {
    root: ContainerElement<TitlebarProps>,
    title: Option<SharedString>,
    leading: Vec<AnyElement>,
    trailing: Vec<AnyElement>,
    controls: Option<WindowControls>,
}

#[track_caller]
pub fn titlebar() -> Titlebar {
    let root = ContainerElement::new::<Titlebar>(Key::Site(Location::caller()), TitlebarProps::default());
    Titlebar {
        root: root.flex().flex_row().items_center().w_full().flex_none().gap_2(),
        title: None,
        leading: Vec::new(),
        trailing: Vec::new(),
        controls: None,
    }
}

impl Titlebar {
    /// Replaces the window's title, which the row shows when nothing else sits leading.
    pub fn title(mut self, title: impl Into<SharedString>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn leading(mut self, item: impl IntoElement) -> Self {
        self.leading.push(item.into_any_element());
        self
    }

    pub fn trailing(mut self, item: impl IntoElement) -> Self {
        self.trailing.push(item.into_any_element());
        self
    }

    /// Places the OS window controls itself (at a point, or in an embedded slot here) instead
    /// of where the OS puts them.
    pub fn controls(mut self, controls: WindowControls) -> Self {
        self.controls = Some(controls);
        self
    }
}

impl Styled for Titlebar {
    fn style(&mut self) -> &mut StyleRefinement {
        self.root.style()
    }
}

impl FluentBuilder for Titlebar {}

fn title_text(title: SharedString) -> AnyElement {
    text(title).font_weight(FontWeight::SEMIBOLD).into_any_element()
}

fn inset(edge: InsetEdge, controls_in_row: bool) -> AnyElement {
    let key = Key::Id(if edge == InsetEdge::Leading { "nd-inset-leading" } else { "nd-inset-trailing" }.into());
    NativeElement::new::<TitlebarInset>(key, TitlebarInsetProps { edge, controls_in_row })
        .flex_none()
        .into_any_element()
}

impl IntoElement for Titlebar {
    fn into_any_element(mut self) -> AnyElement {
        let embedded = self.controls.as_ref().is_some_and(|c| c.props().embedded);
        let style = self.root.style().clone();
        self.root.props_mut().style = style;
        // A row with nothing leading shows the window's title there, as the OS title bar would.
        let title = match self.title {
            Some(title) => Some(title_text(title)),
            None if self.leading.is_empty() => Some(deferred(|window, _| title_text(window.title().clone()))),
            None => None,
        };
        self.root
            .child(inset(InsetEdge::Leading, embedded))
            .children(self.controls)
            .children(self.leading)
            .children(title)
            .child(div().flex_1())
            .children(self.trailing)
            .child(inset(InsetEdge::Trailing, embedded))
            .into_any_element()
    }
}
