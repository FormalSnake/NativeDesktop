//! The AppKit side of the backend seam (ARCHITECTURE.md 3.15). Retained nodes own NSViews; one
//! transaction is one `CATransaction` with implicit animations off.
//!
//! Chrome intents map to the highest-level system component (3.10): `scaffold()` to an
//! `NSSplitViewController` with sidebar and inspector items, `toolbar()` to the window's
//! `NSToolbar`, `glass()` to `NSGlassEffectView`. Containers that place their own children
//! (split panes) are layout boundaries: [`Backend::boundary_size`] reports the size they give.
//!
//! Theme: native controls draw with system colours. What an override changes (an accent, a
//! radius scale, a forced appearance) reaches them through [`NativeTheme`], per window from the
//! app and per subtree from a container's [`SubtreeScope`], applied after each transaction.

use std::any::TypeId;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};

use nd_components::{
    BannerKind, BannerProps, CommandPaletteKind, CommandPaletteProps, Form, FormRow, FormRowProps, FormSection,
    FormSectionProps, Glass, GlassGroup, GlassGroupProps, GlassProps, Pane, PaneProps, PaneRole, Scaffold,
    ScaffoldProps, SeparatorKind, SeparatorProps, SourceTreeKind, SourceTreeProps, SplitKind, SplitPaneKind,
    SplitPaneProps, SplitProps, StatusPageKind, StatusPageProps, SubtreeScope, SystemItemProps, Titlebar,
    TitlebarInset, TitlebarInsetProps, TitlebarProps, ToastOverlayKind, ToastOverlayProps, Toolbar, ToolbarItem,
    ToolbarItemProps, ToolbarProps, ToolbarSystemItem, ToolbarTab, ToolbarTabProps,
};
use nd_core::{NodeId, Rect, Size, WindowId};
use nd_theme::{NativeTheme, Scheme, Theme, ThemeRefinement, ThemeRegistry};
use nd_ui::backend::{AvailableSpace, Backend, Constraint, NodeKind, Props, PropsPatch, SurfaceOp};
use nd_ui::events::EventQueue;
use nd_ui::overlay::Overlay;
use nd_ui::surface::NativeSurface;
use nd_ui::{
    Button, ButtonProps, DivProps, OverlayProps, RootProps, SurfaceProps, WindowControls, WindowControlsProps,
};
use objc2::Message;
use objc2::rc::Retained;
use objc2_app_kit::{NSAppearanceCustomization, NSApplication, NSBox, NSView, NSWindowOrderingMode};
use objc2_foundation::{MainThreadMarker, NSAlignmentOptions};
use objc2_quartz_core::CATransaction;

use crate::NdWindow;
use crate::button::NativeButtonView;
use crate::events::Wake;
use crate::form::{FormNative, RowSpec, SectionSpec};
use crate::forms::FormControl;
use crate::glass::{GlassGroupNative, GlassNative};
use crate::look::{Color, TextLook, ViewLook};
use crate::overlay::OverlayNative;
use crate::palette::PaletteNative;
use crate::scaffold::{PaneNative, ScaffoldNative};
use crate::source_list::SourceTreeNative;
use crate::split::{SplitNative, SplitPaneNative};
use crate::status_page::StatusPageNative;
use crate::surface::SurfaceNative;
use crate::system_colors::appearance_for;
use crate::toast::{BannerNative, ToastNative};
use crate::toolbar::{Entry, ToolbarItemNative, ToolbarNative, ToolbarTabNative};
use crate::traffic_lights::TrafficLights;
use crate::views::{NdLayerView, NdRootView, NdTextView, ns_rect};
use crate::virtual_list::ListNative;

mod automation;
mod nav;

pub use nav::{PaletteLayout, PaletteRowLayout};

#[allow(clippy::large_enum_variant)]
enum Native {
    Root(Retained<NdLayerView>),
    Layer(Retained<NdLayerView>),
    Text(Retained<NdTextView>),
    Button(NativeButtonView),
    /// A `window_controls()` node: it places the window's own buttons. Embedded, `slot` is the
    /// layout box they are moved onto.
    WindowControls {
        window: WindowId,
        slot: Retained<NdLayerView>,
        props: WindowControlsProps,
    },
    /// A view nd did not create (a toolbar item's field until toolbars are nodes), registered
    /// so focus and the key view loop can name it.
    External(Retained<NSView>),
    Glass(GlassNative),
    GlassGroup(GlassGroupNative),
    Scaffold(ScaffoldNative),
    Pane(PaneNative),
    Toolbar(ToolbarNative),
    ToolbarItem(ToolbarItemNative),
    SystemItem,
    ToolbarTab(ToolbarTabNative),
    Titlebar(Retained<NdLayerView>),
    Inset(WindowId, TitlebarInsetProps),
    /// A forms control (`crate::forms`): text fields, toggles, choices, values, pickers.
    Control(ControlNode),
    Separator(Retained<NSBox>, bool),
    SourceTree(SourceTreeNative),
    Palette(PaletteNative),
    Toasts(ToastNative),
    Banner(BannerNative),
    /// An `nd-virtual` list, table or tree.
    List(ListNative),
    Form(FormNative),
    /// A widget a crate outside the backend registered (`crate::custom`).
    Custom(Box<dyn crate::custom::CustomNative>),
    /// Logical: the form draws sections and rows; these hold what goes in them.
    FormSection(FormSectionProps, Vec<NodeId>),
    FormRow(FormRowProps, Option<NodeId>),
    /// Its own window above the owner; never a subview of its parent, which is its anchor.
    Overlay(OverlayNative),
    Surface(SurfaceNative),
    /// `split()`: an `NSSplitView`; its two panes are layout boundaries it frames.
    Split(SplitNative),
    SplitPane(SplitPaneNative),
    StatusPage(StatusPageNative),
}

pub(crate) struct ControlNode {
    pub control: Box<dyn FormControl>,
    pub kind: nd_ui::backend::WidgetKind,
    pub props: Props,
}

impl Native {
    /// The view the parent holds.
    fn view(&self) -> Option<&NSView> {
        match self {
            Native::Root(v) | Native::Layer(v) | Native::Titlebar(v) => Some(v),
            Native::Text(v) => Some(v),
            Native::Button(b) => Some(&b.button),
            Native::External(v) => Some(v),
            Native::WindowControls { slot, props, .. } => props.embedded.then_some(&**slot),
            Native::Glass(g) => Some(g.view()),
            Native::GlassGroup(g) => Some(g.view()),
            Native::Scaffold(s) => Some(&s.host),
            Native::Pane(p) => Some(p.outer()),
            Native::ToolbarItem(i) => Some(&i.host),
            Native::Control(c) => Some(c.control.view()),
            Native::Separator(b, _) => Some(b),
            Native::SourceTree(s) => Some(s.view()),
            Native::Palette(p) => Some(p.view()),
            Native::Toasts(t) => Some(t.view()),
            Native::Banner(b) => Some(b.view()),
            Native::List(l) => Some(l.view()),
            Native::Form(f) => Some(&f.outer),
            Native::Surface(s) => Some(s.view()),
            Native::Custom(c) => Some(c.view()),
            Native::Split(s) => Some(&s.view),
            Native::SplitPane(p) => Some(&p.host),
            Native::StatusPage(p) => Some(p.view()),
            Native::FormSection(..) | Native::FormRow(..) | Native::Overlay(_) => None,
            Native::Toolbar(_) | Native::SystemItem | Native::ToolbarTab(_) | Native::Inset(..) => None,
        }
    }

    /// The view that takes first responder when nd focuses the node.
    fn key_view(&self) -> Option<Retained<NSView>> {
        match self {
            Native::Control(c) => Some(c.control.focus_view().retain()),
            Native::Custom(c) => c.focus_view().or_else(|| Some(c.view().retain())),
            _ => self.view().map(|v| v.retain()),
        }
    }

    /// The view children are added to.
    fn container(&self) -> Option<&NSView> {
        match self {
            Native::Root(v) | Native::Layer(v) | Native::Titlebar(v) => Some(v),
            Native::Glass(g) => Some(&g.content),
            Native::GlassGroup(g) => Some(&g.content),
            Native::Pane(p) => Some(&p.content),
            Native::Scaffold(s) => Some(&s.host),
            Native::ToolbarItem(i) => Some(&i.host),
            Native::Overlay(o) => Some(&o.content),
            // NSSplitView arranges every subview it is given.
            Native::Split(s) => Some(&s.view),
            Native::SplitPane(p) => Some(&p.content),
            Native::Custom(c) => c.container(),
            _ => None,
        }
    }
}

pub(crate) struct WindowRecord {
    pub window: Retained<NdWindow>,
    pub root: Retained<NdRootView>,
    pub lights: Rc<TrafficLights>,
}

