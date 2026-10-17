//! The GPUI side of the backend seam (ARCHITECTURE.md 3.15). Seam ops only edit a retained copy of
//! each window's tree; nothing here touches GPUI. A commit marks the window for a redraw, and the
//! window's GPUI view builds elements from the copy when GPUI draws it ([`crate::render`]). GPUI
//! lays out with its own Taffy, so frames and measures never reach this backend.

use std::any::TypeId;
use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};
use std::rc::Rc;

use nd_core::{NodeId, Point, Rect, Size, WindowId};
use nd_gpui::gpui::{self, AnyWindowHandle, AsyncApp};
use nd_theme::{OsTheme, Preset, Scheme, ThemeRegistry};
use nd_ui::backend::{Backend, Constraint, NodeKind, Props, PropsPatch, SurfaceOp};
use nd_ui::events::EventQueue;
use nd_ui::semantics::{ActionError, Capture, Paint, Probe, SemanticAction, SyntheticInput, WindowState};
use nd_ui::{App, Button, ButtonProps, ClickEvent, InputCounts, SurfaceProps};

#[derive(Clone)]
pub(crate) struct Node {
    pub kind: NodeKind,
    pub props: Props,
    pub parent: Option<NodeId>,
    pub children: Vec<NodeId>,
}

#[derive(Default)]
pub(crate) struct WindowRecord {
    pub root: Option<NodeId>,
    pub handle: Option<AnyWindowHandle>,
    /// The window as of its last frame, for reads that arrive while GPUI is drawing.
    pub state: WindowState,
    /// What a toolbar with no title of its own shows.
    pub title: nd_gpui::gpui::SharedString,
}

/// Drawn controls the user focused, by window, waiting for the host's next pump.
pub(crate) type PlatformFocus = Rc<RefCell<Vec<(WindowId, Option<NodeId>)>>>;

/// Called from a native callback with GPUI's app, to schedule the host's pump on the next turn.
pub(crate) type Wake = Rc<dyn Fn(&mut gpui::App)>;

/// Work for the app at the host's next pump (see [`GpuiBackend::defer`]).
pub(crate) type Deferred = Box<dyn FnOnce(&mut App)>;

/// Told the platform's light or dark setting, with the app, when it changes.
pub(crate) type SchemeListener = Rc<dyn Fn(&mut App, Scheme)>;

/// Told of files dropped onto an app window, at a point in window coordinates.
pub type FileDropListener = Rc<dyn Fn(WindowId, Vec<std::path::PathBuf>, (f32, f32))>;

/// Told when the app gains (`true`) or loses its last active window.
pub type ActivationListener = Rc<dyn Fn(bool)>;

pub(crate) struct State {
    pub nodes: HashMap<NodeId, Node>,
    pub windows: HashMap<WindowId, WindowRecord>,
    open: Option<WindowId>,
    pub events: EventQueue,
    /// Windows whose retained tree changed since GPUI last drew them.
    pub dirty: BTreeSet<WindowId>,
    pub app: Option<AsyncApp>,
    pub wake: Option<Wake>,
    /// Synthetic input waiting for the host's next pump, which dispatches it with the app free.
    pending: Vec<(WindowId, gpui::PlatformInput)>,
    /// Each window's open drawn context menu.
    pub menus: HashMap<WindowId, crate::overlay::MenuOverlay>,
    /// Menu items chosen since the last pump, dispatched from the window's focused node.
    pub actions: Vec<(WindowId, Box<dyn gpui::Action>)>,
    /// Windows whose next mouse release ends a press an overlay backdrop consumed.
    pub swallow_up: std::collections::HashSet<WindowId>,
    /// Every colour the elements paint comes from here: the app's registry once `nd` hands it
    /// over, until then one following the platform's scheme.
    pub theme: ThemeRegistry,
    pub platform_scheme: Option<Scheme>,
    pub scheme_listener: Option<SchemeListener>,
    pub file_drop_listener: Option<FileDropListener>,
    pub activation_listener: Option<ActivationListener>,
    /// What each window painted in its last build.
    pub paints: HashMap<WindowId, Vec<Paint>>,
}

