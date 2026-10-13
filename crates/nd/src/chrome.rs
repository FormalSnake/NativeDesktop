//! Composed stand-ins for the chrome intents: `scaffold()`, `toolbar()`, `sidebar()`,
//! `titlebar()`, `glass()` and `separator()`. `nd-components` replaces them with the native
//! mappings of ARCHITECTURE.md 3.6 (`NSSplitViewController`, `NSToolbar`, `NSGlassEffectView`);
//! the call sites stay the same.

use nd_gpui::gpui::Action;
use nd_ui::prelude::*;
use nd_ui::{DispatchProps, StyleRefinement, WindowControls};

use crate::theme::{Pattern, ThemeRefinement};

/// Forwards `Styled`, `ParentElement` and `IntoElement` to the `Div` a stand-in wraps.
macro_rules! wraps_div {
    ($ty:ident . $field:ident) => {
        impl Styled for $ty {
            fn style(&mut self) -> &mut StyleRefinement {
                self.$field.style()
            }
        }

        impl ParentElement for $ty {
            fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
                self.$field.extend(elements);
            }
        }

        impl FluentBuilder for $ty {}
    };
}

pub struct Scaffold {
    root: Div,
    toolbar: Option<Toolbar>,
    sidebar: Option<Sidebar>,
    content: Option<AnyElement>,
    overlay: Option<AnyElement>,
}

pub fn scaffold() -> Scaffold {
    Scaffold { root: v_flex().size_full(), toolbar: None, sidebar: None, content: None, overlay: None }
}

impl Scaffold {
    pub fn toolbar(mut self, toolbar: Toolbar) -> Self {
        self.toolbar = Some(toolbar);
        self
    }

    pub fn sidebar(mut self, sidebar: Sidebar) -> Self {
        self.sidebar = Some(sidebar);
        self
    }