struct State {
    mtm: MainThreadMarker,
    nodes: HashMap<NodeId, Native>,
    windows: HashMap<WindowId, WindowRecord>,
    open: Option<WindowId>,
    events: EventQueue,
    wake: Wake,
    expose: bool,
    parents: HashMap<NodeId, NodeId>,
    node_window: HashMap<NodeId, WindowId>,
    scopes: HashMap<NodeId, SubtreeScope>,
    window_native: HashMap<WindowId, NativeTheme>,
    /// What windows with no theme of their own get: the app-level override.
    default_native: NativeTheme,
    system_scheme: Scheme,
    theme_dirty: HashSet<WindowId>,
    system_items: HashMap<NodeId, SystemItemProps>,
    /// Forms whose sections, rows or row controls changed in this transaction.
    forms_dirty: HashSet<NodeId>,
    /// The size each form row's control was laid out at, which the form shows it at.
    form_sizes: HashMap<NodeId, (f64, f64)>,
    /// `nd-virtual` row roots and table cells, by the tag their div carries.
    row_tags: HashMap<NodeId, nd_virtual::RowTag>,
    cell_tags: HashMap<NodeId, usize>,
    /// `overflow_*_scroll()` divs: the scroll view inside the div's layer view and the document
    /// their children go in.
    scrolls: HashMap<NodeId, crate::scroll_view::ScrollBox>,
    /// Text nodes with rich text, whose colour tokens resolve again when their theme changes.
    rich_text: HashMap<NodeId, Props>,
    /// The app's resolved themes, for custom widgets that draw their own content.
    registry: Option<ThemeRegistry>,
    weak: Weak<RefCell<State>>,
}

/// The AppKit backend. A cheap handle: `nd-ui`'s `App` owns one clone as its `Box<dyn Backend>`,
/// window hosts and the dispatcher keep others.
#[derive(Clone)]
pub struct AppKitBackend(Rc<RefCell<State>>);

fn props<'a, T: 'static>(props: &'a Props, kind: &str) -> &'a T {
    props.downcast_ref::<T>().unwrap_or_else(|| panic!("{kind} node needs {} props", std::any::type_name::<T>()))
}

fn is<W: 'static>(kind: nd_ui::backend::WidgetKind) -> bool {
    kind.type_id == TypeId::of::<W>()
}

/// The subtree scope a node's props carry, if any.
fn scope_of(p: &Props) -> Option<SubtreeScope> {
    if let Some(d) = p.downcast_ref::<DivProps>() {
        return d.scope.as_ref().and_then(|s| s.get::<SubtreeScope>()).cloned();
    }
    if let Some(g) = p.downcast_ref::<GlassProps>() {
        return Some(g.scope.clone());
    }
    p.downcast_ref::<ScaffoldProps>().map(|s| s.scope.clone())
}

fn gesture_host(div: &DivProps, emit: nd_ui::events::Emit, wake: &Wake) -> Option<crate::gestures::GestureHost> {
    (!div.gestures.is_empty()).then(|| crate::gestures::GestureHost { set: div.gestures, emit, wake: wake.clone() })
}

fn insert_before(list: &mut Vec<NodeId>, node: NodeId, before: Option<NodeId>) {
    list.retain(|n| *n != node);
    let at = before.and_then(|b| list.iter().position(|n| *n == b)).unwrap_or(list.len());
    list.insert(at, node);
}

impl AppKitBackend {
    pub fn new(mtm: MainThreadMarker) -> Self {
        crate::media::register();
        let state = Rc::new_cyclic(|weak| {
            RefCell::new(State {
                mtm,
                nodes: HashMap::new(),
                windows: HashMap::new(),
                open: None,
                events: EventQueue::default(),
                wake: Wake::default(),
                expose: std::env::var_os("ND_DEBUG_EXPOSE").is_some_and(|v| v == "1"),
                parents: HashMap::new(),
                node_window: HashMap::new(),
                scopes: HashMap::new(),
                window_native: HashMap::new(),
                default_native: NativeTheme::default(),
                system_scheme: Scheme::Light,
                theme_dirty: HashSet::new(),
                system_items: HashMap::new(),
                forms_dirty: HashSet::new(),
                form_sizes: HashMap::new(),
                row_tags: HashMap::new(),
                cell_tags: HashMap::new(),
                scrolls: HashMap::new(),
                rich_text: HashMap::new(),
                registry: None,
                weak: weak.clone(),
            })
        });
        Self(state)
    }

    /// Runs `callback` on the main run loop turn after a native callback queued an event.
    pub fn on_wake(&self, callback: impl Fn() + 'static) {
        self.0.borrow().wake.set_callback(callback);
    }

    pub(crate) fn add_window(&self, id: WindowId, record: WindowRecord) {
        let mut state = self.0.borrow_mut();
        let native = state.window_native.get(&id).cloned();
        state.windows.insert(id, record);
        if let Some(native) = native {
            state.apply_window_appearance(id, &native);
            // Controls created before the window existed resolved their colours under the
            // app's appearance; the window's may differ.
            state.apply_theme(id);
            state.retheme_custom(Some(id));
        }
    }

    pub(crate) fn remove_window(&self, id: WindowId) {
        if let Ok(mut state) = self.0.try_borrow_mut() {
            state.windows.remove(&id);
        }
    }

    pub fn register_external(&self, node: NodeId, view: &NSView) {
        self.0.borrow_mut().nodes.insert(node, Native::External(view.retain()));
    }

    pub fn view(&self, node: NodeId) -> Option<Retained<NSView>> {
        self.0.borrow().nodes.get(&node)?.view().map(|v| v.retain())
    }

    /// The view that takes first responder when nd focuses `node`.
    pub fn key_view(&self, node: NodeId) -> Option<Retained<NSView>> {
        self.0.borrow().nodes.get(&node)?.key_view()
    }

    /// The node whose view is `view`, if nd created or registered it. `None` while a
    /// transaction is applying.
    /// The `nd-virtual` lists, by node, oldest first.
    pub fn lists(&self) -> Vec<NodeId> {
        let state = self.0.borrow();
        let mut lists: Vec<NodeId> =
            state.nodes.iter().filter(|(_, n)| matches!(n, Native::List(_))).map(|(id, _)| *id).collect();
        lists.sort();
        lists
    }

    /// The scroll views of `overflow_*_scroll()` divs, by node, oldest first.
    pub fn scroll_views(&self) -> Vec<(NodeId, Retained<objc2_app_kit::NSScrollView>)> {
        let state = self.0.borrow();
        let mut out: Vec<_> = state.scrolls.iter().map(|(n, s)| (*n, s.scroll.clone())).collect();
        out.sort_by_key(|(n, _)| *n);
        out
    }

    /// Titles of the native row actions the list shows for its `row`th row on one edge.
    pub fn list_row_actions(&self, list: NodeId, row: usize, leading: bool) -> Vec<String> {
        match self.0.borrow().nodes.get(&list) {
            Some(Native::List(l)) => l.row_action_titles(row, leading),
            _ => Vec::new(),
        }
    }

    /// The `nd-virtual` row a row root node shows.
    pub fn list_row(&self, node: NodeId) -> Option<nd_virtual::RowId> {
        self.0.borrow().row_tags.get(&node).map(|t| t.row)
    }

    /// The view presenting the list (an `NSTableView`, `NSOutlineView` or `NSCollectionView`)
    /// and its scroll view.
    pub fn list_views(&self, list: NodeId) -> Option<(Retained<NSView>, Retained<objc2_app_kit::NSScrollView>)> {
        let state = self.0.borrow();
        match state.nodes.get(&list)? {
            Native::List(l) => Some((l.document().retain(), l.scroll().retain())),
            _ => None,
        }
    }

    pub fn node_for_view(&self, view: *const NSView) -> Option<NodeId> {
        let state = self.0.try_borrow().ok()?;
        state.nodes.iter().find(|(_, n)| n.view().is_some_and(|v| std::ptr::eq(v, view))).map(|(id, _)| *id)
    }

    /// The background colour the node's layer shows now, in sRGB: what the last display pass
    /// put on screen, for tests that time a recolour.
    pub fn layer_background(&self, node: NodeId) -> Option<nd_ui::prelude::Hsla> {
        let state = self.0.try_borrow().ok()?;
        let cg = state.nodes.get(&node)?.view()?.layer()?.backgroundColor()?;
        let color = objc2_app_kit::NSColor::colorWithCGColor(&cg)?
            .colorUsingColorSpace(&objc2_app_kit::NSColorSpace::sRGBColorSpace())?;
        let rgba = nd_ui::prelude::Rgba {
            r: color.redComponent() as f32,
            g: color.greenComponent() as f32,
            b: color.blueComponent() as f32,
            a: color.alphaComponent() as f32,
        };
        Some(rgba.into())
    }

    /// Times the split placed its panes for a ratio the app set; a ratio fed back from the
    /// split's own report places nothing.
    pub fn split_placements(&self, split: NodeId) -> Option<usize> {
        match self.0.borrow().nodes.get(&split)? {
            Native::Split(s) => Some(s.placements()),
            _ => None,
        }
    }

    /// The window's NSWindow, once its host is open.
    pub fn ns_window(&self, window: WindowId) -> Option<Retained<NdWindow>> {
        self.0.borrow().windows.get(&window).map(|r| r.window.clone())
    }