/// The GPUI backend. A cheap handle: `nd-ui`'s `App` owns one clone as its `Box<dyn Backend>`, the
/// host and every window view keep others.
#[derive(Clone)]
pub struct GpuiBackend {
    pub(crate) state: Rc<RefCell<State>>,
    /// Each node's bounds in window coordinates as of the last paint, written by the elements
    /// while GPUI prepaints; kept apart from `state` so a paint never contends with a seam op.
    pub(crate) bounds: Rc<RefCell<HashMap<NodeId, Rect>>>,
    /// What a drawn forms control keeps between frames (a text field's caret and selection, a
    /// slider mid-drag), by node; dropped when the node is released.
    pub(crate) form_drawn: Rc<RefCell<HashMap<NodeId, crate::forms::Drawn>>>,
    /// nd focus moves the drawn controls have yet to follow, by window. The window's next draw
    /// gives the target GPUI focus, or blurs when nd focus left the drawn controls.
    pub(crate) focus_requests: Rc<RefCell<HashMap<WindowId, Option<NodeId>>>>,
    /// Focus the user gave a drawn control (a click), for the host to record in nd.
    pub(crate) platform_focus: PlatformFocus,
    /// Pointer input each native surface received, written by its paint-time listeners.
    pub(crate) surface_counts: Rc<RefCell<HashMap<NodeId, InputCounts>>>,
    /// The app's menus, drawn above the windows of a single canvas.
    pub(crate) menu_bar: Rc<RefCell<crate::canvas::MenuBar>>,
    /// Work [`GpuiBackend::defer`] queued for the next pump.
    deferred: Rc<RefCell<Vec<Deferred>>>,
    /// Presenter state of each `nd-virtual` list, by its node.
    pub(crate) lists: crate::virtual_list::Presenters,
    /// Gesture recognition and scroll physics per window.
    pub(crate) gestures: crate::gestures::Gestures,
    /// Controls' fills fading between rest, hover and pressed.
    pub(crate) fades: crate::fade::Fades,
    /// Each open overlay's subtree as last drawn, and the closed ones still animating out.
    pub(crate) overlay_exits: crate::overlay::Exits,
    /// Per-node state of the drawn navigation widgets ([`crate::nav`]).
    pub(crate) drawn: crate::nav::DrawnMap,
    /// Scaffolds whose sidebar the drawn toolbar toggle hid, as AppKit's split view collapses
    /// it without the app's state changing.
    pub(crate) hidden_sidebars: Rc<RefCell<std::collections::HashSet<NodeId>>>,
    /// Split divider drags in progress, by split node.
    pub(crate) splits: crate::split::SplitDrags,
    /// The drawn alerts and about panels showing ([`crate::dialogs`]).
    pub(crate) dialogs: crate::dialogs::Dialogs,
    /// What the host and the seam share about tabs ([`crate::window_tabs`]).
    pub(crate) tab_seam: Rc<RefCell<crate::window_tabs::TabSeam>>,
}

impl Default for GpuiBackend {
    fn default() -> Self {
        Self::new()
    }
}

