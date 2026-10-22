//! `scaffold()`: a window's structure, declared as intent (ARCHITECTURE.md 3.6, 3.10). On macOS
//! it is an `NSSplitViewController` whose sidebar is `NSSplitViewItem(sidebarWithViewController:)`
//! and whose inspector is `NSSplitViewItem(inspectorWithViewController:)`, with the toolbar an
//! `NSToolbar`; the panes are layout boundaries sized by the split view. GPUI backends draw
//! the same structure with the skin's tokens.

use std::panic::Location;

use nd_ui::prelude::*;
use nd_ui::{ContainerElement, DispatchProps, InteractiveElement, Key, StyleRefinement};

use crate::overrides::{HasScope, ScopedContainer, SubtreeScope};
use crate::toolbar::Toolbar;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ScaffoldProps {
    pub scope: SubtreeScope,
}

impl HasScope for ScaffoldProps {
    fn scope_mut(&mut self) -> &mut SubtreeScope {
        &mut self.scope
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum PaneRole {
    Sidebar,
    #[default]
    Content,
    Inspector,
    /// Floats over the whole window: palettes, in-window sheets.
    Overlay,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PaneProps {
    pub role: PaneRole,
    pub collapsed: bool,
    pub min_width: Option<f32>,
    pub max_width: Option<f32>,
    /// Width the pane opens at, where the platform lets the app choose.
    pub width: Option<f32>,
    /// The pane runs up under the window's toolbar: lists in it scroll under the bar, which
    /// blurs them, and rest clear of it; other content is the app's to keep clear.
    pub under_toolbar: bool,
    /// Points the toolbar's band reaches below its row over this pane (at least 18 on macOS).
    pub toolbar_extension: f32,
    /// The edge effect under the toolbar shows only while the pointer is over the toolbar.
    pub edge_on_hover: bool,
    /// Points a bar floating over the pane's foot takes (Music's mini player): the pane runs
    /// down under it, its scroll views scroll under the bar and rest clear of it, behind the
    /// system's scroll edge effect on macOS.
    pub bottom_bar: f32,
    /// While the sidebar is collapsed, the pointer at the window's leading edge slides it in
    /// over the content as a floating panel.
    pub edge_reveal: bool,
    pub style: StyleRefinement,
}

/// How far from the window's leading edge the pointer reveals a collapsed sidebar: narrow
/// enough that it never sits over anything the content draws.
pub const REVEAL_EDGE: f32 = 6.;

/// The collapsed sidebar slid in over the content (`revealed`) or away again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SidebarRevealed {
    pub revealed: bool,
}

/// Reported when the user collapses or expands a pane (the toolbar's sidebar button, a
/// divider drag, a narrow window).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaneCollapsed {
    pub collapsed: bool,
}

pub struct Scaffold {
    root: ContainerElement<ScaffoldProps>,
    toolbar: Option<Toolbar>,
    sidebar: Option<Pane>,
    content: Option<AnyElement>,
    inspector: Option<Pane>,
    overlay: Option<AnyElement>,
    content_under_toolbar: bool,
    toolbar_extension: f32,
    content_edge_on_hover: bool,
    content_bottom_bar: f32,
}

#[track_caller]
pub fn scaffold() -> Scaffold {
    let root = ContainerElement::new::<Scaffold>(Key::Site(Location::caller()), ScaffoldProps::default());
    Scaffold {
        root: root.size_full().flex().flex_row(),
        toolbar: None,
        sidebar: None,
        content: None,
        inspector: None,
        overlay: None,
        content_under_toolbar: false,
        toolbar_extension: 0.,
        content_edge_on_hover: false,
        content_bottom_bar: 0.,
    }
}

impl Scaffold {
    pub fn toolbar(mut self, toolbar: Toolbar) -> Self {
        self.toolbar = Some(toolbar);
        self
    }

    pub fn sidebar(mut self, sidebar: Pane) -> Self {
        self.sidebar = Some(sidebar.role(PaneRole::Sidebar));
        self
    }

    pub fn inspector(mut self, inspector: Pane) -> Self {
        self.inspector = Some(inspector.role(PaneRole::Inspector));
        self
    }

    pub fn content(mut self, content: impl IntoElement) -> Self {
        self.content = Some(content.into_any_element());
        self
    }

    /// The content runs up under the toolbar, the way a transcript scrolls under a unified
    /// toolbar on macOS: its lists keep their rows clear of the bar at rest.
    pub fn content_under_toolbar(mut self) -> Self {
        self.content_under_toolbar = true;
        self
    }

    /// A bar `points` tall floats over the content's foot: the content's scroll views scroll
    /// under it and rest clear of it. Zero where the bar docks below the content instead.
    pub fn content_bottom_bar(mut self, points: f32) -> Self {
        self.content_bottom_bar = points;
        self
    }

    /// The toolbar's band, and its blur, reach `points` further down over the content: room for
    /// what the content shows at its top under the toolbar (Messages' conversation name).
    pub fn toolbar_extension(mut self, points: f32) -> Self {
        self.toolbar_extension = points;
        self
    }

    /// The blur and bottom edge over content under the toolbar show only while the pointer is
    /// over the toolbar's band, as Messages' transcript on macOS 26; at rest the content runs
    /// under the toolbar untouched.
    pub fn content_edge_on_hover(mut self) -> Self {
        self.content_edge_on_hover = true;
        self
    }