    /// The app window whose NSWindow is `window`.
    pub fn window_id_of(&self, window: &objc2_app_kit::NSWindow) -> Option<WindowId> {
        let state = self.0.try_borrow().ok()?;
        state
            .windows
            .iter()
            .find(|(_, r)| std::ptr::eq::<objc2_app_kit::NSWindow>(&**r.window, window))
            .map(|(id, _)| *id)
    }

    /// The app windows sharing `window`'s native tab group, in AppKit's tab order; just the
    /// window itself when it has no tabs.
    pub fn tab_group_windows(&self, window: WindowId) -> Vec<WindowId> {
        let Some(ns) = self.ns_window(window) else { return Vec::new() };
        match ns.tabGroup().map(|g| g.windows()) {
            Some(windows) if windows.count() > 0 => windows.iter().filter_map(|w| self.window_id_of(&w)).collect(),
            _ => vec![window],
        }
    }

    /// The app window whose tab `window`'s tab group shows.
    pub fn selected_tab(&self, window: WindowId) -> Option<WindowId> {
        let selected = self.ns_window(window)?.tabGroup()?.selectedWindow()?;
        self.window_id_of(&selected)
    }

    /// The window a node lives in now.
    pub fn node_window(&self, node: NodeId) -> Option<WindowId> {
        self.0.borrow().node_window.get(&node).copied()
    }

    /// The scheme windows follow when nothing forces one; subtree overrides resolve against it.
    pub fn set_system_scheme(&self, scheme: Scheme) {
        let mut state = self.0.borrow_mut();
        if state.system_scheme != scheme {
            state.system_scheme = scheme;
            let windows: Vec<WindowId> = state.windows.keys().copied().collect();
            state.theme_dirty.extend(windows);
            if state.open.is_none() {
                state.apply_dirty_themes();
            }
        }
    }

    /// What native controls take from the window's theme (app and window overrides merged).
    pub fn set_window_theme(&self, window: WindowId, theme: NativeTheme) {
        let mut state = self.0.borrow_mut();
        if state.window_native.get(&window) == Some(&theme) {
            return;
        }
        state.apply_window_appearance(window, &theme);
        state.window_native.insert(window, theme);
        state.theme_dirty.insert(window);
        if state.open.is_none() {
            state.apply_dirty_themes();
        }
    }

    /// The theme windows without one of their own get (the app-level override).
    pub fn set_default_theme(&self, theme: NativeTheme) {
        let mut state = self.0.borrow_mut();
        if state.default_native != theme {
            state.default_native = theme;
            let windows: Vec<WindowId> = state.windows.keys().copied().collect();
            state.theme_dirty.extend(windows);
            if state.open.is_none() {
                state.apply_dirty_themes();
            }
        }
    }

    /// The app's theme registry, which custom widgets draw their own content from.
    pub fn set_theme_registry(&self, registry: ThemeRegistry) {
        let mut state = self.0.borrow_mut();
        state.registry = Some(registry);
        state.retheme_custom(None);
    }

    /// The app's forced appearance (`NSApp.appearance`); `None` follows the system.
    pub fn set_app_appearance(&self, scheme: Option<Scheme>) {
        let mtm = self.0.borrow().mtm;
        let appearance = scheme.and_then(appearance_for);
        NSApplication::sharedApplication(mtm).setAppearance(appearance.as_deref());
    }

    /// Reports collapses the user made since the last check.
    fn check_collapses(&self) {
        let Ok(state) = self.0.try_borrow() else { return };
        let mut fired = false;
        for (node, native) in &state.nodes {
            if let Native::Pane(p) = native
                && let Some(event) = p.take_collapse()
            {
                state.events.emitter(*node).emit(event);
                fired = true;
            }
        }
        if fired {
            state.wake.wake();
        }
    }
}

impl State {
    fn window(&self) -> &WindowRecord {
        let id = self.open.expect("seam ops arrive inside begin/commit");
        self.windows.get(&id).unwrap_or_else(|| panic!("window {id:?} has no host"))
    }

    fn native(&self, node: NodeId) -> &Native {
        self.nodes.get(&node).unwrap_or_else(|| panic!("{node:?} is not a live node"))
    }

    fn view(&self, node: NodeId) -> Option<&NSView> {
        self.native(node).view()
    }

    fn apply_window_appearance(&self, window: WindowId, theme: &NativeTheme) {
        if let Some(record) = self.windows.get(&window) {
            let appearance = theme.appearance.and_then(appearance_for);
            record.window.setAppearance(appearance.as_deref());
        }
    }

    /// Asks the window to lay out again and the backend to check pane collapses: what a split
    /// view resizing a pane needs.
    fn pane_hook(&self) -> crate::scaffold::Hook {
        let root = objc2::rc::Weak::from_retained(&self.window().root);
        let weak = self.weak.clone();
        // The split view resizes panes inside the window's own layout pass, which would drop a
        // needs-layout set now; the next turn lays the panes' content out at their new size.
        Rc::new(move || {
            let root = root.clone();
            let weak = weak.clone();
            crate::events::defer(move || {
                if let Some(root) = root.load() {
                    root.setNeedsLayout(true);
                }
                if let Some(state) = weak.upgrade() {
                    AppKitBackend(state).check_collapses();
                }
            });
        })
    }

    /// The list a node belongs to as a row (`parent` is the list) or a table cell (`parent` is a
    /// row of the list).
    fn list_of(&self, parent: NodeId) -> Option<NodeId> {
        match self.nodes.get(&parent)? {
            Native::List(_) => Some(parent),
            _ => {
                let list = *self.parents.get(&parent)?;
                matches!(self.nodes.get(&list), Some(Native::List(l)) if l.is_row(parent)).then_some(list)
            }
        }
    }

    /// Applies each list's changes in one table update; a list whose rows must lay out at a new
    /// width asks the window for another layout pass on the next turn.
    fn commit_lists(&mut self, window: WindowId) {
        let lists: Vec<NodeId> = self
            .nodes
            .iter()
            .filter(|(id, n)| matches!(n, Native::List(_)) && self.node_window.get(id) == Some(&window))
            .map(|(id, _)| *id)
            .collect();
        let mut relayout = false;
        for id in lists {
            if let Some(Native::List(l)) = self.nodes.get_mut(&id) {
                relayout |= l.commit();
            }
        }
        if relayout && let Some(record) = self.windows.get(&window) {
            let root = objc2::rc::Weak::from_retained(&record.root);
            crate::events::defer(move || {
                if let Some(root) = root.load() {
                    root.setNeedsLayout(true);
                }
            });
        }
    }

    /// Rows and cells of a native list go into the containers the table shows, not under the
    /// list's view. Returns false for any other node.
    fn attach_to_list(&mut self, parent: NodeId, node: NodeId) -> bool {
        let Some(list) = self.list_of(parent) else { return false };
        let Some(view) = self.view(node).map(|v| v.retain()) else { return false };
        let row = self.row_tags.get(&node).cloned();
        let cell = self.cell_tags.get(&node).copied();
        let Some(Native::List(l)) = self.nodes.get_mut(&list) else { return false };
        if list == parent {
            if let Some(row) = row {
                l.attach_row(node, row.row, &view);
                l.set_row_actions(row.row, row.actions);
                return true;
            }
            return false;
        }
        match cell {
            Some(column) if l.is_table() => {
                l.attach_cell(parent, node, column, &view);
                true
            }
            _ => false,
        }
    }

    fn detach_from_list(&mut self, node: NodeId, parent: Option<NodeId>) -> bool {
        let Some(list) = parent.and_then(|p| self.list_of(p)) else { return false };
        let Some(Native::List(l)) = self.nodes.get_mut(&list) else { return false };
        if l.is_row(node) {
            l.detach_row(node);
            true
        } else if l.is_cell(node) {
            l.detach_cell(node);
            true
        } else {
            false
        }
    }

    /// `node` and every node under it now live in window `to`: nd moved a view between windows
    /// and kept its native nodes, so their views move rather than being recreated.
    fn move_to_window(&mut self, node: NodeId, from: WindowId, to: WindowId) {
        let under = |mut n: NodeId| loop {
            if n == node {
                return true;
            }
            match self.parents.get(&n) {
                Some(p) => n = *p,
                None => return false,
            }
        };
        let moved: Vec<NodeId> =
            self.node_window.iter().filter(|(n, w)| **w == from && under(**n)).map(|(n, _)| *n).collect();
        for n in &moved {
            self.node_window.insert(*n, to);
        }
        self.theme_dirty.insert(from);
        self.theme_dirty.insert(to);
        let Some(owner) = self.windows.get(&to).map(|r| r.window.clone()) else { return };
        let old_owner = self.windows.get(&from).map(|r| r.window.clone());
        for n in moved {
            match self.nodes.get_mut(&n) {
                Some(Native::WindowControls { window, .. }) => *window = to,
                Some(Native::Toolbar(toolbar)) if n != node => {
                    if let Some(old) = &old_owner {
                        old.setToolbar(None);
                    }
                    toolbar.attach_to(&owner);
                    toolbar.dirty.set(true);
                }
                _ => {}
            }
            // The moved node's own overlay is attached by the caller; nested ones follow here.
            if n != node
                && let Some(Native::Overlay(overlay)) = self.nodes.get(&n)
            {
                let anchor = self
                    .parents
                    .get(&n)
                    .and_then(|p| self.nodes.get(p))
                    .and_then(|p| p.view().or(p.container()))
                    .map(|v| v.retain());
                overlay.detach();
                overlay.attach(anchor, owner.clone());
            }
        }
    }