fn props<'a, T: 'static>(props: &'a Props, kind: &str) -> &'a T {
    props.downcast_ref::<T>().unwrap_or_else(|| panic!("{kind} node needs {} props", std::any::type_name::<T>()))
}

pub(crate) fn is_button(kind: NodeKind) -> bool {
    matches!(kind, NodeKind::Widget(w) if w.type_id == TypeId::of::<Button>())
}

fn not_actionable(why: impl Into<String>) -> ActionError {
    ActionError::NotActionable(why.into())
}

impl GpuiBackend {
    pub fn new() -> Self {
        Self {
            state: Rc::new(RefCell::new(State {
                nodes: HashMap::new(),
                windows: HashMap::new(),
                open: None,
                events: EventQueue::default(),
                dirty: BTreeSet::new(),
                app: None,
                wake: None,
                pending: Vec::new(),
                menus: HashMap::new(),
                actions: Vec::new(),
                swallow_up: Default::default(),
                theme: ThemeRegistry::new(Preset::current()),
                platform_scheme: None,
                scheme_listener: None,
                file_drop_listener: None,
                activation_listener: None,
                paints: HashMap::new(),
            })),
            bounds: Rc::default(),
            form_drawn: Rc::default(),
            focus_requests: Rc::default(),
            platform_focus: Rc::default(),
            surface_counts: Rc::default(),
            deferred: Rc::default(),
            menu_bar: Rc::default(),
            lists: Rc::default(),
            gestures: crate::gestures::Gestures::default(),
            fades: crate::fade::Fades::default(),
            overlay_exits: crate::overlay::Exits::default(),
            drawn: Rc::default(),
            hidden_sidebars: Rc::default(),
            splits: Rc::default(),
            dialogs: Rc::default(),
            tab_seam: Rc::default(),
        }
    }

    /// Runs `f` on the app at the host's next pump, outside every entity lease and GPUI update.
    /// For services answering from a browser or OS callback (a picked file, a clipboard read).
    pub fn defer(&self, f: impl FnOnce(&mut App) + 'static) {
        self.deferred.borrow_mut().push(Box::new(f));
        let (app, wake) = {
            let state = self.state.borrow();
            (state.app.clone(), state.wake.clone())
        };
        if let (Some(app), Some(wake)) = (app, wake) {
            app.spawn(async move |cx| cx.update(|cx| wake(cx))).detach();
        }
    }

    /// The app's menu bar, for a single canvas to draw (the web). Other GPUI desktops fold
    /// menus into the command palette and ignore it.
    pub fn set_menu_bar(&self, menus: Vec<nd_components::Menu>) {
        self.menu_bar.borrow_mut().set(menus);
        let mut state = self.state.borrow_mut();
        let windows: Vec<WindowId> = state.windows.keys().copied().collect();
        state.dirty.extend(windows);
    }

    /// Puts `text` on the platform clipboard (the async Clipboard API on the web), on the next
    /// turn, outside the GPUI update an app handler runs in.
    pub fn write_clipboard_text(&self, text: String) {
        let Some(app) = self.state.borrow().app.clone() else { return };
        app.spawn(async move |cx| cx.update(|cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string(text))))
            .detach();
    }

    /// Reads the clipboard's text and hands it to `done` on the app. On the web the browser may
    /// ask the user first, and refuses outside a user gesture; `done` then gets `None`.
    pub fn read_clipboard_text(&self, done: impl FnOnce(Option<String>, &mut App) + 'static) {
        let Some(app) = self.state.borrow().app.clone() else { return };
        let backend = self.clone();
        app.spawn(async move |cx| {
            let read = cx.update(|cx| cx.read_from_clipboard_async());
            let text = read.await.ok().flatten().and_then(|item| item.text());
            backend.defer(move |app| done(text, app));
        })
        .detach();
    }

    /// Schedules the host's pump, which delivers the events nodes queued, from outside a GPUI
    /// update (a browser engine's callback).
    pub fn wake(&self) {
        let (app, wake) = {
            let state = self.state.borrow();
            (state.app.clone(), state.wake.clone())
        };
        if let (Some(app), Some(wake)) = (app, wake) {
            app.spawn(async move |cx| cx.update(|cx| wake(cx))).detach();
        }
    }

    /// GPUI's app, for presenters that run tasks of their own (a browser engine's message
    /// loop). `None` until the host has started.
    pub fn async_app(&self) -> Option<AsyncApp> {
        self.state.borrow().app.clone()
    }

    pub(crate) fn take_deferred(&self) -> Vec<Deferred> {
        std::mem::take(&mut *self.deferred.borrow_mut())
    }

    pub(crate) fn attach_app(&self, app: AsyncApp, wake: Wake) {
        let mut state = self.state.borrow_mut();
        state.app = Some(app);
        state.wake = Some(wake);
    }

    /// The theme registry the windows paint from. Every window redraws.
    pub fn set_theme(&self, registry: ThemeRegistry) {
        let mut state = self.state.borrow_mut();
        state.theme = registry;
        let windows: Vec<WindowId> = state.windows.keys().copied().collect();
        state.dirty.extend(windows);
    }

    /// The platform's light or dark setting, as GPUI reports it.
    pub fn platform_scheme(&self) -> Option<Scheme> {
        self.state.borrow().platform_scheme
    }

    /// Calls `f` with the app whenever the platform's light or dark setting changes.
    pub fn on_platform_scheme(&self, f: impl Fn(&mut App, Scheme) + 'static) {
        self.state.borrow_mut().scheme_listener = Some(Rc::new(f));
    }

    /// Sets the one listener for files dropped onto any app window (`nd` routes them to
    /// `nd-system`'s events).
    pub fn on_file_drop(&self, f: impl Fn(WindowId, Vec<std::path::PathBuf>, (f32, f32)) + 'static) {
        self.state.borrow_mut().file_drop_listener = Some(Rc::new(f));
    }

    /// Sets the one listener for the app becoming active or inactive: some app window is the
    /// platform's active window, or none is.
    pub fn on_activation(&self, f: impl Fn(bool) + 'static) {
        self.state.borrow_mut().activation_listener = Some(Rc::new(f));
    }

    /// The OS's own colours (the system accent), as the OS hooks last read them.
    pub fn platform_palettes(&self) -> nd_theme::SchemePalettes {
        self.state.borrow().theme.os().palettes
    }

    /// The OS's animations-off setting, as the OS hooks read it at launch.
    pub fn platform_reduce_motion(&self) -> Option<bool> {
        self.state.borrow().theme.os().metrics.reduce_motion
    }

    pub(crate) fn set_os_reduce_motion(&self, reduce: Option<bool>) {
        self.gestures.set_reduce_motion(reduce == Some(true));
        let mut state = self.state.borrow_mut();
        let mut os = state.theme.os().clone();
        os.metrics.reduce_motion = reduce;
        state.theme.set_os(os);
    }

    /// Records the OS's own colours; true when they changed.
    pub(crate) fn set_os_palettes(&self, palettes: nd_theme::SchemePalettes) -> bool {
        let mut state = self.state.borrow_mut();
        if state.theme.os().palettes == palettes {
            return false;
        }
        let os = OsTheme { palettes, ..state.theme.os().clone() };
        state.theme.set_os(os);
        let windows: Vec<WindowId> = state.windows.keys().copied().collect();
        state.dirty.extend(windows);
        true
    }

    /// Records the platform's scheme; true when it changed.
    pub(crate) fn set_platform_scheme(&self, scheme: Scheme) -> bool {
        let mut state = self.state.borrow_mut();
        if state.platform_scheme == Some(scheme) {
            return false;
        }
        state.platform_scheme = Some(scheme);
        let os = OsTheme { scheme: Some(scheme), ..state.theme.os().clone() };
        state.theme.set_os(os);
        let windows: Vec<WindowId> = state.windows.keys().copied().collect();
        state.dirty.extend(windows);
        true
    }

    pub(crate) fn set_window_handle(&self, window: WindowId, handle: AnyWindowHandle) {
        self.state.borrow_mut().windows.entry(window).or_default().handle = Some(handle);
    }

    pub(crate) fn set_window_state(&self, window: WindowId, window_state: WindowState) {
        if let Some(record) = self.state.borrow_mut().windows.get_mut(&window) {
            record.state = window_state;
        }
    }

    pub(crate) fn forget_window(&self, window: WindowId) {
        self.gestures.forget_window(window);
        self.forget_dialogs(window);
        let mut state = self.state.borrow_mut();
        state.windows.remove(&window);
        state.dirty.remove(&window);
        state.paints.remove(&window);
    }

    pub(crate) fn take_pending(&self) -> Vec<(WindowId, gpui::PlatformInput)> {
        std::mem::take(&mut self.state.borrow_mut().pending)
    }

    pub(crate) fn wake_fn(&self) -> Option<Wake> {
        self.state.borrow().wake.clone()
    }

    /// Windows committed since they were last drawn; clears the set.
    pub(crate) fn take_dirty(&self) -> BTreeSet<WindowId> {
        std::mem::take(&mut self.state.borrow_mut().dirty)
    }

    pub fn window_handle(&self, window: WindowId) -> Option<AnyWindowHandle> {
        self.state.borrow().windows.get(&window)?.handle
    }

    /// The nd window a GPUI window shows.
    pub fn window_for(&self, handle: AnyWindowHandle) -> Option<WindowId> {
        // One GPUI window shows several app windows on a single canvas (the web); the newest
        // is on top and takes the keys. A group window shows its selected tab only.
        let hidden = &self.tab_seam.borrow().hidden;
        self.state
            .borrow()
            .windows
            .iter()
            .filter(|(id, r)| r.handle == Some(handle) && !hidden.contains(id))
            .map(|(id, _)| *id)
            .max()
    }

    /// Where a drawn tab strip's tabs, close buttons and `+` were painted, in window coordinates.
    pub fn tab_strip_layout(&self, node: NodeId) -> Option<crate::tab_bar::BarLayout> {
        Some(self.tab_seam.borrow().strips.get(&node)?.borrow().clone())
    }

    /// A tab group window's hidden tabs are neither visible nor key.
    fn tab_state(&self, window: WindowId, mut state: WindowState) -> WindowState {
        if self.tab_seam.borrow().hidden.contains(&window) {
            state.visible = false;
            state.key = false;
            state.main = false;
        }
        state
    }

    /// The `nd-virtual` lists drawn so far, by node.
    pub fn lists(&self) -> Vec<NodeId> {
        let mut lists: Vec<NodeId> = self.lists.borrow().keys().copied().collect();
        lists.sort();
        lists
    }

    /// The rows the list painted in its last frame, in order, with their bounds in window
    /// coordinates as GPUI laid them out; and the list's viewport (below a table's header).
    pub fn painted_rows(&self, list: NodeId) -> Option<(Rect, Vec<(nd_virtual::RowId, Rect)>)> {
        let lists = self.lists.borrow();
        let p = lists.get(&list)?;
        let bounds = self.bounds.borrow();
        let rows =
            p.painted.iter().map(|(row, node, placed)| (*row, bounds.get(node).copied().unwrap_or(*placed))).collect();
        Some((p.viewport, rows))
    }

    /// The node's bounds in window coordinates, as GPUI last laid it out.
    pub fn bounds(&self, node: NodeId) -> Option<Rect> {
        self.bounds.borrow().get(&node).copied()
    }

    pub fn is_live(&self, node: NodeId) -> bool {
        self.state.borrow().nodes.contains_key(&node)
    }

    /// Live nodes, for tests asserting the backend's copy matches the reconciler.
    pub fn live_count(&self) -> usize {
        self.state.borrow().nodes.len()
    }

    pub fn children(&self, node: NodeId) -> Vec<NodeId> {
        self.state.borrow().nodes.get(&node).map_or_else(Vec::new, |n| n.children.clone())
    }

    pub(crate) fn window_of(&self, node: NodeId) -> Option<WindowId> {
        let state = self.state.borrow();
        let mut at = node;
        loop {
            let n = state.nodes.get(&at)?;
            match n.parent {
                Some(p) => at = p,
                None => return state.windows.iter().find(|(_, r)| r.root == Some(at)).map(|(id, _)| *id),
            }
        }
    }

    /// Runs `f` on the GPUI window when GPUI's app is free (not drawing, not dispatching).
    fn with_window<R>(
        &self,
        window: WindowId,
        f: impl FnOnce(&mut gpui::Window, &mut gpui::App) -> R,
    ) -> Result<R, ActionError> {
        let (app, handle) = {
            let state = self.state.borrow();
            let handle = state.windows.get(&window).and_then(|r| r.handle).ok_or(ActionError::WindowGone)?;
            (state.app.clone().ok_or_else(|| not_actionable("no GPUI app"))?, handle)
        };
        let mut app = app;
        gpui::AppContext::update_window(&mut app, handle, |_, window, cx| f(window, cx))
            .map_err(|e| not_actionable(format!("GPUI window busy or gone: {e}")))
    }

    fn center(&self, node: NodeId) -> Result<(WindowId, Point), ActionError> {
        let window = self.window_of(node).ok_or_else(|| not_actionable(format!("{node:?} is not in a window")))?;
        let b = self.bounds(node).ok_or_else(|| not_actionable(format!("{node:?} has not been laid out")))?;
        Ok((window, Point { x: b.origin.x + b.size.width / 2., y: b.origin.y + b.size.height / 2. }))
    }
}

