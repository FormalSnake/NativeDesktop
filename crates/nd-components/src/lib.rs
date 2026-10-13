//! Built-in components. Today: the chrome intents (`scaffold`, `toolbar`, `sidebar`,
//! `inspector`, `titlebar`), `glass()`, menus, alerts and pickers, the override layers, the
//! controls they need (button variants, `select`, `slider`, `separator`), and navigation and
//! feedback: the command palette, the source tree, toasts and banners. Each is a widget
//! kind every backend maps (ARCHITECTURE.md 3.6); this crate holds no platform code.

pub mod controls;
pub mod dialogs;
pub mod form;
pub mod glass;
pub mod icon;
pub mod menu;
pub mod overrides;
pub mod palette;
pub mod scaffold;
pub mod services;
pub mod source_list;
pub mod titlebar;
pub mod toast;
pub mod toolbar;

pub use controls::{
    ButtonExt, Mapping, Select, SelectChanged, SelectKind, SelectProps, Separator, SeparatorKind, SeparatorProps,
    Slider, SliderChanged, SliderKind, SliderProps, icon_button, select, separator, slider,
};
pub use dialogs::{Alert, AlertButton, AlertResponse, AlertStyle, ButtonRole, OpenDialog, SaveDialog};
pub use form::{Form, FormProps, FormRow, FormRowProps, FormSection, FormSectionProps, form, form_row, form_section};
pub use glass::{Glass, GlassGroup, GlassGroupProps, GlassProps, glass, glass_group};
pub use icon::Icon;
pub use menu::{ContextMenu, ContextMenuRequested, Menu, MenuItem, MenuKind, MenuRole};
pub use overrides::{HasScope, OnPlatform, Pattern, Patterns, ScopedContainer, SubtreeOverrides, SubtreeScope};
pub use palette::{
    CommandPalette, CommandPaletteKind, CommandPaletteProps, FieldSelection, PALETTE, PaletteActivated,
    PaletteCancelled, PaletteEvent, PaletteItem, PaletteKey, PaletteMetrics, PaletteModel, PaletteQueryChanged,
    PaletteSubmitted, command_palette, emit_palette_event, palette_item,
};
pub use scaffold::{Pane, PaneCollapsed, PaneProps, PaneRole, Scaffold, ScaffoldProps, inspector, scaffold, sidebar};
pub use services::{Chrome, ChromeServices};
pub use source_list::{
    ActionVisibility, EmptyState, SourceAction, SourceActionClicked, SourceActivated, SourceCollapsed, SourceExpanded,
    SourceIndex, SourceItem, SourceNode, SourceSelected, SourceTree, SourceTreeKind, SourceTreeProps, source_action,
    source_item, source_list, source_node, source_section, source_tree,
};
pub use titlebar::{InsetEdge, Titlebar, TitlebarInset, TitlebarInsetProps, TitlebarProps, titlebar};
pub use toast::{
    BANNER, Banner, BannerActionClicked, BannerKind, BannerMetrics, BannerProps, DEFAULT_TIMEOUT, DismissReason, TOAST,
    Toast, ToastActionClicked, ToastChange, ToastDismissed, ToastMetrics, ToastOverlay, ToastOverlayKind,
    ToastOverlayProps, ToastPriority, ToastQueue, banner, toast, toast_overlay,
};
pub use toolbar::{
    SystemItem, SystemItemProps, TabSelected, Toolbar, ToolbarDisplay, ToolbarItem, ToolbarItemProps, ToolbarProps,
    ToolbarSystemItem, ToolbarTab, ToolbarTabProps, system_item, toolbar, toolbar_sidebar_toggle,
};