    fn attach(&mut self, parent: NodeId, node: NodeId, before: Option<NodeId>) {
        self.parents.insert(node, parent);
        if let (Some(from), Some(to)) = (self.node_window.get(&node).copied(), self.node_window.get(&parent).copied())
            && from != to
        {
            self.move_to_window(node, from, to);
        }
        if !self.node_window.contains_key(&node)
            && let Some(window) = self.node_window.get(&parent).copied()
        {
            self.node_window.insert(node, window);
        }
        if let Some(window) = self.node_window.get(&node).copied() {
            self.theme_dirty.insert(window);
        }
        if let Some(Native::Overlay(overlay)) = self.nodes.get(&node) {
            let anchor = self.nodes.get(&parent).and_then(|p| p.view().or(p.container())).map(|v| v.retain());
            let owner = self.node_window.get(&node).and_then(|w| self.windows.get(w)).map(|r| r.window.clone());
            if let Some(owner) = owner {
                overlay.attach(anchor, owner);
            }
            return;
        }
        if self.attach_to_list(parent, node) {
            return;
        }
        let hook = matches!(self.nodes.get(&node), Some(Native::Pane(_))).then(|| self.pane_hook());
        match self.nodes.get_disjoint_mut([&parent, &node]) {
            [Some(Native::Scaffold(scaffold)), Some(Native::Pane(pane))] => {
                scaffold.insert(node, pane, hook.expect("pane hook"));
                return;
            }
            // A toolbar anywhere in the tree is the window's toolbar.
            [Some(_), Some(Native::Toolbar(toolbar))] => {
                toolbar.attach_to(&self.windows[&self.open.expect("in a transaction")].window);
                toolbar.dirty.set(true);
                return;
            }
            [Some(Native::Toolbar(toolbar)), Some(child)] => {
                if let Native::ToolbarTab(tab) = child {
                    tab.report_to(self.events.emitter(parent), self.wake.clone());
                }
                toolbar.children.retain(|n| *n != node);
                let at = before.and_then(|b| toolbar.children.iter().position(|n| *n == b));
                toolbar.children.insert(at.unwrap_or(toolbar.children.len()), node);
                toolbar.dirty.set(true);
                return;
            }
            [Some(Native::Form(form)), Some(Native::FormSection(..))] => {
                insert_before(&mut form.sections, node, before);
                self.forms_dirty.insert(parent);
                return;
            }
            [Some(Native::FormSection(_, rows)), Some(Native::FormRow(..))] => {
                insert_before(rows, node, before);
                self.mark_form(parent);
                return;
            }
            [Some(Native::FormRow(_, child)), Some(_)] => {
                *child = Some(node);
                self.mark_form(parent);
                return;
            }
            [Some(Native::ToolbarItem(item)), Some(child)] => {
                match child {
                    Native::WindowControls { .. } => item.skip.set(true),
                    Native::Button(button) if button.is_icon_only() => {
                        let props = button.props();
                        let (target, action) = button.action();
                        item.item.setView(None);
                        item.item.setImage(props.icon.as_deref().and_then(crate::icons::symbol).as_deref());
                        let label = item.props.label.clone().or(props.tooltip.clone()).unwrap_or_default();
                        item.item.setLabel(&objc2_foundation::NSString::from_str(&label));
                        if item.props.tooltip.is_none() {
                            item.item.setToolTip(
                                props.tooltip.as_deref().map(objc2_foundation::NSString::from_str).as_deref(),
                            );
                        }
                        unsafe {
                            item.item.setTarget(Some(target));
                            item.item.setAction(Some(action));
                        }
                        item.item.setBordered(true);
                    }
                    other => {
                        if let Some(view) = other.view() {
                            item.show_view(view);
                        }
                        if let (Native::Button(b), None) = (other, &item.props.label) {
                            item.item.setLabel(&objc2_foundation::NSString::from_str(&b.props().label));
                        }
                    }
                }
                if let Some(toolbar) = self.parents.get(&parent).and_then(|t| self.nodes.get(t))
                    && let Native::Toolbar(t) = toolbar
                {
                    t.dirty.set(true);
                }
                return;
            }
            _ => {}
        }
        let scroll_doc = self.scrolls.get(&parent).map(|s| Retained::into_super(s.document.clone()));
        let (Some(container), Some(view)) =
            (scroll_doc.as_deref().or_else(|| self.native(parent).container()), self.view(node))
        else {
            return;
        };
        match before.and_then(|b| self.view(b)) {
            Some(sibling) if unsafe { sibling.superview() }.is_some_and(|s| std::ptr::eq(&*s, container)) => {
                container.addSubview_positioned_relativeTo(view, NSWindowOrderingMode::Below, Some(sibling))
            }
            _ => container.addSubview(view),
        }
    }

    fn detach(&mut self, node: NodeId) {
        let parent = self.parents.get(&node).copied();
        if self.detach_from_list(node, parent) {
            self.parents.remove(&node);
            return;
        }
        let parent = self.parents.remove(&node);
        if let Some(Native::Overlay(overlay)) = self.nodes.get(&node) {
            overlay.detach();
            return;
        }
        match parent.map(|p| self.nodes.get_disjoint_mut([&p, &node])) {
            Some([Some(Native::Scaffold(scaffold)), Some(Native::Pane(pane))]) => scaffold.remove(node, pane),
            Some([Some(_), Some(Native::Toolbar(_))]) => {
                if let Some(record) = self.node_window.get(&node).and_then(|w| self.windows.get(w)) {
                    record.window.setToolbar(None);
                }
            }
            Some([Some(Native::Toolbar(toolbar)), Some(_)]) => {
                toolbar.children.retain(|n| *n != node);
                toolbar.dirty.set(true);
            }
            Some([Some(Native::Form(form)), Some(_)]) => {
                form.sections.retain(|n| *n != node);
                self.forms_dirty.insert(parent.expect("parent"));
            }
            Some([Some(Native::FormSection(_, rows)), Some(_)]) => {
                rows.retain(|n| *n != node);
                self.mark_form(parent.expect("parent"));
            }
            Some([Some(Native::FormRow(_, child)), Some(_)]) => {
                *child = None;
                self.mark_form(parent.expect("parent"));
                if let Some(view) = self.nodes.get(&node).and_then(Native::view) {
                    view.removeFromSuperview();
                }
            }
            _ => {
                if let Some(view) = self.nodes.get(&node).and_then(Native::view) {
                    view.removeFromSuperview();
                }
            }
        }
    }

    /// Marks the form `node` belongs to (a section or row of it, or the form) for a rebuild.
    fn mark_form(&mut self, node: NodeId) {
        let mut at = Some(node);
        while let Some(n) = at {
            if matches!(self.nodes.get(&n), Some(Native::Form(_))) {
                self.forms_dirty.insert(n);
                return;
            }
            at = self.parents.get(&n).copied();
        }
    }

    /// Hands every changed form its sections, rows and controls.
    fn rebuild_forms(&mut self) {
        let mtm = self.mtm;
        let dirty: Vec<NodeId> = self.forms_dirty.drain().collect();
        for form in dirty {
            let Some(Native::Form(f)) = self.nodes.get(&form) else { continue };
            let sections: Vec<SectionSpec> = f
                .sections
                .iter()
                .filter_map(|s| match self.nodes.get(s)? {
                    Native::FormSection(props, rows) => Some(SectionSpec {
                        node: *s,
                        props: props.clone(),
                        rows: rows
                            .iter()
                            .filter_map(|r| match self.nodes.get(r)? {
                                Native::FormRow(props, child) => Some(RowSpec {
                                    node: *r,
                                    props: props.clone(),
                                    view: child.and_then(|c| self.nodes.get(&c)?.view().map(|v| v.retain())),
                                    size: child.and_then(|c| self.form_sizes.get(&c).copied()).unwrap_or_default(),
                                }),
                                _ => None,
                            })
                            .collect(),
                    }),
                    _ => None,
                })
                .collect();
            if let Some(Native::Form(f)) = self.nodes.get_mut(&form) {
                f.apply(mtm, &sections);
            }
        }
    }

    fn place_lights(&self, window: WindowId, p: &WindowControlsProps) {
        if p.embedded {
            return;
        }
        if let Some(record) = self.windows.get(&window) {
            record.lights.set_at(&record.window, p.at);
        }
    }

    /// Moves embedded traffic lights onto their slots, now that this transaction set frames.
    fn place_embedded_lights(&self, window: WindowId) {
        let Some(record) = self.windows.get(&window) else { return };
        for (node, native) in &self.nodes {
            let Native::WindowControls { window: w, slot, props } = native else { continue };
            if *w != window || !props.embedded || self.in_toolbar(*node) || slot.window().is_none() {
                continue;
            }
            let rect = slot.convertRect_toView(slot.bounds(), None);
            let height = record.window.frame().size.height;
            let at = nd_core::Point { x: rect.origin.x as f32, y: (height - rect.origin.y - rect.size.height) as f32 };
            record.lights.set_at(&record.window, Some(at));
        }
    }