impl Backend for GpuiBackend {
    fn set_event_queue(&mut self, events: EventQueue) {
        self.state.borrow_mut().events = events;
    }

    fn begin(&mut self, window: WindowId) {
        let mut state = self.state.borrow_mut();
        assert!(state.open.is_none(), "transactions do not nest");
        state.open = Some(window);
        state.windows.entry(window).or_default();
    }

    fn create(&mut self, node: NodeId, kind: NodeKind, props: &Props) {
        let mut state = self.state.borrow_mut();
        if kind == NodeKind::Root {
            let window = state.open.expect("seam ops arrive inside begin/commit");
            state.windows.entry(window).or_default().root = Some(node);
        }
        state.nodes.insert(node, Node { kind, props: props.clone(), parent: None, children: Vec::new() });
    }

    fn insert(&mut self, parent: NodeId, node: NodeId, before: Option<NodeId>) {
        let mut state = self.state.borrow_mut();
        let siblings = &mut state.nodes.get_mut(&parent).expect("live parent").children;
        let at = before.and_then(|b| siblings.iter().position(|c| *c == b)).unwrap_or(siblings.len());
        siblings.insert(at, node);
        state.nodes.get_mut(&node).expect("live node").parent = Some(parent);
        // A subtree another window let go of keeps its per-node state (bounds, drawn controls,
        // lists); its scroll and swipe physics move to this window's gesture router.
        let from = self.tab_seam.borrow_mut().detached.remove(&node);
        if let (Some(from), Some(to)) = (from, state.open)
            && from != to
        {
            let mut subtree = vec![node];
            let mut i = 0;
            while let Some(at) = subtree.get(i).copied() {
                subtree.extend(state.nodes.get(&at).map_or(&[][..], |n| &n.children[..]).iter().copied());
                i += 1;
            }
            state.dirty.insert(from);
            drop(state);
            self.gestures.hand_over(from, to, &subtree);
        }
    }

