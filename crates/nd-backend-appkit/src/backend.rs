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
    Form, FormRow, FormRowProps, FormSection, FormSectionProps, Glass, GlassGroup, GlassGroupProps, GlassProps, Pane,
    PaneProps, PaneRole, Scaffold, ScaffoldProps, SelectKind, SelectProps, SeparatorKind, SeparatorProps, SliderKind,
    SliderProps, SourceListKind, SourceListProps, SubtreeScope, SystemItemProps, Titlebar, TitlebarInset,
    TitlebarInsetProps, TitlebarProps, Toolbar, ToolbarItem, ToolbarItemProps, ToolbarProps, ToolbarSystemItem,
    ToolbarTab, ToolbarTabProps,
};
use nd_core::{NodeId, Rect, Size, WindowId};
use nd_theme::{NativeTheme, Scheme, ThemeRefinement};
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
use crate::controls::{SelectNative, SliderNative};
use crate::events::Wake;
use crate::form::{FormNative, RowSpec, SectionSpec};
use crate::glass::{GlassGroupNative, GlassNative};
use crate::look::{TextLook, ViewLook};
use crate::overlay::OverlayNative;
use crate::scaffold::{PaneNative, ScaffoldNative};
use crate::source_list::SourceListNative;
use crate::surface::SurfaceNative;
use crate::system_colors::appearance_for;
use crate::toolbar::{Entry, ToolbarItemNative, ToolbarNative, ToolbarTabNative};
use crate::traffic_lights::TrafficLights;
use crate::views::{NdLayerView, NdRootView, NdTextView, ns_rect};

mod automation;

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
    Select(SelectNative),
    Slider(SliderNative),
    Separator(Retained<NSBox>, bool),
    SourceList(SourceListNative),
    Form(FormNative),
    /// Logical: the form draws sections and rows; these hold what goes in them.
    FormSection(FormSectionProps, Vec<NodeId>),
    FormRow(FormRowProps, Option<NodeId>),
    /// Its own window above the owner; never a subview of its parent, which is its anchor.
    Overlay(OverlayNative),
    Surface(SurfaceNative),
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
            Native::Select(s) => Some(s.view()),
            Native::Slider(s) => Some(s.view()),
            Native::Separator(b, _) => Some(b),
            Native::SourceList(s) => Some(s.view()),
            Native::Form(f) => Some(&f.outer),
            Native::Surface(s) => Some(s.view()),
            Native::FormSection(..) | Native::FormRow(..) | Native::Overlay(_) => None,
            Native::Toolbar(_) | Native::SystemItem | Native::ToolbarTab(_) | Native::Inset(..) => None,
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

fn insert_before(list: &mut Vec<NodeId>, node: NodeId, before: Option<NodeId>) {
    list.retain(|n| *n != node);
    let at = before.and_then(|b| list.iter().position(|n| *n == b)).unwrap_or(list.len());
    list.insert(at, node);
}

impl AppKitBackend {
    pub fn new(mtm: MainThreadMarker) -> Self {
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

    /// The node whose view is `view`, if nd created or registered it. `None` while a
    /// transaction is applying.
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

    fn attach(&mut self, parent: NodeId, node: NodeId, before: Option<NodeId>) {
        self.parents.insert(node, parent);
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
        let (Some(container), Some(view)) = (self.native(parent).container(), self.view(node)) else { return };
        match before.and_then(|b| self.view(b)) {
            Some(sibling) if unsafe { sibling.superview() }.is_some_and(|s| std::ptr::eq(&*s, container)) => {
                container.addSubview_positioned_relativeTo(view, NSWindowOrderingMode::Below, Some(sibling))
            }
            _ => container.addSubview(view),
        }
    }

    fn detach(&mut self, node: NodeId) {
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
            let split = self.parents.get(node).and_then(|p| match self.nodes.get(p) {
                Some(Native::Scaffold(s)) if s.has_sidebar() => Some(s.controller.splitView()),
                _ => None,
            });
            toolbar.rebuild(entries, split.as_deref());
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
            match self.nodes.get_mut(&node) {
                Some(Native::Layer(v) | Native::Root(v) | Native::Titlebar(v)) => {
                    v.set_radius_scale(theme.radius_scale);
                    if let Some(a) = appearance {
                        v.setAppearance(a.as_deref());
                    }
                }
                Some(Native::Pane(p)) => p.content.set_radius_scale(theme.radius_scale),
                Some(Native::Glass(g)) => {
                    g.set_radius_scale(theme.radius_scale);
                    g.content.set_radius_scale(theme.radius_scale);
                    if let Some(a) = appearance {
                        g.view().setAppearance(a.as_deref());
                    }
                }
                Some(Native::Button(b)) => b.set_accent(theme.accent),
                Some(Native::Select(s)) => s.set_accent(theme.accent),
                Some(Native::Slider(s)) => s.set_accent(theme.accent),
                _ => {}
            }
        }
    }