    fn in_toolbar(&self, node: NodeId) -> bool {
        self.parents.get(&node).is_some_and(|p| matches!(self.nodes.get(p), Some(Native::ToolbarItem(_))))
    }

    fn rebuild_toolbars(&self) {
        for (node, native) in &self.nodes {
            let Native::Toolbar(toolbar) = native else { continue };
            if !toolbar.dirty.get() {
                continue;
            }
            let entries = toolbar
                .children
                .iter()
                .filter_map(|child| match self.nodes.get(child)? {
                    Native::ToolbarItem(item) if !item.skip.get() => {
                        Some(Entry::Item(item.item.clone(), item.props.leading))
                    }
                    Native::ToolbarItem(_) => None,
                    Native::SystemItem => self.system_items.get(child).map(|p| Entry::System(p.item)),
                    Native::ToolbarTab(tab) => Some(Entry::Tab(tab.item.clone())),
                    _ => None,
                })
                .collect();
            let scaffold = self.parents.get(node).and_then(|p| match self.nodes.get(p) {
                Some(Native::Scaffold(s)) => Some(s),
                _ => None,
            });
            let split = scaffold.map(|s| s.controller.splitView());
            let sidebar = split.as_deref().filter(|_| scaffold.is_some_and(|s| s.has_sidebar()));
            let inspector = split.as_deref().zip(scaffold.and_then(|s| s.inspector_divider()));
            toolbar.rebuild(entries, sidebar, inspector);
        }
    }

    /// The theme native controls see at `node`: the window's, refined by every scope above.
    fn effective(&self, node: NodeId, memo: &mut HashMap<NodeId, NativeTheme>) -> NativeTheme {
        if let Some(t) = memo.get(&node) {
            return t.clone();
        }
        let base = match self.parents.get(&node) {
            Some(parent) => self.effective(*parent, memo),
            None => self
                .node_window
                .get(&node)
                .and_then(|w| self.window_native.get(w))
                .unwrap_or(&self.default_native)
                .clone(),
        };
        let theme = match self.scopes.get(&node).and_then(|s| s.theme.as_ref()) {
            Some(refinement) => base.refine(refinement, self.system_scheme),
            None => base,
        };
        memo.insert(node, theme.clone());
        theme
    }

    fn apply_theme(&mut self, window: WindowId) {
        let mut memo = HashMap::new();
        let nodes: Vec<NodeId> = self.node_window.iter().filter(|(_, w)| **w == window).map(|(n, _)| *n).collect();
        for node in nodes {
            let theme = self.effective(node, &mut memo);
            let forced: Option<&ThemeRefinement> = self.scopes.get(&node).and_then(|s| s.theme.as_ref());
            let appearance = forced.and_then(|r| r.appearance).map(|_| theme.appearance.and_then(appearance_for));
            let outer = match self.parents.get(&node) {
                Some(parent) => self.effective(*parent, &mut memo),
                None => theme.clone(),
            };
            let flips =
                theme.appearance.unwrap_or(self.system_scheme) != outer.appearance.unwrap_or(self.system_scheme);
            match self.nodes.get_mut(&node) {
                Some(Native::Layer(v) | Native::Root(v) | Native::Titlebar(v)) => {
                    v.set_radius_scale(theme.radius_scale);
                    if let Some(a) = appearance {
                        v.setAppearance(a.as_deref());
                    }
                    v.set_surface(flips.then_some(Color::WindowBackground));
                }
                Some(Native::Pane(p)) => p.content.set_radius_scale(theme.radius_scale),
                Some(Native::SplitPane(p)) => p.content.set_radius_scale(theme.radius_scale),
                Some(Native::Glass(g)) => {
                    g.set_radius_scale(theme.radius_scale);
                    g.content.set_radius_scale(theme.radius_scale);
                    if let Some(a) = appearance {
                        g.view().setAppearance(a.as_deref());
                    }
                }
                Some(Native::Button(b)) => b.set_accent(theme.accent),
                Some(Native::Control(c)) => {
                    c.control.set_accent(theme.accent);
                    c.control.set_font_family(theme.font.as_deref());
                    c.control.appearance_changed();
                }
                Some(Native::Text(v)) => v.set_family(theme.font.as_deref()),
                _ => {}
            }
        }
    }

    fn apply_dirty_themes(&mut self) {
        let windows: Vec<WindowId> = self.theme_dirty.drain().collect();
        for window in &windows {
            self.apply_theme(*window);
        }
        for window in windows {
            self.retheme_custom(Some(window));
            self.retheme_text(window);
        }
    }

    /// What a text node draws. Rich text resolves its colour tokens under the node's theme,
    /// which plain text never needs.
    fn text_look(&mut self, node: NodeId, window: WindowId, p: &Props) -> TextLook {
        let text: &nd_ui::TextProps = props(p, "Text");
        let theme = text.rich.is_some().then(|| self.theme_of(node, window));
        match theme {
            Some(_) => self.rich_text.insert(node, p.clone()),
            None => self.rich_text.remove(&node),
        };
        TextLook::from_props(text, theme.as_ref())
    }

    /// Re-resolves the colour tokens of every rich text node in `window`, after its theme
    /// changed.
    fn retheme_text(&mut self, window: WindowId) {
        let nodes: Vec<(NodeId, Props)> = self
            .rich_text
            .iter()
            .filter(|(n, _)| self.node_window.get(n) == Some(&window))
            .map(|(n, p)| (*n, p.clone()))
            .collect();
        for (node, p) in nodes {
            let look = self.text_look(node, window, &p);
            if let Some(Native::Text(view)) = self.nodes.get(&node) {
                view.set_props(&look);
            }
        }
    }

    /// The theme in effect at `node`: its window's, under every subtree scope above it.
    fn theme_of(&self, node: NodeId, window: WindowId) -> Theme {
        let fallback;
        let registry = match &self.registry {
            Some(r) => r,
            None => {
                fallback = ThemeRegistry::default();
                &fallback
            }
        };
        let mut chain = Vec::new();
        let mut at = Some(node);
        while let Some(id) = at {
            if let Some(refinement) = self.scopes.get(&id).and_then(|s| s.theme.clone()) {
                chain.push(refinement);
            }
            at = self.parents.get(&id).copied();
        }
        let mut composed: Option<ThemeRefinement> = None;
        for refinement in chain.into_iter().rev() {
            composed = Some(match composed {
                Some(outer) => refinement.over(&outer),
                None => refinement,
            });
        }
        match composed {
            Some(r) => registry.resolve_subtree(Some(window), &r),
            None => registry.window_theme(window).clone(),
        }
    }

    /// Hands custom widgets in `window` (every window for `None`) the theme at their node.
    fn retheme_custom(&mut self, window: Option<WindowId>) {
        let nodes: Vec<(NodeId, WindowId)> = self
            .nodes
            .iter()
            .filter(|(_, n)| matches!(n, Native::Custom(_)))
            .filter_map(|(id, _)| Some((*id, *self.node_window.get(id)?)))
            .filter(|(_, w)| window.is_none_or(|x| x == *w))
            .collect();
        for (node, window) in nodes {
            let theme = self.theme_of(node, window);
            if let Some(Native::Custom(c)) = self.nodes.get_mut(&node) {
                c.set_theme(&theme);
            }
        }
    }

    fn set_scope(&mut self, node: NodeId, scope: Option<SubtreeScope>) {
        let menu = scope.as_ref().is_some_and(|s| s.context_menu);
        let changed = self.scopes.get(&node) != scope.as_ref();
        match scope {
            Some(s) => self.scopes.insert(node, s),
            None => self.scopes.remove(&node),
        };
        if changed && let Some(window) = self.node_window.get(&node) {
            self.theme_dirty.insert(*window);
        }
        let emit = menu.then(|| (self.events.emitter(node), self.wake.clone()));
        match self.nodes.get(&node) {
            Some(Native::Layer(v)) => v.set_menu(emit),
            Some(Native::Glass(g)) => g.content.set_menu(emit),
            _ => {}
        }
    }
}

impl Backend for AppKitBackend {
    fn set_event_queue(&mut self, events: EventQueue) {
        self.0.borrow_mut().events = events;
    }

    fn begin(&mut self, window: WindowId) {
        let mut state = self.0.borrow_mut();
        assert!(state.open.is_none(), "transactions do not nest");
        state.open = Some(window);
        CATransaction::begin();
        CATransaction::setDisableActions(true);
    }