    fn move_node(&mut self, node: NodeId, parent: NodeId, before: Option<NodeId>) {
        self.remove(node);
        self.insert(parent, node, before);
    }

    fn remove(&mut self, node: NodeId) {
        let window = self.window_of(node);
        let mut state = self.state.borrow_mut();
        let Some(parent) = state.nodes.get_mut(&node).and_then(|n| n.parent.take()) else { return };
        if let Some(p) = state.nodes.get_mut(&parent) {
            p.children.retain(|c| *c != node);
        }
        // A view moving to another window is detached in either window's transaction; the one
        // it left redraws without it.
        if let Some(window) = window {
            state.dirty.insert(window);
            self.tab_seam.borrow_mut().detached.insert(node, window);
        }
    }

    fn update(&mut self, node: NodeId, patch: &PropsPatch) {
        let mut state = self.state.borrow_mut();
        state.nodes.get_mut(&node).expect("live node").props = patch.new.clone();
    }

    /// Drawn controls show their props every frame; the window repaints so a refused change
    /// (props unchanged) replaces what the control showed during the gesture.
    fn reaffirm(&mut self, node: NodeId) {
        let mut state = self.state.borrow_mut();
        let mut at = node;
        while let Some(parent) = state.nodes.get(&at).and_then(|n| n.parent) {
            at = parent;
        }
        let window = state.windows.iter().find(|(_, r)| r.root == Some(at)).map(|(id, _)| *id);
        if let Some(window) = window {
            state.dirty.insert(window);
        }
    }