    fn apply_dirty_themes(&mut self) {
        let windows: Vec<WindowId> = self.theme_dirty.drain().collect();
        for window in windows {
            self.apply_theme(window);
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
        let emit = || state.events.emitter(node);
        let native = match kind {
            NodeKind::Root => {
                let _: &RootProps = props(p, "Root");
                Native::Root(state.window().root.content().retain())
            }
            NodeKind::View => Native::Layer(NdLayerView::new(
                mtm,
                &ViewLook::from_style(&props::<DivProps>(p, "View").style, state.expose),
            )),
            NodeKind::Text => Native::Text(NdTextView::new(mtm, &TextLook::from_props(props(p, "Text")))),
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
            NodeKind::Widget(w) if is::<SelectKind>(w) => {
                Native::Select(SelectNative::new(mtm, emit(), state.wake.clone(), props::<SelectProps>(p, "Select")))
            }
            NodeKind::Widget(w) if is::<SliderKind>(w) => {
                Native::Slider(SliderNative::new(mtm, emit(), state.wake.clone(), props::<SliderProps>(p, "Slider")))
            }
            NodeKind::Widget(w) if is::<SeparatorKind>(w) => {
                Native::Separator(crate::controls::separator(mtm), props::<SeparatorProps>(p, "Separator").vertical)
            }
            NodeKind::Widget(w) if is::<SourceListKind>(w) => Native::SourceList(SourceListNative::new(
                mtm,
                emit(),
                state.wake.clone(),
                props::<SourceListProps>(p, "SourceList"),
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
            NodeKind::Widget(w) if is::<FormSection>(w) => {
                Native::FormSection(props::<FormSectionProps>(p, "FormSection").clone(), Vec::new())
            }
            NodeKind::Widget(w) if is::<FormRow>(w) => {
                Native::FormRow(props::<FormRowProps>(p, "FormRow").clone(), None)
            }
            NodeKind::Canvas | NodeKind::Image | NodeKind::Widget(_) => {
                panic!("the AppKit backend has no {kind:?} yet")
            }
        };
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
        let expose = state.expose;
        if let Some(scope) = scope_of(&patch.new) {
            state.set_scope(node, Some(scope));
        } else if state.scopes.contains_key(&node) {
            state.set_scope(node, None);
        }
        let window = state.node_window.get(&node).copied();
        let mut place: Option<(WindowId, WindowControlsProps)> = None;
        let mut recreate_select = None;
        let mut recreate_surface = None;
        let mut form_changed = false;
        events.programmatic(|| match state.nodes.get_mut(&node).unwrap_or_else(|| panic!("{node:?} is not live")) {
            Native::Layer(v) => {
                v.set_props(&ViewLook::from_style(&props::<DivProps>(&patch.new, "View").style, expose))
            }
            Native::Titlebar(v) => {
                v.set_props(&ViewLook::from_style(&props::<TitlebarProps>(&patch.new, "Titlebar").style, false))
            }
            Native::Text(v) => v.set_props(&TextLook::from_props(props(&patch.new, "Text"))),
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
            Native::Select(s) => {
                let p: &SelectProps = props(&patch.new, "Select");
                if s.can_update(p) {
                    s.update(p);
                } else {
                    recreate_select = Some(p.clone());
                }
            }
            Native::Slider(s) => {
                let p: &SliderProps = props(&patch.new, "Slider");
                s.apply(p);
            }
            Native::Separator(_, vertical) => *vertical = props::<SeparatorProps>(&patch.new, "Separator").vertical,
            Native::SourceList(s) => s.update(props(&patch.new, "SourceList")),
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
        if let Some((w, wc)) = place {
            state.place_lights(w, &wc);
        }
        if let Some(p) = recreate_select {
            let mtm = state.mtm;
            let fresh = SelectNative::new(mtm, state.events.emitter(node), state.wake.clone(), &p);
            let old = state.nodes.insert(node, Native::Select(fresh));
            if let (Some(old), Some(parent)) = (old, state.parents.get(&node).copied())
                && let (Some(old_view), Some(container)) = (old.view(), state.native(parent).container())
            {
                let new_view = state.view(node).expect("select view");
                container.addSubview_positioned_relativeTo(new_view, NSWindowOrderingMode::Above, Some(old_view));
                old_view.removeFromSuperview();
            }
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
        let view = native.and_then(Native::view).map(|v| v.retain());
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
            Native::Select(s) => s.measure(),
            Native::Slider(s) => s.measure(),
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

    fn capture(&mut self, window: WindowId) -> Result<nd_ui::semantics::Capture, nd_ui::semantics::ActionError> {
        self.capture_window(window)
    }
}