    fn create(&mut self, node: NodeId, kind: NodeKind, p: &Props) {
        let mut state = self.0.borrow_mut();
        let mtm = state.mtm;
        let window = state.open.expect("inside a transaction");
        let text_look = (kind == NodeKind::Text).then(|| state.text_look(node, window, p));
        let emit = || state.events.emitter(node);
        let native = match kind {
            NodeKind::Root => {
                let _: &RootProps = props(p, "Root");
                Native::Root(state.window().root.content().retain())
            }
            NodeKind::View => {
                let div = props::<DivProps>(p, "View");
                let view = NdLayerView::new(mtm, &ViewLook::of_div(div, state.expose));
                view.set_gestures(gesture_host(div, emit(), &state.wake));
                view.set_hover_target(div.tracks_hover.then(|| (emit(), state.wake.clone())));
                if div.gestures.scroll.any() {
                    state.scrolls.insert(node, crate::scroll_view::ScrollBox::new(mtm, &view, div.gestures.scroll));
                }
                Native::Layer(view)
            }
            NodeKind::Text => {
                let look = text_look.expect("a text node's look");
                let view = NdTextView::new(mtm, &look);
                view.set_link_target(look.has_links().then(|| (emit(), state.wake.clone())));
                Native::Text(view)
            }
            NodeKind::Widget(w) if is::<Button>(w) => {
                Native::Button(NativeButtonView::new(mtm, emit(), state.wake.clone(), props(p, "Button")))
            }
            NodeKind::Widget(w) if is::<WindowControls>(w) => {
                let wc: &WindowControlsProps = props(p, "WindowControls");
                state.place_lights(window, wc);
                Native::WindowControls { window, slot: NdLayerView::new(mtm, &ViewLook::default()), props: wc.clone() }
            }
            NodeKind::Widget(w) if is::<Glass>(w) => Native::Glass(GlassNative::new(mtm, props(p, "Glass"))),
            NodeKind::Widget(w) if is::<GlassGroup>(w) => {
                Native::GlassGroup(GlassGroupNative::new(mtm, props::<GlassGroupProps>(p, "GlassGroup").spacing))
            }
            NodeKind::Widget(w) if is::<Scaffold>(w) => {
                let _: &ScaffoldProps = props(p, "Scaffold");
                let record = state.window();
                record.root.set_full_size(true);
                let scaffold = ScaffoldNative::new(mtm);
                // Swapping the window's content view inside its own layout pass leaves the new
                // one at a stale size; the next turn is outside it.
                let (window, root, controller, host) =
                    (record.window.clone(), record.root.clone(), scaffold.controller.clone(), scaffold.host.clone());
                crate::events::defer(move || crate::scaffold::install(&controller, &host, &window, &root));
                Native::Scaffold(scaffold)
            }
            NodeKind::Widget(w) if is::<Pane>(w) => {
                Native::Pane(PaneNative::new(mtm, props::<PaneProps>(p, "Pane"), state.expose))
            }
            NodeKind::Widget(w) if is::<Toolbar>(w) => {
                let id = format!("nd.toolbar.{}", window.0);
                Native::Toolbar(ToolbarNative::new(mtm, props::<ToolbarProps>(p, "Toolbar"), &id))
            }
            NodeKind::Widget(w) if is::<ToolbarItem>(w) => {
                let id = format!("nd.item.{}", node.0);
                Native::ToolbarItem(ToolbarItemNative::new(mtm, &id, props::<ToolbarItemProps>(p, "ToolbarItem")))
            }
            NodeKind::Widget(w) if is::<ToolbarSystemItem>(w) => {
                state.system_items.insert(node, *props::<SystemItemProps>(p, "ToolbarSystemItem"));
                Native::SystemItem
            }
            NodeKind::Widget(w) if is::<ToolbarTab>(w) => {
                Native::ToolbarTab(ToolbarTabNative::new(mtm, props::<ToolbarTabProps>(p, "ToolbarTab")))
            }
            NodeKind::Widget(w) if is::<Titlebar>(w) => {
                let look = ViewLook::from_style(&props::<TitlebarProps>(p, "Titlebar").style, false);
                let view = NdLayerView::new(mtm, &look);
                view.set_drag_region(true);
                state.window().root.set_full_size(true);
                Native::Titlebar(view)
            }
            NodeKind::Widget(w) if is::<TitlebarInset>(w) => {
                Native::Inset(window, *props::<TitlebarInsetProps>(p, "TitlebarInset"))
            }
            NodeKind::Widget(w)
                if let Some(control) =
                    crate::forms::create(w, p, crate::forms::Ctx { mtm, emit: emit(), wake: state.wake.clone() }) =>
            {
                Native::Control(ControlNode { control, kind: w, props: p.clone() })
            }
            NodeKind::Widget(w) if is::<SeparatorKind>(w) => {
                Native::Separator(crate::controls::separator(mtm), props::<SeparatorProps>(p, "Separator").vertical)
            }
            NodeKind::Widget(w) if is::<SourceTreeKind>(w) => Native::SourceTree(SourceTreeNative::new(
                mtm,
                emit(),
                state.wake.clone(),
                props::<SourceTreeProps>(p, "SourceTree"),
            )),
            NodeKind::Widget(w) if is::<CommandPaletteKind>(w) => Native::Palette(PaletteNative::new(
                mtm,
                emit(),
                state.wake.clone(),
                props::<CommandPaletteProps>(p, "CommandPalette"),
            )),
            NodeKind::Widget(w) if is::<ToastOverlayKind>(w) => Native::Toasts(ToastNative::new(
                mtm,
                emit(),
                state.wake.clone(),
                props::<ToastOverlayProps>(p, "ToastOverlay"),
            )),
            NodeKind::Widget(w) if is::<BannerKind>(w) => {
                Native::Banner(BannerNative::new(mtm, emit(), state.wake.clone(), props::<BannerProps>(p, "Banner")))
            }
            NodeKind::Widget(w) if is::<nd_virtual::ListWidget>(w) => Native::List(ListNative::new(
                mtm,
                emit(),
                state.wake.clone(),
                props::<nd_virtual::ListProps>(p, "List"),
            )),
            NodeKind::Widget(w) if is::<Form>(w) => Native::Form(FormNative::new(mtm)),
            NodeKind::Widget(w) if is::<Overlay>(w) => Native::Overlay(OverlayNative::new(
                mtm,
                node,
                props::<OverlayProps>(p, "Overlay"),
                emit(),
                state.wake.clone(),
            )),
            NodeKind::Widget(w) if is::<NativeSurface>(w) => {
                Native::Surface(SurfaceNative::new(mtm, props::<SurfaceProps>(p, "NativeSurface")))
            }
            NodeKind::Widget(w) if is::<SplitKind>(w) => Native::Split(SplitNative::new(
                mtm,
                props::<SplitProps>(p, "Split"),
                emit(),
                state.wake.clone(),
                state.pane_hook(),
            )),
            NodeKind::Widget(w) if is::<SplitPaneKind>(w) => Native::SplitPane(SplitPaneNative::new(
                mtm,
                props::<SplitPaneProps>(p, "SplitPane"),
                state.expose,
                state.pane_hook(),
            )),
            NodeKind::Widget(w) if is::<StatusPageKind>(w) => {
                Native::StatusPage(StatusPageNative::new(mtm, props::<StatusPageProps>(p, "StatusPage")))
            }
            NodeKind::Widget(w) if is::<FormSection>(w) => {
                Native::FormSection(props::<FormSectionProps>(p, "FormSection").clone(), Vec::new())
            }
            NodeKind::Widget(w) if is::<FormRow>(w) => {
                Native::FormRow(props::<FormRowProps>(p, "FormRow").clone(), None)
            }
            NodeKind::Widget(w) if let Some(custom) = crate::custom::lookup(w.type_id) => {
                let cx = crate::custom::CustomCx {
                    mtm,
                    node,
                    window,
                    props: p,
                    emit: emit(),
                    wake: crate::custom::Waker(state.wake.clone()),
                    theme: state.theme_of(node, window),
                };
                Native::Custom(custom.create(&cx))
            }
            NodeKind::Canvas | NodeKind::Image | NodeKind::Widget(_) => {
                panic!("the AppKit backend has no {kind:?} yet")
            }
        };
        if let Some(scope) = p.downcast_ref::<DivProps>().and_then(|d| d.scope.as_ref()) {
            if let Some(tag) = scope.get::<nd_virtual::RowTag>() {
                state.row_tags.insert(node, tag.clone());
            }
            if let Some(cell) = scope.get::<nd_virtual::CellTag>() {
                state.cell_tags.insert(node, cell.0);
            }
        }
        state.nodes.insert(node, native);
        state.node_window.insert(node, window);
        let scope = scope_of(p);
        if scope.is_some() {
            state.set_scope(node, scope);
        }
        state.theme_dirty.insert(window);
    }

    fn insert(&mut self, parent: NodeId, node: NodeId, before: Option<NodeId>) {
        self.0.borrow_mut().attach(parent, node, before);
    }

    fn move_node(&mut self, node: NodeId, parent: NodeId, before: Option<NodeId>) {
        // addSubview re-parents in place: the view keeps its layer, state and first responder.
        let mut state = self.0.borrow_mut();
        if state.parents.get(&node) != Some(&parent) {
            state.detach(node);
        }
        state.attach(parent, node, before);
    }

    fn remove(&mut self, node: NodeId) {
        self.0.borrow_mut().detach(node);
    }