    fn set_frame(&mut self, _node: NodeId, _frame: Rect) {}

    fn surface(&mut self, _node: NodeId, _op: &SurfaceOp) {}

    /// nd owns focus (3.12). Drawn forms controls follow it with GPUI focus, which is where text
    /// input and their keys arrive; the window's next draw applies it.
    fn focus(&mut self, node: Option<NodeId>) {
        let Some(window) = self.state.borrow().open else { return };
        self.focus_requests.borrow_mut().insert(window, node);
    }

    fn release(&mut self, node: NodeId) {
        self.form_drawn.borrow_mut().remove(&node);
        self.splits.borrow_mut().remove(&node);
        self.gestures.forget(node);
        let custom = match self.state.borrow().nodes.get(&node).map(|n| n.kind) {
            Some(NodeKind::Widget(w)) => crate::custom::lookup(w.type_id),
            _ => None,
        };
        if let Some(custom) = custom {
            custom.released(node);
        }
        // A closing overlay keeps drawing its last subtree while it animates out.
        if let Some(window) = self.overlay_exits.leave(node, self.platform_reduce_motion() == Some(true)) {
            self.state.borrow_mut().dirty.insert(window);
        }
        let mut state = self.state.borrow_mut();
        if let Some(n) = state.nodes.remove(&node) {
            if let Some(p) = n.parent.and_then(|p| state.nodes.get_mut(&p)) {
                p.children.retain(|c| *c != node);
            }
            for c in n.children {
                if let Some(child) = state.nodes.get_mut(&c) {
                    child.parent = None;
                }
            }
        }
        drop(state);
        self.bounds.borrow_mut().remove(&node);
        self.fades.forget(node);
        self.surface_counts.borrow_mut().remove(&node);
        self.drawn.borrow_mut().remove(&node);
        let mut tabs = self.tab_seam.borrow_mut();
        tabs.strips.remove(&node);
        tabs.detached.remove(&node);
    }

    fn commit(&mut self, window: WindowId) {
        let mut state = self.state.borrow_mut();
        assert_eq!(state.open, Some(window), "commit closes the open transaction");
        state.open = None;
        state.dirty.insert(window);
    }

    /// Never asked: GPUI lays out with its own Taffy (`retains_frames` is false).
    fn measure(&mut self, _node: NodeId, _constraint: Constraint) -> Size {
        Size::default()
    }

    fn boundary_size(&self, _node: NodeId) -> Option<Size> {
        None
    }

    fn retains_frames(&self) -> bool {
        false
    }

    fn probe(&self, node: NodeId) -> Probe {
        let mut probe = Probe { bounds: self.bounds(node), ..Probe::default() };
        if let Ok(state) = self.state.try_borrow()
            && let Some(n) = state.nodes.get(&node)
            && is_button(n.kind)
        {
            let props = n.props.downcast_ref::<ButtonProps>();
            probe.name = props.map(|p| p.label.to_string());
            probe.enabled = Some(props.is_none_or(|p| p.enabled));
        }
        if let Ok(state) = self.state.try_borrow()
            && let Some(p) = state.nodes.get(&node).and_then(|n| n.props.downcast_ref::<SurfaceProps>())
            && p.count_input
        {
            probe.value = Some(self.surface_counts.borrow().get(&node).copied().unwrap_or_default().to_string());
        }
        self.probe_nav(node, &mut probe);
        self.probe_media(node, &mut probe);
        probe
    }

