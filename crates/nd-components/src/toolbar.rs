//! `toolbar()`: the window's toolbar, as intent. On macOS an `NSToolbar` with system items
//! (`toggleSidebar`, the sidebar tracking separator, flexible space) and one `NSToolbarItem`
//! per app item whose view is the item's native view; the item is a layout boundary sized to
//! its content. On Windows and Linux the title row the skin draws.

use std::panic::Location;

use nd_ui::prelude::*;
use nd_ui::{ContainerElement, Key, NativeElement, StyleRefinement};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ToolbarDisplay {
    #[default]
    Automatic,
    IconOnly,
    IconAndLabel,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ToolbarProps {
    pub title: Option<SharedString>,
    pub display: ToolbarDisplay,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ToolbarItemProps {
    /// Shown in the overflow menu and the customization sheet.
    pub label: Option<SharedString>,
    pub tooltip: Option<SharedString>,
    /// Sits at the leading edge, before the sidebar tracking separator: over the sidebar when
    /// there is one.
    pub leading: bool,
}

/// An item the OS draws and implements itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SystemItem {
    FlexibleSpace,
    Space,
    /// Collapses and expands the scaffold's sidebar.
    ToggleSidebar,
    ToggleInspector,
    /// Splits the toolbar over the sidebar's edge. Added automatically after the leading
    /// items when the window has a sidebar.
    SidebarTrackingSeparator,
    InspectorTrackingSeparator,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SystemItemProps {
    pub item: SystemItem,
}

/// A wrapper type naming the system item widget kind.
pub struct ToolbarSystemItem;

/// The widget kind of each app item's wrapper.
pub struct ToolbarItem;

#[track_caller]
pub fn system_item(item: SystemItem) -> AnyElement {
    NativeElement::new::<ToolbarSystemItem>(Key::Site(Location::caller()), SystemItemProps { item })
        .absolute()
        .into_any_element()
}

/// The system sidebar toggle.
#[track_caller]
pub fn toolbar_sidebar_toggle() -> AnyElement {
    system_item(SystemItem::ToggleSidebar)
}

pub struct Toolbar {
    root: ContainerElement<ToolbarProps>,
    count: usize,
}

#[track_caller]
pub fn toolbar() -> Toolbar {
    let root = ContainerElement::new::<Toolbar>(Key::Site(Location::caller()), ToolbarProps::default());
    // Out of the content's flow: on AppKit the items live in the window's toolbar, elsewhere
    // the scaffold places the row.
    Toolbar { root: root.absolute().top_0().left_0().flex().flex_row().items_center(), count: 0 }
}

impl Toolbar {
    fn push(mut self, item: impl IntoElement, props: ToolbarItemProps) -> Self {
        let key = Key::Id(format!("nd-toolbar-item-{}", self.count).into());
        self.count += 1;
        let wrapper = ContainerElement::new::<ToolbarItem>(key, props).flex().child(item);
        self.root = self.root.child(wrapper);
        self
    }

    pub fn item(self, item: impl IntoElement) -> Self {
        self.push(item, ToolbarItemProps::default())
    }

    /// An item with the label the overflow menu and customization show.
    pub fn labeled_item(self, label: impl Into<SharedString>, item: impl IntoElement) -> Self {
        self.push(item, ToolbarItemProps { label: Some(label.into()), ..ToolbarItemProps::default() })
    }

    /// Sits before the title: window controls embedded in the toolbar, a back button.
    pub fn leading(self, item: impl IntoElement) -> Self {
        self.push(item, ToolbarItemProps { leading: true, ..ToolbarItemProps::default() })
    }

    pub fn title(mut self, title: impl Into<SharedString>) -> Self {
        self.root.props_mut().title = Some(title.into());
        self
    }

    pub fn display(mut self, display: ToolbarDisplay) -> Self {
        self.root.props_mut().display = display;
        self
    }

    /// Pushes the items after it to the trailing edge.
    #[track_caller]
    pub fn flexible_space(mut self) -> Self {
        self.root = self.root.child(system_item(SystemItem::FlexibleSpace));
        self
    }

    #[track_caller]
    pub fn space(mut self) -> Self {
        self.root = self.root.child(system_item(SystemItem::Space));
        self
    }

    #[track_caller]
    pub fn sidebar_toggle(mut self) -> Self {
        self.root = self.root.child(system_item(SystemItem::ToggleSidebar));
        self
    }

    #[track_caller]
    pub fn inspector_toggle(mut self) -> Self {
        self.root = self.root.child(system_item(SystemItem::ToggleInspector));
        self
    }
}

impl Styled for Toolbar {
    fn style(&mut self) -> &mut StyleRefinement {
        self.root.style()
    }
}

impl IntoElement for Toolbar {
    fn into_any_element(self) -> AnyElement {
        self.root.into_any_element()
    }
}
