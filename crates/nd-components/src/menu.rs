//! Menus: the app's menu bar and context menus, built from actions so a menu item and its key
//! binding never disagree (ARCHITECTURE.md 3.3). macOS maps them to `NSMenu`; GPUI backends
//! draw them in the overlay stack.

use nd_gpui::gpui::{Action, SharedString};

use crate::icon::Icon;

/// An item the OS implements itself, placed where the platform expects it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MenuRole {
    About,
    Services,
    Hide,
    HideOthers,
    ShowAll,
    Quit,
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    SelectAll,
    Minimize,
    Zoom,
    BringAllToFront,
    ToggleFullScreen,
    /// Show Sidebar or Hide Sidebar, whichever applies, for a View menu. AppKit titles it from
    /// the window's split view; elsewhere it dispatches `nd::ToggleSidebar`.
    ToggleSidebar,
}

pub enum MenuItem {
    Action {
        label: SharedString,
        action: Box<dyn Action>,
        icon: Option<Icon>,
        destructive: bool,
        enabled: bool,
        checked: bool,
    },
    Submenu(Menu),
    Separator,
    Role(MenuRole),
}

impl MenuItem {
    pub fn action(label: impl Into<SharedString>, action: impl Action) -> Self {
        MenuItem::Action {
            label: label.into(),
            action: Box::new(action),
            icon: None,
            destructive: false,
            enabled: true,
            checked: false,
        }
    }

    pub fn separator() -> Self {
        MenuItem::Separator
    }

    pub fn submenu(menu: Menu) -> Self {
        MenuItem::Submenu(menu)
    }

    pub fn role(role: MenuRole) -> Self {
        MenuItem::Role(role)
    }

    pub fn icon(mut self, with: Icon) -> Self {
        if let MenuItem::Action { icon, .. } = &mut self {
            *icon = Some(with);
        }
        self
    }

    pub fn destructive(mut self) -> Self {
        if let MenuItem::Action { destructive, .. } = &mut self {
            *destructive = true;
        }
        self
    }

    pub fn enabled(mut self, on: bool) -> Self {
        if let MenuItem::Action { enabled, .. } = &mut self {
            *enabled = on;
        }
        self
    }

    pub fn checked(mut self, on: bool) -> Self {
        if let MenuItem::Action { checked, .. } = &mut self {
            *checked = on;
        }
        self
    }
}

/// Where a top-level menu sits and which system items the platform adds to it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MenuKind {
    #[default]
    Normal,
    /// The application menu: About, Settings, Services, Hide and Quit come from the OS.
    App,
    /// Undo, Redo, Cut, Copy, Paste, Select All.
    Edit,
    /// Minimize, Zoom, the window list.
    Window,
    /// The help menu, with the OS's search field on macOS.
    Help,
}

pub struct Menu {
    pub title: SharedString,
    pub kind: MenuKind,
    pub items: Vec<MenuItem>,
}

impl Menu {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self { title: title.into(), kind: MenuKind::Normal, items: Vec::new() }
    }

    /// The application menu; `items` go after About, before Services, Hide and Quit.
    pub fn app(items: impl IntoIterator<Item = MenuItem>) -> Self {
        Self { title: SharedString::default(), kind: MenuKind::App, items: items.into_iter().collect() }
    }

    /// The standard Edit menu; `items` follow the system ones.
    pub fn edit(items: impl IntoIterator<Item = MenuItem>) -> Self {
        Self { title: "Edit".into(), kind: MenuKind::Edit, items: items.into_iter().collect() }
    }

    pub fn window(items: impl IntoIterator<Item = MenuItem>) -> Self {
        Self { title: "Window".into(), kind: MenuKind::Window, items: items.into_iter().collect() }
    }

    pub fn help(items: impl IntoIterator<Item = MenuItem>) -> Self {
        Self { title: "Help".into(), kind: MenuKind::Help, items: items.into_iter().collect() }
    }

    pub fn items(mut self, items: impl IntoIterator<Item = MenuItem>) -> Self {
        self.items.extend(items);
        self
    }

    pub fn item(mut self, item: MenuItem) -> Self {
        self.items.push(item);
        self
    }
}

/// A context menu's items, built when it opens.
#[derive(Default)]
pub struct ContextMenu {
    pub items: Vec<MenuItem>,
    /// Opens with its bottom edge at the point instead of its top: a menu button at the
    /// bottom of a window, as Messages' composer "+" opens above itself.
    pub above: bool,
    /// Shows the rows' icons where the platform would hide them: macOS 27 hides menu item
    /// images unless an item asks for them, and Messages' "+" menu asks.
    pub icons_shown: bool,
}

impl ContextMenu {
    pub fn above(mut self) -> Self {
        self.above = true;
        self
    }

    pub fn show_icons(mut self) -> Self {
        self.icons_shown = true;
        self
    }

    pub fn item(mut self, label: impl Into<SharedString>, action: impl Action) -> Self {
        self.items.push(MenuItem::action(label, action));
        self
    }

    pub fn item_destructive(mut self, label: impl Into<SharedString>, action: impl Action) -> Self {
        self.items.push(MenuItem::action(label, action).destructive());
        self
    }

    pub fn separator(mut self) -> Self {
        self.items.push(MenuItem::Separator);
        self
    }

    pub fn submenu(mut self, menu: Menu) -> Self {
        self.items.push(MenuItem::Submenu(menu));
        self
    }

    pub fn entry(mut self, item: MenuItem) -> Self {
        self.items.push(item);
        self
    }
}

/// Reported by a backend when the user asks for a node's context menu (right click,
/// control-click, the menu key). `x`/`y` are in the node's coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ContextMenuRequested {
    pub node: nd_core::NodeId,
    pub x: f32,
    pub y: f32,
}