    fn window_state(&self, window: WindowId) -> Option<WindowState> {
        if let Ok(fresh) = self.with_window(window, |w, _| crate::host::window_state(w)) {
            return Some(self.tab_state(window, fresh));
        }
        let state = self.state.try_borrow().ok()?.windows.get(&window)?.state.clone();
        Some(self.tab_state(window, state))
    }

    fn set_window_title(&mut self, window: WindowId, title: &str) {
        let mut state = self.state.borrow_mut();
        state.windows.entry(window).or_default().title = title.to_owned().into();
        state.dirty.insert(window);
    }

    /// Recorded for the host, which carries it out on its next pump with the app free.
    fn window_command(&mut self, window: WindowId, command: nd_ui::backend::WindowCommand) {
        self.tab_seam.borrow_mut().commands.push((window, command));
        let (app, wake) = {
            let state = self.state.borrow();
            (state.app.clone(), state.wake.clone())
        };
        if let (Some(app), Some(wake)) = (app, wake) {
            app.spawn(async move |cx| cx.update(|cx| wake(cx))).detach();
        }
    }

    /// A button's click is the same `ClickEvent` its pointer handler emits, without moving a
    /// pointer: the drawn equivalent of an accessibility press. Pointer-shaped actions go through
    /// [`Backend::synthesize`] at the node's center. The host drains the events after the job.
    fn perform(&mut self, node: NodeId, action: &SemanticAction) -> Result<(), ActionError> {
        let kind = {
            let state = self.state.borrow();
            state.nodes.get(&node).map(|n| n.kind).ok_or_else(|| not_actionable(format!("{node:?} is not live")))?
        };
        // A drawn control takes GPUI focus on its window's next draw, as a click would give it.
        if *action == SemanticAction::Focus {
            let takes = {
                let state = self.state.borrow();
                state.nodes.get(&node).is_some_and(|n| crate::forms::takes_focus(n.kind, &n.props))
            };
            if takes {
                let window =
                    self.window_of(node).ok_or_else(|| not_actionable(format!("{node:?} is not in a window")))?;
                self.focus_requests.borrow_mut().insert(window, Some(node));
                self.state.borrow_mut().dirty.insert(window);
                let _ = self.with_window(window, |w, _| w.refresh());
                return Ok(());
            }
        }
        // A forms control answers the way its props say the drawn control would.
        if !matches!(action, SemanticAction::RightClick | SemanticAction::Hover | SemanticAction::Focus) {
            let state = self.state.borrow();
            let n = state.nodes.get(&node).expect("checked live above");
            if let NodeKind::Widget(w) = n.kind
                && let Some(response) = w.respond(&*n.props, action)
            {
                state.events.emitter(node).emit_boxed(response?);
                return Ok(());
            }
        }
        let press = |times: usize| {
            let events = self.state.borrow().events.clone();
            for _ in 0..times {
                events.emitter(node).emit(ClickEvent);
            }
        };
        let refused = {
            let state = self.state.borrow();
            state.nodes.get(&node).and_then(|n| n.props.downcast_ref::<ButtonProps>()).is_some_and(|p| !p.enabled)
        };
        if refused && matches!(action, SemanticAction::Click | SemanticAction::DoubleClick) {
            return Err(not_actionable(format!("{node:?} is disabled")));
        }
        match action {
            SemanticAction::Click if is_button(kind) => press(1),
            SemanticAction::DoubleClick if is_button(kind) => press(2),
            SemanticAction::RightClick | SemanticAction::Hover => {
                use nd_ui::semantics::{MouseButton, PointerPhase};
                let (window, at) = self.center(node)?;
                let phases: &[PointerPhase] = match action {
                    SemanticAction::Hover => &[PointerPhase::Move],
                    _ => &[PointerPhase::Down, PointerPhase::Up],
                };
                for phase in phases {
                    let input =
                        SyntheticInput::Pointer { phase: *phase, at, button: MouseButton::Right, click_count: 1 };
                    self.synthesize(window, &input)?;
                }
            }
            SemanticAction::ScrollIntoView => self.scroll_row_into_view(node)?,
            _ => return Err(ActionError::Unsupported(format!("{action:?} on {kind:?}"))),
        }
        Ok(())
    }

