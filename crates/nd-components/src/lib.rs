//! Built-in components: the chrome intents (`scaffold`, `toolbar`, `sidebar`, `inspector`,
//! `titlebar`), `glass()`, menus, alerts and pickers, the override layers, button variants
//! and `separator`, and the forms family ([`forms`]: text fields, toggles, choices, values,
//! pickers, progress, labels, settings rows and `settings_form`). Each is a widget
//! kind every backend maps (ARCHITECTURE.md 3.6); this crate holds no platform code.

pub mod controls;
pub mod dialogs;
pub mod form;
pub mod forms;
pub mod glass;
pub mod icon;
pub mod menu;
pub mod overrides;
pub mod scaffold;
pub mod services;
pub mod source_list;
pub mod titlebar;
pub mod toolbar;

pub use controls::{ButtonExt, Mapping, Separator, SeparatorKind, SeparatorProps, icon_button, separator};
pub use dialogs::{Alert, AlertButton, AlertResponse, AlertStyle, ButtonRole, OpenDialog, SaveDialog};
pub use form::{Form, FormProps, FormRow, FormRowProps, FormSection, FormSectionProps, form, form_row, form_section};
pub use forms::*;
pub use glass::{Glass, GlassGroup, GlassGroupProps, GlassProps, glass, glass_group};
pub use icon::Icon;
pub use menu::{ContextMenu, ContextMenuRequested, Menu, MenuItem, MenuKind, MenuRole};
pub use overrides::{HasScope, OnPlatform, Pattern, Patterns, ScopedContainer, SubtreeOverrides, SubtreeScope};
pub use scaffold::{Pane, PaneCollapsed, PaneProps, PaneRole, Scaffold, ScaffoldProps, inspector, scaffold, sidebar};
pub use services::{Chrome, ChromeServices};
pub use source_list::{
    SourceItem, SourceList, SourceListKind, SourceListProps, SourceSection, SourceSelected, source_item, source_list,
};
pub use titlebar::{InsetEdge, Titlebar, TitlebarInset, TitlebarInsetProps, TitlebarProps, titlebar};
pub use toolbar::{
    SystemItem, SystemItemProps, TabSelected, Toolbar, ToolbarDisplay, ToolbarItem, ToolbarItemProps, ToolbarProps,
    ToolbarSystemItem, ToolbarTab, ToolbarTabProps, system_item, toolbar, toolbar_sidebar_toggle,
};