    fn update(&mut self, node: NodeId, patch: &PropsPatch) {
        let mut state = self.0.borrow_mut();
        let events = state.events.clone();
        let wake = state.wake.clone();
        let expose = state.expose;
        if let Some(scope) = scope_of(&patch.new) {
            state.set_scope(node, Some(scope));
        } else if state.scopes.contains_key(&node) {
            state.set_scope(node, None);
        }
        let window = state.node_window.get(&node).copied();
        let mut place: Option<(WindowId, WindowControlsProps)> = None;
        let mut recreate_control = false;
        let mut recreate_surface = None;
        let mut form_changed = false;
        let mut scroll_axes = None;
        let custom_cx = match (state.nodes.get(&node), window) {
            (Some(Native::Custom(_)), Some(window)) => Some(crate::custom::CustomCx {
                mtm: state.mtm,
                node,
                window,
                props: &patch.new,
                emit: events.emitter(node),
                wake: crate::custom::Waker(wake.clone()),
                theme: state.theme_of(node, window),
            }),
            _ => None,
        };
        let is_text = matches!(state.nodes.get(&node), Some(Native::Text(_)));
        let text_look = match window {
            Some(window) if is_text => Some(state.text_look(node, window, &patch.new)),
            _ => None,
        };
        events.programmatic(|| match state.nodes.get_mut(&node).unwrap_or_else(|| panic!("{node:?} is not live")) {
            Native::Custom(c) => {
                if let Some(cx) = &custom_cx {
                    c.update(cx);
                }
            }
            Native::Layer(v) => {
                let div = props::<DivProps>(&patch.new, "View");
                v.set_props(&ViewLook::of_div(div, expose));
                if props::<DivProps>(&patch.old, "View").tracks_hover != div.tracks_hover {
                    v.set_hover_target(div.tracks_hover.then(|| (events.emitter(node), wake.clone())));
                }
                if props::<DivProps>(&patch.old, "View").gestures != div.gestures {
                    v.set_gestures(gesture_host(div, events.emitter(node), &wake));
                    scroll_axes = Some(div.gestures.scroll);
                }
            }
            Native::Titlebar(v) => {
                v.set_props(&ViewLook::from_style(&props::<TitlebarProps>(&patch.new, "Titlebar").style, false))
            }
            Native::Text(v) => {
                let look = text_look.clone().expect("a text node's look");
                v.set_link_target(look.has_links().then(|| (events.emitter(node), wake.clone())));
                v.set_props(&look);
            }
            Native::Button(b) => b.update(props::<ButtonProps>(&patch.old, "Button"), props(&patch.new, "Button")),
            Native::WindowControls { window, props: wc, .. } => {
                *wc = props::<WindowControlsProps>(&patch.new, "WindowControls").clone();
                place = Some((*window, wc.clone()));
            }
            Native::Glass(g) => g.update(props(&patch.new, "Glass")),
            Native::GlassGroup(g) => g.set_spacing(props::<GlassGroupProps>(&patch.new, "GlassGroup").spacing),
            Native::Pane(pane) => pane.update(props(&patch.new, "Pane")),
            Native::Toolbar(t) => {
                t.update(props(&patch.new, "Toolbar"));
                t.dirty.set(true);
            }
            Native::ToolbarItem(i) => {
                let p: &ToolbarItemProps = props(&patch.new, "ToolbarItem");
                i.apply(p);
                i.props = p.clone();
            }
            Native::SystemItem => {}
            Native::ToolbarTab(tab) => tab.apply(props(&patch.new, "ToolbarTab")),
            Native::Inset(_, inset) => *inset = *props(&patch.new, "TitlebarInset"),
            Native::Control(c) => {
                recreate_control = !c.control.update(&patch.old, &patch.new);
                c.props = patch.new.clone();
            }
            Native::Separator(_, vertical) => *vertical = props::<SeparatorProps>(&patch.new, "Separator").vertical,
            Native::SourceTree(s) => s.update(props(&patch.new, "SourceTree")),
            Native::Palette(p) => p.update(props(&patch.new, "CommandPalette")),
            Native::Toasts(t) => t.update(props(&patch.new, "ToastOverlay")),
            Native::Banner(b) => b.update(props(&patch.new, "Banner")),
            Native::List(l) => l.update(props(&patch.new, "List")),
            Native::FormSection(p, _) => {
                *p = props::<FormSectionProps>(&patch.new, "FormSection").clone();
                form_changed = true;
            }
            Native::FormRow(p, _) => {
                *p = props::<FormRowProps>(&patch.new, "FormRow").clone();
                form_changed = true;
            }
            Native::Form(_) => {}
            Native::Overlay(o) => o.update(props(&patch.new, "Overlay")),
            Native::Surface(s) => {
                let p: &SurfaceProps = props(&patch.new, "NativeSurface");
                if !s.update(p) {
                    recreate_surface = Some(p.clone());
                }
            }
            Native::Split(s) => s.update(props(&patch.new, "Split")),
            Native::SplitPane(p) => p.update(props(&patch.new, "SplitPane"), expose),
            Native::StatusPage(p) => p.update(props(&patch.new, "StatusPage")),
            Native::Root(_) | Native::External(_) | Native::Scaffold(_) => {}
        });
        if let Some(Native::ToolbarItem(_)) = state.nodes.get(&node)
            && let Some(Native::Toolbar(t)) = state.parents.get(&node).and_then(|p| state.nodes.get(p))
        {
            t.dirty.set(true);
        }
        if form_changed {
            state.mark_form(node);
        }
        let row_tag = patch.new.downcast_ref::<DivProps>().and_then(|d| d.scope.as_ref()?.get::<nd_virtual::RowTag>());
        if let Some(tag) = row_tag {
            let tag = tag.clone();
            state.row_tags.insert(node, tag.clone());
            if let Some(list) = state.parents.get(&node).copied()
                && let Some(Native::List(l)) = state.nodes.get_mut(&list)
                && l.is_row(node)
            {
                l.set_row_actions(tag.row, tag.actions);
            }
        }
        if let (Some(axes), Some(Native::Layer(v))) = (scroll_axes, state.nodes.get(&node)) {
            let v = v.clone();
            match (state.scrolls.get(&node), axes.any()) {
                (Some(s), true) => s.set_axes(axes),
                (Some(_), false) => {
                    if let Some(s) = state.scrolls.remove(&node) {
                        s.dissolve(&v);
                    }
                }
                (None, true) => {
                    let mtm = state.mtm;
                    state.scrolls.insert(node, crate::scroll_view::ScrollBox::adopt(mtm, &v, axes));
                }
                (None, false) => {}
            }
        }
        if let Some((w, wc)) = place {
            state.place_lights(w, &wc);
        }
        if recreate_control && let Some(Native::Control(c)) = state.nodes.get(&node) {
            let (kind, p) = (c.kind, c.props.clone());
            let cx = crate::forms::Ctx { mtm: state.mtm, emit: state.events.emitter(node), wake: state.wake.clone() };
            // Built inside `programmatic`, so the initial value the fresh control takes never echoes.
            let control = events.programmatic(|| crate::forms::create(kind, &p, cx)).expect("a forms kind");
            let old = state.nodes.insert(node, Native::Control(ControlNode { control, kind, props: p }));
            if let (Some(old), Some(parent)) = (old, state.parents.get(&node).copied())
                && let (Some(old_view), Some(container)) = (old.view(), state.native(parent).container())
            {
                let new_view = state.view(node).expect("control view");
                container.addSubview_positioned_relativeTo(new_view, NSWindowOrderingMode::Above, Some(old_view));
                old_view.removeFromSuperview();
            }
            // A form row's slot holds the old view; the form takes the new one when it rebuilds.
            state.mark_form(node);
        }
        if let Some(p) = recreate_surface {
            let fresh = SurfaceNative::new(state.mtm, &p);
            let old = state.nodes.insert(node, Native::Surface(fresh));
            if let (Some(old), Some(parent)) = (old, state.parents.get(&node).copied())
                && let (Some(old_view), Some(container)) = (old.view(), state.native(parent).container())
            {
                let new_view = state.view(node).expect("surface view");
                new_view.setFrame(old_view.frame());
                container.addSubview_positioned_relativeTo(new_view, NSWindowOrderingMode::Above, Some(old_view));
                old_view.removeFromSuperview();
            }
        }
        if let Some(window) = window {
            state.theme_dirty.insert(window);
        }
    }

    fn reaffirm(&mut self, node: NodeId) {
        let mut state = self.0.borrow_mut();
        let events = state.events.clone();
        if let Some(Native::Control(c)) = state.nodes.get_mut(&node) {
            let props = c.props.clone();
            // Old and new are the same props: only controlled values whose native side drifted
            // from them are set again.
            events.programmatic(|| c.control.update(&props, &props));
        }
    }