    /// Queues the input for the GPUI window's own event path, as the platform would deliver it.
    /// The host dispatches it on its next pump, after the job that posted it returned (the input
    /// reaches nd dispatch, which needs the app the job holds).
    fn synthesize(&mut self, window: WindowId, input: &SyntheticInput) -> Result<(), ActionError> {
        let mut state = self.state.borrow_mut();
        if state.windows.get(&window).and_then(|r| r.handle).is_none() {
            return Err(ActionError::WindowGone);
        }
        state.pending.extend(crate::input::platform_inputs(input).into_iter().map(|e| (window, e)));
        Ok(())
    }

    /// The scene GPUI last drew, rendered offscreen. Needs the `capture` feature (GPUI's
    /// `render_to_image`); Linux windows other than headless ones have no readback yet.
    fn capture(&mut self, window: WindowId) -> Result<Capture, ActionError> {
        #[cfg(feature = "capture")]
        {
            let image = self
                .with_window(window, |w, _| w.render_to_image())?
                .map_err(|e| ActionError::Unsupported(format!("capture: {e}")))?;
            crate::input::png(&image).map_err(|e| not_actionable(format!("PNG encoding: {e}")))
        }
        #[cfg(not(feature = "capture"))]
        {
            let _ = window;
            Err(ActionError::Unsupported("capture needs nd-backend-gpui's `capture` feature".into()))
        }
    }

    fn paints(&self, window: WindowId) -> Result<Vec<Paint>, ActionError> {
        self.paints_of(window)
    }
}

impl GpuiBackend {
    /// Runs one input through the window's gesture router with the current tree and the bounds
    /// of the last paint, then queues the events it produced and redraws for moved scroll views.
    pub(crate) fn gesture_input(
        &self,
        id: WindowId,
        window: &mut gpui::Window,
        cx: &mut gpui::App,
        wake: &Wake,
        f: impl FnOnce(&crate::gestures::Gestures, crate::gestures::Tree<'_>) -> Vec<nd_gesture::Output>,
    ) {
        if self.palette_covers(id) {
            return;
        }
        let outputs = {
            let state = self.state.borrow();
            // An open context menu's or palette's backdrop takes every press.
            if state.menus.contains_key(&id) {
                return;
            }
            let Some(root) = state.windows.get(&id).and_then(|w| w.root) else { return };
            let bounds = self.bounds.borrow();
            f(&self.gestures, (&state.nodes, &bounds, root))
        };
        let events = self.state.borrow().events.clone();
        let (queued, scrolled) = crate::gestures::deliver(&events, outputs);
        if queued {
            wake(cx);
        }
        if scrolled || self.gestures.needs_frame(id) {
            window.refresh();
        }
    }

    /// Scrolls the virtual list `node` sits in until the row holding it shows, the nearest edge
    /// first. A node in no list has nothing to scroll here.
    fn scroll_row_into_view(&self, node: NodeId) -> Result<(), ActionError> {
        let (handle, row) = {
            let state = self.state.borrow();
            let mut row = None;
            let mut at = Some(node);
            let mut found = None;
            while let Some(id) = at {
                let Some(n) = state.nodes.get(&id) else { break };
                if let Some(tag) = n
                    .props
                    .downcast_ref::<nd_ui::DivProps>()
                    .and_then(|p| p.scope.as_ref())
                    .and_then(|s| s.get::<nd_virtual::RowTag>())
                {
                    row = Some(tag.row);
                }
                if let Some(list) = n.props.downcast_ref::<nd_virtual::ListProps>() {
                    found = Some(list.handle.clone());
                    break;
                }
                at = n.parent;
            }
            match (found, row) {
                (Some(handle), Some(row)) => (handle, row),
                _ => return Err(ActionError::Unsupported(format!("ScrollIntoView: {node:?} is in no list row"))),
            }
        };
        handle.0.borrow_mut().scroll_to(row, nd_virtual::Align::Nearest, 0., false);
        let window = self.window_of(node).ok_or_else(|| not_actionable(format!("{node:?} is not in a window")))?;
        self.state.borrow_mut().dirty.insert(window);
        let _ = self.with_window(window, |w, _| w.refresh());
        Ok(())
    }

    pub(crate) fn paints_of(&self, window: WindowId) -> Result<Vec<Paint>, ActionError> {
        let state = self.state.try_borrow().map_err(|_| not_actionable("the backend is busy"))?;
        if !state.windows.contains_key(&window) {
            return Err(ActionError::WindowGone);
        }
        state.paints.get(&window).cloned().ok_or_else(|| not_actionable("the window has not drawn yet"))
    }

    /// The props of a live node, for the element builder.
    pub(crate) fn button_label(props: &Props) -> &ButtonProps {
        self::props(props, "Button")
    }
}