    pub fn content(mut self, content: impl IntoElement) -> Self {
        self.content = Some(content.into_any_element());
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

impl IntoElement for Scaffold {
    fn into_any_element(self) -> AnyElement {
        let body = h_flex()
            .id("body")
            .flex_1()
            .min_h_0()
            .items_start()
            .children(self.sidebar.filter(|s| !s.collapsed))
            .child(div().id("content").flex_1().h_full().min_w_0().children(self.content));
        self.root.children(self.toolbar).child(body).children(self.overlay).into_any_element()
    }
}

pub struct Toolbar {
    leading: Vec<AnyElement>,
    title: Option<SharedString>,
    items: Vec<AnyElement>,
}

pub fn toolbar() -> Toolbar {
    Toolbar { leading: Vec::new(), title: None, items: Vec::new() }
}

impl Toolbar {
    pub fn item(mut self, item: impl IntoElement) -> Self {
        self.items.push(item.into_any_element());
        self
    }

    /// Sits before the title: window controls embedded in the toolbar, a back button.
    pub fn leading(mut self, item: impl IntoElement) -> Self {
        self.leading.push(item.into_any_element());
        self
    }

    pub fn title(mut self, title: impl Into<SharedString>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Pushes the items after it to the trailing edge.
    pub fn flexible_space(self) -> Self {
        self.item(div().flex_1())
    }
}

impl IntoElement for Toolbar {
    fn into_any_element(self) -> AnyElement {
        // On GPUI backends the toolbar is the window's title row: it moves the window and
        // keeps clear of the OS controls.
        drag_region()
            .id("toolbar")
            .titlebar_insets()
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .px_3()
            .py_2()
            .children(self.leading)
            .children(self.title.map(text))
            .children(self.items)
            .into_any_element()
    }
}

pub struct Sidebar {
    root: Div,
    collapsed: bool,
}

pub fn sidebar() -> Sidebar {
    Sidebar { root: v_flex().id("sidebar").h_full().w(px(220.)), collapsed: false }
}

impl Sidebar {
    pub fn collapsed(mut self, collapsed: bool) -> Self {
        self.collapsed = collapsed;
        self
    }
}

wraps_div!(Sidebar.root);

impl IntoElement for Sidebar {
    fn into_any_element(self) -> AnyElement {
        self.root.into_any_element()
    }
}

/// A custom title bar. It reserves the side and size of the OS window controls (5.4).
pub struct Titlebar {
    title: Option<SharedString>,
    trailing: Vec<AnyElement>,
    controls: Option<WindowControls>,
}

pub fn titlebar() -> Titlebar {
    Titlebar { title: None, trailing: Vec::new(), controls: None }
}

impl Titlebar {
    pub fn title(mut self, title: impl Into<SharedString>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn trailing(mut self, item: impl IntoElement) -> Self {
        self.trailing.push(item.into_any_element());
        self
    }

    /// Places the OS window controls itself instead of in the default slot.
    pub fn controls(mut self, controls: WindowControls) -> Self {
        self.controls = Some(controls);
        self
    }
}

impl IntoElement for Titlebar {
    fn into_any_element(self) -> AnyElement {
        drag_region()
            .id("titlebar")
            .titlebar_insets()
            .flex()
            .flex_row()
            .items_center()
            .h(px(52.))
            .px_3()
            .gap_2()
            .children(self.controls)
            .children(self.title.map(text))
            .child(div().flex_1())
            .children(self.trailing)
            .into_any_element()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GlassVariant {
    #[default]
    Regular,
    Clear,
}

/// Liquid Glass on macOS 26 and later, the platform material or a drawn blur elsewhere.
pub struct Glass {
    root: Div,
    variant: GlassVariant,
    tint: Option<Hsla>,
}

pub fn glass() -> Glass {
    Glass { root: v_flex(), variant: GlassVariant::default(), tint: None }
}

impl Glass {
    pub fn variant(mut self, variant: GlassVariant) -> Self {
        self.variant = variant;
        self
    }

    pub fn tint(mut self, tint: impl Into<Hsla>) -> Self {
        self.tint = Some(tint.into());
        self
    }
}

wraps_div!(Glass.root);

impl IntoElement for Glass {
    fn into_any_element(self) -> AnyElement {
        let root = self.root;
        match self.tint {
            Some(tint) => root.bg(tint),
            None => root,
        }
        .into_any_element()
    }
}

pub struct Separator {
    vertical: bool,
}

pub fn separator() -> Separator {
    Separator { vertical: false }
}

impl Separator {
    pub fn vertical(mut self) -> Self {
        self.vertical = true;
        self
    }
}

impl IntoElement for Separator {
    #[track_caller]
    fn into_any_element(self) -> AnyElement {
        let line = div().bg(hsla(0., 0., 0., 0.1));
        if self.vertical { line.w(px(1.)).h_full() } else { line.h(px(1.)).w_full() }.into_any_element()
    }
}

/// A context menu's items, built when it opens.
#[derive(Default)]
pub struct ContextMenu {
    pub items: Vec<(SharedString, Box<dyn Action>)>,
}

impl ContextMenu {
    pub fn item(mut self, label: impl Into<SharedString>, action: impl Action) -> Self {
        self.items.push((label.into(), Box::new(action)));
        self
    }
}

/// Subtree overrides (3.6): theme, pattern and a context menu on a container. Recorded by the
/// stand-ins; applied once `nd-theme` and `nd-components` land.
pub trait SubtreeOverrides: Sized {
    fn theme(self, _refine: fn(ThemeRefinement) -> ThemeRefinement) -> Self {
        self
    }

    fn pattern(self, _pattern: Pattern) -> Self {
        self
    }

    fn context_menu(self, _build: impl Fn(ContextMenu, &mut Window, &mut App) -> ContextMenu + 'static) -> Self {
        self
    }
}

impl SubtreeOverrides for Div {}
impl SubtreeOverrides for Glass {}
