//! Built-in components: the chrome intents (`scaffold`, `toolbar`, `sidebar`, `inspector`,
//! `titlebar`), `glass()`, menus, alerts and pickers, the override layers, button variants and
//! `separator`, the forms family ([`forms`]: text fields, toggles, choices, values, pickers,
//! progress, labels, settings rows and `settings_form`), and navigation and feedback: the
//! command palette, the source tree, toasts and banners; layout and windows: in-window tabs,
//! splits and the pane models behind them ([`panes`]), status pages and dialogs; media and
//! content ([`media`]: images, transport, lyrics, shelves, the composer, chat bubbles and
//! avatars). Each is a widget
//! kind every backend maps (ARCHITECTURE.md 3.6); this crate holds no platform code.

pub mod controls;
pub mod dialogs;
pub mod form;
pub mod forms;
pub mod fuzzy;
pub mod glass;
pub mod icon;
pub mod media;
pub mod menu;
pub mod overrides;
pub mod palette;
pub mod panes;
pub mod rich;
pub mod scaffold;
pub mod services;
pub mod source_list;
pub mod split;
pub mod status_page;
pub mod tabs;
pub mod titlebar;
pub mod toast;
pub mod toolbar;

pub use controls::{ButtonExt, Mapping, Separator, SeparatorKind, SeparatorProps, icon_button, menu_button, separator};
pub use dialogs::{
    AboutPanel, Alert, AlertButton, AlertPresentation, AlertResponse, AlertStyle, ButtonRole, OpenDialog, SaveDialog,
};
pub use form::{Form, FormProps, FormRow, FormRowProps, FormSection, FormSectionProps, form, form_row, form_section};
pub use forms::*;
pub use glass::{Glass, GlassGroup, GlassGroupProps, GlassProps, glass, glass_group};
pub use icon::{Icon, IconKind, IconProps, IconView, icon, register as register_icon};
pub use media::*;
pub use menu::{ContextMenu, ContextMenuRequested, Menu, MenuItem, MenuKind, MenuRole, primary_menu_items};
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
pub use split::{
    Split, SplitKind, SplitPane, SplitPaneKind, SplitPaneProps, SplitProps, SplitResized, split, split_pane,
};
pub use status_page::{StatusPage, StatusPageKind, StatusPageProps, status_page};
pub use tabs::{
    TabAddRequested, TabChanged, TabCloseRequested, TabPage, TabSpec, TabStrip, TabStripKind, TabStripProps, TabStyle,
    TabView, tab_page, tab_strip, tab_view,
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