    /// Floats over the content: palettes, sheets drawn in-window.
    pub fn overlay(mut self, overlay: impl IntoElement) -> Self {
        self.overlay = Some(overlay.into_any_element());
        self
    }
}

impl InteractiveElement for Scaffold {
    fn dispatch_props(&mut self) -> &mut DispatchProps {
        self.root.dispatch_props()
    }
}

impl ScopedContainer for Scaffold {
    fn update_scope(mut self, f: impl FnOnce(&mut SubtreeScope)) -> Self {
        self.root = self.root.update_scope(f);
        self
    }

    fn on_native<E: 'static>(mut self, handler: impl Fn(&E, &mut Window, &mut App) + 'static) -> Self {
        self.root = self.root.on(handler);
        self
    }
}

impl IntoElement for Scaffold {
    fn into_any_element(self) -> AnyElement {
        let under = self.content_under_toolbar;
        let extension = self.toolbar_extension;
        let edge_on_hover = self.content_edge_on_hover;
        let bottom_bar = self.content_bottom_bar;
        let content = self.content.map(|c| {
            let mut pane = pane_with(Key::Id("nd-content".into()), PaneRole::Content);
            pane.0.props_mut().under_toolbar = under;
            pane.0.props_mut().toolbar_extension = extension;
            pane.0.props_mut().edge_on_hover = edge_on_hover;
            pane.0.props_mut().bottom_bar = bottom_bar;
            pane.flex_1().min_w_0().h_full().child(c)
        });
        let overlay = self.overlay.map(|o| {
            pane_with(Key::Id("nd-overlay".into()), PaneRole::Overlay).absolute().top_0().left_0().size_full().child(o)
        });
        self.root
            .children(self.toolbar)
            .children(self.sidebar)
            .children(content)
            .children(self.inspector)
            .children(overlay)
            .into_any_element()
    }
}

/// A side pane of a [`scaffold`]: the sidebar or the inspector.
pub struct Pane(ContainerElement<PaneProps>);

fn pane_with(key: Key, role: PaneRole) -> Pane {
    Pane(ContainerElement::new::<Pane>(key, PaneProps { role, ..PaneProps::default() }).flex().flex_col())
}

/// The sidebar. On macOS its material, width and collapse come from the split view.
pub fn sidebar() -> Pane {
    pane_with(Key::Id("nd-sidebar".into()), PaneRole::Sidebar).w(px(220.)).h_full()
}

/// The inspector, on the trailing side.
pub fn inspector() -> Pane {
    pane_with(Key::Id("nd-inspector".into()), PaneRole::Inspector).w(px(260.)).h_full()
}

impl Pane {
    fn role(mut self, role: PaneRole) -> Self {
        self.0.props_mut().role = role;
        self
    }

    pub fn is_collapsed(&self) -> bool {
        self.0.props().collapsed
    }

    /// Controlled: the pane shows or hides when this changes. A collapse the user makes is
    /// reported through [`Pane::on_collapse_change`].
    pub fn collapsed(mut self, collapsed: bool) -> Self {
        self.0.props_mut().collapsed = collapsed;
        self
    }

    pub fn on_collapse_change(self, handler: impl Fn(&PaneCollapsed, &mut Window, &mut App) + 'static) -> Self {
        Pane(self.0.on(handler))
    }

    /// While collapsed, the sidebar slides in over the content as a floating panel when the
    /// pointer touches the window's leading edge, and away again 250 ms after the pointer leaves
    /// it (200 ms in, 150 ms out; a fade with reduce motion). The content is not resized, and
    /// window controls in the sidebar ride along. Arc's hidden sidebar.
    pub fn edge_reveal(mut self) -> Self {
        self.0.props_mut().edge_reveal = true;
        self
    }

    pub fn on_reveal_change(self, handler: impl Fn(&SidebarRevealed, &mut Window, &mut App) + 'static) -> Self {
        Pane(self.0.on(handler))
    }

    /// Width the pane opens at.
    pub fn width(mut self, width: Pixels) -> Self {
        self.0.props_mut().width = Some(width.as_f32());
        self.w(width)
    }

    /// The pane's content runs up under the toolbar, its lists scrolling under the bar's blur,
    /// as Messages' conversation list does under its search field.
    pub fn under_toolbar(mut self) -> Self {
        self.0.props_mut().under_toolbar = true;
        self
    }

    /// The toolbar's band, and its blur, reach `points` further down over this pane.
    pub fn toolbar_extension(mut self, points: f32) -> Self {
        self.0.props_mut().toolbar_extension = points;
        self
    }
}

impl Styled for Pane {
    fn style(&mut self) -> &mut StyleRefinement {
        self.0.style()
    }
}

impl ParentElement for Pane {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.0.extend(elements);
    }
}

impl FluentBuilder for Pane {}

impl IntoElement for Pane {
    fn into_any_element(mut self) -> AnyElement {
        let style = self.0.style().clone();
        let px_of = |l: Option<nd_gpui::gpui::Length>| match l {
            Some(nd_gpui::gpui::Length::Definite(nd_gpui::gpui::DefiniteLength::Absolute(a))) => {
                Some(a.to_pixels(px(16.)).as_f32())
            }
            _ => None,
        };
        let props = self.0.props_mut();
        props.min_width = px_of(style.min_size.width).or(props.min_width);
        props.max_width = px_of(style.max_size.width).or(props.max_width);
        props.style = style;
        self.0.into_any_element()
    }
}