    fn set_frame(&mut self, node: NodeId, frame: Rect) {
        // The form places its rows' controls, at the size nd laid them out at.
        {
            let mut state = self.0.borrow_mut();
            let in_form =
                state.parents.get(&node).is_some_and(|p| matches!(state.nodes.get(p), Some(Native::FormRow(..))));
            if in_form {
                let size = (f64::from(frame.size.width), f64::from(frame.size.height));
                if state.form_sizes.insert(node, size) != Some(size) {
                    state.mark_form(node);
                }
                return;
            }
        }
        {
            let mut state = self.0.borrow_mut();
            if let Some(Native::List(l)) = state.nodes.get_mut(&node) {
                l.set_frame(frame);
                return;
            }
            let parent = state.parents.get(&node).copied();
            if let Some(list) = parent.and_then(|p| state.list_of(p))
                && let Some(Native::List(l)) = state.nodes.get_mut(&list)
            {
                if l.is_row(node) {
                    l.set_row_frame(node, frame);
                    return;
                }
                if l.is_cell(node) {
                    l.set_cell_frame(node, frame);
                    return;
                }
            }
        }
        let state = self.0.borrow();
        match state.nodes.get(&node) {
            // The split view places a pane's host; nd frames the content inside it. An overlay
            // fills the scaffold by its autoresizing.
            Some(Native::Pane(p)) => {
                if p.role != PaneRole::Overlay {
                    let rect = ns_rect(frame);
                    if p.content.frame() != rect {
                        p.content.setFrame(rect);
                    }
                }
                return;
            }
            Some(Native::SplitPane(p)) => {
                let rect = ns_rect(frame);
                if p.content.frame() != rect {
                    p.content.setFrame(rect);
                }
                return;
            }
            Some(Native::ToolbarItem(item)) => {
                item.host.set_size(objc2_foundation::NSSize::new(frame.size.width.into(), frame.size.height.into()));
                return;
            }
            Some(Native::Overlay(overlay)) => {
                overlay.set_size(frame);
                return;
            }
            _ => {}
        }
        let Some(view) = state.view(node) else { return };
        let rect = ns_rect(frame);
        // Rounded outward on the backing grid, so a label rounded down never loses its last glyph.
        let rect = match unsafe { view.superview() } {
            Some(parent) => parent.backingAlignedRect_options(rect, NSAlignmentOptions::AlignAllEdgesOutward),
            None => rect,
        };
        if view.frame() != rect {
            view.setFrame(rect);
        }
    }

    fn surface(&mut self, _node: NodeId, _op: &SurfaceOp) {}

    fn focus(&mut self, node: Option<NodeId>) {
        let state = self.0.borrow();
        let window = state.window().window.clone();
        let native = node.and_then(|n| state.nodes.get(&n));
        // nd only focuses focusable nodes, so a div it names takes first responder from now on.
        if let Some(Native::Layer(v)) = native {
            v.set_focusable();
        }
        let view = native.and_then(Native::key_view);
        drop(state);
        window.make_first_responder_from_nd(view.as_deref());
    }

    fn release(&mut self, node: NodeId) {
        let mut state = self.0.borrow_mut();
        let native = state.nodes.remove(&node);
        state.parents.remove(&node);
        state.node_window.remove(&node);
        state.scopes.remove(&node);
        state.system_items.remove(&node);
        state.form_sizes.remove(&node);
        state.row_tags.remove(&node);
        state.cell_tags.remove(&node);
        state.scrolls.remove(&node);
        state.rich_text.remove(&node);
        if let Some(Native::Scaffold(_)) = &native
            && let Some(record) = state.open.and_then(|w| state.windows.get(&w))
        {
            crate::scaffold::uninstall(&record.window, &record.root);
            record.root.set_full_size(false);
        }
        drop(state);
        drop(native);
    }

    fn commit(&mut self, window: WindowId) {
        let mut state = self.0.borrow_mut();
        assert_eq!(state.open, Some(window), "commit closes the open transaction");
        state.rebuild_toolbars();
        state.rebuild_forms();
        state.commit_lists(window);
        let customs: Vec<NodeId> = state
            .nodes
            .iter()
            .filter(|(id, n)| matches!(n, Native::Custom(_)) && state.node_window.get(id) == Some(&window))
            .map(|(id, _)| *id)
            .collect();
        for id in customs {
            if let Some(Native::Custom(c)) = state.nodes.get_mut(&id) {
                c.committed();
            }
        }
        for (node, scroll) in &state.scrolls {
            if state.node_window.get(node) == Some(&window) {
                scroll.fit_document();
            }
        }
        state.place_embedded_lights(window);
        state.apply_dirty_themes();
        CATransaction::commit();
        // After the content's frames are committed, so an overlay opens over laid-out content;
        // outer overlays first, since a nested one is anchored inside its parent's window.
        let mut overlays: Vec<NodeId> = state
            .nodes
            .iter()
            .filter(|(n, native)| matches!(native, Native::Overlay(_)) && state.node_window.get(n) == Some(&window))
            .map(|(n, _)| *n)
            .collect();
        overlays.sort();
        for node in overlays {
            if let Some(Native::Overlay(overlay)) = state.nodes.get(&node) {
                overlay.present();
            }
        }
        state.open = None;
    }

    fn measure(&mut self, node: NodeId, constraint: Constraint) -> Size {
        let width = match constraint.width {
            AvailableSpace::Definite(w) => Some(w),
            AvailableSpace::MinContent | AvailableSpace::MaxContent => None,
        };
        let state = self.0.borrow();
        match state.native(node) {
            Native::Text(v) => v.measure(width),
            Native::Button(b) => b.measure(),
            Native::Control(c) => c.control.measure(width),
            Native::Banner(b) => b.measure(),
            Native::Custom(c) => c.measure(width),
            Native::StatusPage(p) => p.measure(width),
            Native::Separator(b, vertical) => {
                let s = b.fittingSize();
                if *vertical {
                    Size { width: s.width.max(1.) as f32, height: 0. }
                } else {
                    Size { width: 0., height: s.height.max(1.) as f32 }
                }
            }
            Native::Inset(window, props) => match state.windows.get(window) {
                Some(record) => crate::titlebar::measure_inset(&record.window, props),
                None => Size::default(),
            },
            Native::WindowControls { window, props, .. } if props.embedded => state
                .windows
                .get(window)
                .and_then(|r| crate::traffic_lights::cluster_size(&r.window))
                .unwrap_or_default(),
            _ => Size::default(),
        }
    }

    fn boundary_size(&self, node: NodeId) -> Option<Size> {
        match self.0.try_borrow().ok()?.nodes.get(&node)? {
            Native::Pane(p) if p.role != PaneRole::Overlay => Some(p.boundary()),
            Native::SplitPane(p) => Some(p.boundary()),
            _ => None,
        }
    }

    fn boundary_width(&self, node: NodeId) -> Option<f32> {
        let state = self.0.try_borrow().ok()?;
        let list = *state.parents.get(&node)?;
        match state.nodes.get(&list)? {
            Native::List(l) => l.boundary_width(node),
            _ => None,
        }
    }

    fn boundary_insets(&self, node: NodeId) -> nd_ui::backend::BoundaryInsets {
        match self.0.try_borrow().ok().as_ref().and_then(|s| s.nodes.get(&node)) {
            Some(Native::Pane(p)) if p.role != PaneRole::Overlay => p.insets(),
            _ => nd_ui::backend::BoundaryInsets::default(),
        }
    }

    fn retains_frames(&self) -> bool {
        true
    }

    fn probe(&self, node: NodeId) -> nd_ui::semantics::Probe {
        self.probe_node(node)
    }

    fn window_state(&self, window: WindowId) -> Option<nd_ui::semantics::WindowState> {
        self.window_state_of(window)
    }

    fn perform(
        &mut self,
        node: NodeId,
        action: &nd_ui::semantics::SemanticAction,
    ) -> Result<(), nd_ui::semantics::ActionError> {
        self.perform_action(node, action)
    }

    fn synthesize(
        &mut self,
        window: WindowId,
        input: &nd_ui::semantics::SyntheticInput,
    ) -> Result<(), nd_ui::semantics::ActionError> {
        self.post(window, input)
    }

    fn window_command(&mut self, window: WindowId, command: nd_ui::backend::WindowCommand) {
        let Some(ns) = self.ns_window(window) else { return };
        match command {
            nd_ui::backend::WindowCommand::Activate => {
                if let Some(group) = ns.tabGroup() {
                    group.setSelectedWindow(Some(&ns));
                }
                ns.makeKeyAndOrderFront(None);
            }
            nd_ui::backend::WindowCommand::ToggleTabOverview => ns.toggleTabOverview(None),
            _ => {}
        }
    }

    fn set_window_title(&mut self, window: WindowId, title: &str) {
        if let Some(ns) = self.ns_window(window) {
            ns.setTitle(&objc2_foundation::NSString::from_str(title));
        }
    }

    fn haptic(&mut self, haptic: nd_gesture::Haptic) {
        crate::gestures::play(haptic);
    }

    fn capture(&mut self, window: WindowId) -> Result<nd_ui::semantics::Capture, nd_ui::semantics::ActionError> {
        self.capture_window(window)
    }

    fn paints(&self, window: WindowId) -> Result<Vec<nd_ui::semantics::Paint>, nd_ui::semantics::ActionError> {
        self.paints_of(window)
    }
}
