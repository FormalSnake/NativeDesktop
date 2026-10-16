//! Runs an `nd-ui` [`App`] on GPUI: one GPUI window per app window, each showing an [`NdView`]
//! that builds elements from the backend's retained copy of the window's tree.
//!
//! When frames run: a native event (a click, a key, an automation job) queues work and the host
//! pumps on the next turn, outside any GPUI draw: native events drain to app handlers, windows
//! open and close to match the app, and if a view is dirty the app renders and commits there and
//! then. Committed windows are notified, so GPUI draws them at their own display's next frame.
//! Resizes and animation run the app's frame inside the window's draw instead, so the frame GPUI
//! presents is the frame laid out at that size (5.2) and animations tick at the display's rate.
//!
//! Keys reach nd dispatch first (3.12): a keystroke interceptor runs before GPUI's own dispatch,
//! and stops it when an nd binding took the key.
//!
//! The resize gate's switches, as on AppKit: `ND_DEBUG_EXPOSE=1` paints magenta behind the app's
//! tree, so area no node covers shows; `ND_DEBUG_DEFER_LIVE_LAYOUT=1` lays a resized window out
//! at most every 300 ms, the late layout the gate must catch; `ND_DIAG_WINDOW=1` prints
//! `nd: viewport WxH` per layout size, `nd: key <keystroke>` per key and `nd: window move` per
//! drag-region move on stderr.

use nd_core::time::{Duration, Instant};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::rc::{Rc, Weak};

use futures::StreamExt;
use futures::channel::mpsc;
use nd_core::{FrameTime, Point, Rect, Size, WindowId};
use nd_diag::{FrameRecord, InputRecord, Watchdog};
use nd_gpui::gpui::prelude::FluentBuilder as _;
use nd_gpui::gpui::{
    self, AnyElement, AnyWindowHandle, AppContext, Bounds, Context, Decorations, InteractiveElement, IntoElement,
    ParentElement, Render, SharedString, Styled, TitlebarOptions, WindowBounds, div, px,
};
use nd_theme::Scheme;
use nd_ui::dispatch::KeyDispatch;
use nd_ui::semantics::{MouseButton, PointerPhase, SyntheticInput, WindowState};
use nd_ui::{App, TitlebarInsets};

use crate::backend::{GpuiBackend, Wake, is_button};
use crate::budget::Budget;
use crate::clock::{DisplayClock, Pacing};
use crate::os::{FrameRequest, NoOsHooks, OsHooks};
use crate::render::{Build, CONTROL_HEIGHT, CONTROL_WIDTH};
use crate::window_tabs::{CloseHook, Groups, Request, SurfaceId, TabGroup};

fn point_of(p: gpui::Point<gpui::Pixels>) -> Point {
    Point { x: p.x.as_f32(), y: p.y.as_f32() }
}

/// Work for the main thread that needs the app, sent from any thread.
pub type AppJob = Box<dyn FnOnce(&mut App) + Send>;

enum Msg {
    Job(AppJob),
    Main(Box<dyn FnOnce() + Send>),
}

/// Runs [`AppJob`]s on the main thread between frames, outside every entity lease and outside
/// GPUI's own update. Cheap to clone and `Send`; the automation server holds one.
#[derive(Clone)]
pub struct AppSender(mpsc::UnboundedSender<Msg>);

impl AppSender {
    pub fn send(&self, job: AppJob) {
        let _ = self.0.unbounded_send(Msg::Job(job));
    }

    fn post(&self, f: Box<dyn FnOnce() + Send>) {
        let _ = self.0.unbounded_send(Msg::Main(f));
    }
}

/// A window's GPUI root view: one app window, or a group window showing a tab group's tabs.
pub struct NdView {
    host: Weak<Inner>,
    window: WindowId,
    surface: Option<SurfaceId>,
}

impl Render for NdView {
    fn render(&mut self, window: &mut gpui::Window, cx: &mut Context<Self>) -> impl IntoElement {
        match self.host.upgrade() {
            Some(host) if host.single_canvas => host.render_canvas(window, cx),
            Some(host) => match self.surface {
                Some(surface) => host.render_group(surface, window, cx),
                None => host.render_window(self.window, window, cx),
            },
            None => div().into_any_element(),
        }
    }
}

pub(crate) struct HostWindow {
    /// Shared by every tab of a group window, and by every window of a single canvas.
    pub(crate) view: Option<gpui::Entity<NdView>>,
    pub(crate) handle: Option<AnyWindowHandle>,
    clock: DisplayClock,
    /// When the window last printed its pacing line.
    reported: Option<Instant>,
    /// When the app last laid the window out at a new size.
    resized: Option<Instant>,
    /// The size the app asked for, which an in-canvas window keeps while the canvas has room.
    requested: Size,
}

/// The resize gate's environment switches (module docs).
#[derive(Clone, Copy)]
struct DebugFlags {
    expose: bool,
    defer_layout: bool,
    diag: bool,
}

impl DebugFlags {
    fn from_env() -> Self {
        let flag = |name: &str| std::env::var_os(name).is_some_and(|v| v == "1");
        Self {
            expose: flag("ND_DEBUG_EXPOSE"),
            defer_layout: flag("ND_DEBUG_DEFER_LIVE_LAYOUT"),
            diag: flag("ND_DIAG_WINDOW"),
        }
    }
}

pub(crate) struct Inner {
    pub(crate) app: RefCell<App>,
    pub(crate) backend: GpuiBackend,
    pub(crate) windows: RefCell<BTreeMap<WindowId, HostWindow>>,
    pub(crate) hooks: Rc<dyn OsHooks>,
    origin: Instant,
    first_frame: Cell<bool>,
    had_windows: Cell<bool>,
    quit_when_empty: bool,
    sender: AppSender,
    budget: RefCell<Budget>,
    pacing: bool,
    debug: DebugFlags,
    subscriptions: RefCell<Vec<gpui::Subscription>>,
    /// App windows the platform reports active; the app is active while one is.
    active: RefCell<BTreeSet<WindowId>>,
    tasks: RefCell<Vec<gpui::Task<()>>>,
    watchdog: RefCell<Option<Watchdog>>,
    /// Every app window draws into the first GPUI window (see [`HostBuilder::single_canvas`]).
    pub(crate) single_canvas: bool,
    /// The tab groups and the group windows showing them.
    pub(crate) groups: RefCell<Groups>,
}

/// Builds a [`GpuiHost`].
pub struct HostBuilder {
    hooks: Rc<dyn OsHooks>,
    quit_when_empty: bool,
    single_canvas: bool,
}

impl HostBuilder {
    /// The OS crate's hooks (`nd-os-linux`, `nd-os-windows`).
    pub fn hooks(mut self, hooks: impl OsHooks) -> Self {
        self.hooks = Rc::new(hooks);
        self
    }

    /// Quit GPUI's app when the last window closes (on by default, as on AppKit).
    pub fn quit_when_empty(mut self, quit: bool) -> Self {
        self.quit_when_empty = quit;
        self
    }

    /// Draw every app window into one GPUI window: the first window fills it, later ones float
    /// above it as in-canvas windows and sheets as dialogs with a scrim. On by default on the
    /// web, where GPUI has one canvas and no second window.
    pub fn single_canvas(mut self, single: bool) -> Self {
        self.single_canvas = single;
        self
    }

    /// Builds the app with `init` (which opens its windows) inside GPUI's app `cx` and opens
    /// them.
    pub fn start(self, cx: &mut gpui::App, init: impl FnOnce(&mut App)) -> GpuiHost {
        self.hooks.app_started(cx);
        let backend = GpuiBackend::new();
        backend.set_platform_scheme(scheme_of(cx.window_appearance()));
        backend.set_os_palettes(self.hooks.os_palettes());
        let mut app = App::with_backend(Box::new(backend.clone()));
        init(&mut app);
        let (tx, rx) = mpsc::unbounded();
        let inner = Rc::new(Inner {
            app: RefCell::new(app),
            backend,
            windows: RefCell::new(BTreeMap::new()),
            hooks: self.hooks,
            origin: Instant::now(),
            first_frame: Cell::new(false),
            had_windows: Cell::new(false),
            quit_when_empty: self.quit_when_empty,
            sender: AppSender(tx),
            budget: RefCell::new(Budget::from_env()),
            pacing: std::env::var_os("ND_PACING").is_some_and(|v| v == "1"),
            debug: DebugFlags::from_env(),
            subscriptions: RefCell::default(),
            active: RefCell::default(),
            tasks: RefCell::default(),
            watchdog: RefCell::default(),
            single_canvas: self.single_canvas,
            groups: RefCell::default(),
        });
        let weak = Rc::downgrade(&inner);
        let wake: Wake = Rc::new(move |cx: &mut gpui::App| {
            let weak = weak.clone();
            cx.defer(move |cx| {
                if let Some(inner) = weak.upgrade() {
                    inner.pump(cx);
                }
            });
        });
        inner.backend.attach_app(cx.to_async(), wake);
        if inner.debug.diag {
            let keys = cx.observe_keystrokes(|event, _, _| eprintln!("nd: key {}", event.keystroke.unparse()));
            inner.subscriptions.borrow_mut().push(keys);
        }
        inner.listen(cx, rx);
        let weak = Rc::downgrade(&inner);
        inner.hooks.on_os_colors_changed(Rc::new(move |cx| {
            if let Some(inner) = weak.upgrade() {
                inner.os_colors(cx);
            }
        }));
        let weak = Rc::downgrade(&inner);
        cx.set_global(CloseHook(Rc::new(move |handle, cx| {
            if let Some(inner) = weak.upgrade() {
                inner.os_close(handle, cx);
            }
        })));
        if let Some(diag) = nd_diag::global() {
            let sender = inner.sender.clone();
            *inner.watchdog.borrow_mut() = Some(diag.start_watchdog(Box::new(move |f| sender.post(f))));
        }
        inner.sync_windows(cx);
        GpuiHost(inner)
    }
}

/// An `nd-ui` app on GPUI. [`run`] is the usual entry; tests start one inside a headless GPUI
/// app with [`GpuiHost::builder`].
#[derive(Clone)]
pub struct GpuiHost(Rc<Inner>);

impl GpuiHost {
    pub fn builder() -> HostBuilder {
        HostBuilder { hooks: Rc::new(NoOsHooks), quit_when_empty: true, single_canvas: cfg!(target_arch = "wasm32") }
    }

    pub fn backend(&self) -> &GpuiBackend {
        &self.0.backend
    }

    /// A handle for running jobs on the app from other threads.
    pub fn sender(&self) -> AppSender {
        self.0.sender.clone()
    }

    /// Runs `f` on the app, then pumps: events drain, windows open or close, dirty views render.
    /// Panics when called from inside the app (a handler or a frame).
    pub fn update<R>(&self, cx: &mut gpui::App, f: impl FnOnce(&mut App) -> R) -> R {
        let result = {
            let mut app = self.0.app.borrow_mut();
            let result = f(&mut app);
            app.flush_effects();
            result
        };
        self.0.pump(cx);
        result
    }

    /// Runs `f` on the app as an automation job does: outside GPUI's own update, so backend
    /// queries that reach the GPUI window (capture, window state) can. Pump afterwards.
    pub fn job<R>(&self, f: impl FnOnce(&mut App) -> R) -> R {
        let mut app = self.0.app.borrow_mut();
        let result = f(&mut app);
        app.flush_effects();
        result
    }

    /// Reads the app. Panics when called from inside it.
    pub fn read<R>(&self, f: impl FnOnce(&App) -> R) -> R {
        f(&self.0.app.borrow())
    }

    /// Delivers queued input and native events, opens and closes windows, runs a frame when a
    /// view is dirty. Hosts pump by themselves; headless tests call it after posting input.
    pub fn pump(&self, cx: &mut gpui::App) {
        self.0.pump(cx);
    }

    /// The GPUI window showing the app window.
    pub fn window_handle(&self, window: WindowId) -> Option<AnyWindowHandle> {
        self.0.windows.borrow().get(&window)?.handle
    }

    /// Each open window's display rate and the rate it last drew at.
    pub fn pacing(&self) -> BTreeMap<WindowId, Pacing> {
        self.0.windows.borrow().iter().map(|(id, w)| (*id, w.clock.pacing())).collect()
    }

    /// Draws every window now, as GPUI does on its next frame. For headless tests, whose
    /// windows draw only when asked.
    pub fn draw(&self, cx: &mut gpui::App) {
        let mut handles: Vec<AnyWindowHandle> = Vec::new();
        for handle in self.0.windows.borrow().values().filter_map(|w| w.handle) {
            if !handles.contains(&handle) {
                handles.push(handle);
            }
        }
        for handle in handles {
            let _ = handle.update(cx, |_, window, cx| window.draw(cx).clear(cx));
        }
    }
}

impl GpuiHost {
    /// The tab group `window` is a tab of, as the host shows it; `None` for a window outside
    /// every group.
    pub fn tab_group(&self, window: WindowId) -> Option<TabGroup> {
        self.0.groups.borrow().info(window)
    }

    /// What the OS close button of the GPUI window showing `window` does: asks the app to close
    /// it, or every tab of its group window, on the next pump.
    pub fn request_window_close(&self, cx: &mut gpui::App, window: WindowId) {
        let handle = self.window_handle(window);
        if let Some(handle) = handle {
            self.0.os_close(handle, cx);
        }
    }
}

/// A window's state as GPUI has it, in global logical points.
pub(crate) fn window_state(window: &gpui::Window) -> WindowState {
    let b = window.bounds();
    WindowState {
        key: window.is_window_active(),
        main: window.is_window_active(),
        visible: window.is_visible(),
        frame: Some(Rect::new(b.origin.x.as_f32(), b.origin.y.as_f32(), b.size.width.as_f32(), b.size.height.as_f32())),
    }
}

fn scheme_of(appearance: gpui::WindowAppearance) -> Scheme {
    match appearance {
        gpui::WindowAppearance::Dark | gpui::WindowAppearance::VibrantDark => Scheme::Dark,
        gpui::WindowAppearance::Light | gpui::WindowAppearance::VibrantLight => Scheme::Light,
    }
}

/// Where the OS controls sit when no OS crate says otherwise.
pub(crate) fn default_insets(window: &gpui::Window) -> TitlebarInsets {
    if cfg!(target_os = "macos") {
        // The window keeps AppKit's titlebar on the GPUI path, so content starts below it.
        return TitlebarInsets::default();
    }
    match window.window_decorations() {
        Decorations::Server => TitlebarInsets::default(),
        Decorations::Client { .. } => {
            let offered = window.window_controls();
            let buttons = 1 + usize::from(offered.minimize) + usize::from(offered.maximize);
            TitlebarInsets { leading: 0., trailing: buttons as f32 * CONTROL_WIDTH, height: CONTROL_HEIGHT }
        }
    }
}

fn record_frame(app: &App, id: WindowId, time: FrameTime, work_ms: f64) {
    let Some(diag) = nd_diag::global() else { return };
    let interval_ms = time.interval() * 1000.;
    let frame =
        FrameRecord { frame: time.frame, target: time.target, interval_ms, work_ms, missed: work_ms > interval_ms };
    diag.record_frame(id.0, frame, app.semantic_window(id).as_ref());
}

impl Inner {
    pub(crate) fn wake(&self) -> Wake {
        self.backend.wake_fn().expect("host attached its wake")
    }

    /// The job queue, the keystroke interceptor and window close tracking.
    fn listen(self: &Rc<Self>, cx: &mut gpui::App, mut rx: mpsc::UnboundedReceiver<Msg>) {
        let weak = Rc::downgrade(self);
        let task = cx.spawn(async move |cx| {
            while let Some(msg) = rx.next().await {
                let Some(inner) = weak.upgrade() else { break };
                match msg {
                    Msg::Main(f) => f(),
                    Msg::Job(job) => {
                        if let Ok(mut app) = inner.app.try_borrow_mut() {
                            job(&mut app);
                            app.flush_effects();
                        }
                        cx.update(|cx| inner.pump(cx));
                    }
                }
            }
        });
        self.tasks.borrow_mut().push(task);

        let weak = Rc::downgrade(self);
        let keys = cx.intercept_keystrokes(move |event, window, cx| {
            let Some(inner) = weak.upgrade() else { return };
            let Some(id) = inner.backend.window_for(window.window_handle()) else { return };
            if let Some(diag) = nd_diag::global() {
                diag.record_input(InputRecord {
                    t: inner.origin.elapsed().as_secs_f64(),
                    window: id.0 as i64,
                    kind: "key_down".into(),
                    text: event.keystroke.key_char.clone(),
                    ..InputRecord::default()
                });
            }
            if event.keystroke.key == "escape" && inner.backend.close_menu(id) {
                cx.stop_propagation();
                (inner.wake())(cx);
                return;
            }
            if inner.tab_key_first(id, &event.keystroke) {
                cx.stop_propagation();
                window.refresh();
                return;
            }
            if inner.backend.drawn_claims_key(&event.keystroke, window) {
                return;
            }
            let Ok(mut app) = inner.app.try_borrow_mut() else { return };
            let result = app.dispatch_keystroke(id, &event.keystroke);
            drop(app);
            if result != KeyDispatch::Unhandled {
                cx.stop_propagation();
            } else if inner.tab_key_last(id, &event.keystroke) {
                cx.stop_propagation();
                window.refresh();
            }
            (inner.wake())(cx);
        });
        let weak = Rc::downgrade(self);
        let closed = cx.on_window_closed(move |cx, closed| {
            let Some(inner) = weak.upgrade() else { return };
            let ids: Vec<WindowId> = inner
                .windows
                .borrow()
                .iter()
                .filter(|(_, w)| w.handle.is_some_and(|h| h.window_id() == closed))
                .map(|(id, _)| *id)
                .collect();
            if ids.is_empty() {
                return;
            }
            inner.groups.borrow_mut().surfaces.retain(|_, s| s.handle.is_none_or(|h| h.window_id() != closed));
            for id in ids {
                inner.windows.borrow_mut().remove(&id);
                inner.closed(id);
                if let Ok(mut app) = inner.app.try_borrow_mut() {
                    app.close_window(id);
                }
            }
            inner.refresh_hidden();
            inner.pump(cx);
        });
        self.subscriptions.borrow_mut().extend([keys, closed]);
    }

    /// The platform switched light and dark: the backend's own theme follows, then the app's
    /// listener (`nd` feeds its registry), and every window redraws.
    /// The system colours changed (the accent): the backend's theme takes them, then the app's
    /// listener, which re-reads them into its registry, and every window redraws.
    fn os_colors(self: &Rc<Self>, cx: &mut gpui::App) {
        if !self.backend.set_os_palettes(self.hooks.os_palettes()) {
            return;
        }
        let scheme = scheme_of(cx.window_appearance());
        let listener = self.backend.state.borrow().scheme_listener.clone();
        if let Some(listener) = listener
            && let Ok(mut app) = self.app.try_borrow_mut()
        {
            listener(&mut app, scheme);
            app.flush_effects();
        }
        let all: Vec<WindowId> = self.windows.borrow().keys().copied().collect();
        self.notify(cx, all.into_iter());
        self.pump(cx);
    }

    fn platform_appearance(self: &Rc<Self>, cx: &mut gpui::App) {
        let scheme = scheme_of(cx.window_appearance());
        if !self.backend.set_platform_scheme(scheme) {
            return;
        }
        let listener = self.backend.state.borrow().scheme_listener.clone();
        if let Some(listener) = listener
            && let Ok(mut app) = self.app.try_borrow_mut()
        {
            listener(&mut app, scheme);
            app.flush_effects();
        }
        let all: Vec<WindowId> = self.windows.borrow().keys().copied().collect();
        self.notify(cx, all.into_iter());
        self.pump(cx);
    }

    /// The OS close of a GPUI window, which never closes it there and then: the app is asked on
    /// the next pump, for every tab of a group window, and closes what it lets go (3.13, 5.5).
    pub(crate) fn os_close(&self, handle: AnyWindowHandle, cx: &mut gpui::App) {
        let Some(id) = self.backend.window_for(handle) else { return };
        let request = match self.groups.borrow().surface_of(id) {
            Some(surface) => Request::CloseAll(surface),
            None => Request::Close(id),
        };
        self.groups.borrow_mut().requests.push(request);
        (self.wake())(cx);
    }

    fn closed(&self, id: WindowId) {
        self.backend.forget_window(id);
        self.hooks.window_closed(id);
        if let Some(diag) = nd_diag::global() {
            diag.close_window(id.0);
        }
    }

    fn pump(self: &Rc<Self>, cx: &mut gpui::App) {
        let actions = self.backend.take_actions();
        if !actions.is_empty()
            && let Ok(mut app) = self.app.try_borrow_mut()
        {
            for (window, action) in actions {
                app.dispatch_action(window, &*action);
            }
        }
        let deferred = self.backend.take_deferred();
        if !deferred.is_empty()
            && let Ok(mut app) = self.app.try_borrow_mut()
        {
            for job in deferred {
                job(&mut app);
                app.flush_effects();
            }
        }
        self.tab_requests(cx);
        for (id, event) in self.backend.take_pending() {
            if let Some(handle) = self.backend.window_handle(id) {
                self.budget.borrow_mut().input(Instant::now());
                let _ = handle.update(cx, |_, window, cx| match event {
                    // As the platform does: the key goes through dispatch, and text no binding or
                    // key listener took goes to the focused input handler.
                    gpui::PlatformInput::KeyDown(down) => {
                        window.dispatch_keystroke(down.keystroke, cx);
                    }
                    event => {
                        window.dispatch_event(event, cx);
                    }
                });
            }
        }
        let needs_frame = {
            let Ok(mut app) = self.app.try_borrow_mut() else { return };
            // A drawn control the app did not make focusable still takes nd focus off the
            // node that had it, so bindings stop matching in that node's contexts.
            for (window, node) in std::mem::take(&mut *self.backend.platform_focus.borrow_mut()) {
                if !app.platform_focused(window, node) {
                    app.platform_focused(window, None);
                }
            }
            app.flush_effects();
            app.drain_events();
            app.needs_frame()
        };
        self.sync_windows(cx);
        if needs_frame {
            self.frame_now(cx);
        } else {
            // A context menu opened or closed without a frame.
            let dirty = self.backend.take_dirty();
            self.notify(cx, dirty.into_iter());
        }
    }

    /// Runs the app's frame outside any draw and asks GPUI to draw the windows it committed.
    fn frame_now(&self, cx: &mut gpui::App) {
        let committed = {
            let Ok(mut app) = self.app.try_borrow_mut() else { return };
            let now = Instant::now();
            let mut times = Vec::new();
            for (id, w) in self.windows.borrow_mut().iter_mut() {
                let time = w.clock.tick(now);
                app.tick_window(*id, time);
                times.push((*id, time));
            }
            app.frame();
            let work_ms = now.elapsed().as_secs_f64() * 1000.;
            let committed = self.backend.take_dirty();
            for (id, time) in times {
                if committed.contains(&id) {
                    record_frame(&app, id, time, work_ms);
                }
            }
            committed
        };
        self.notify(cx, committed.into_iter());
    }

    fn notify(&self, cx: &mut gpui::App, windows: impl Iterator<Item = WindowId>) {
        let views: Vec<gpui::Entity<NdView>> = {
            let open = self.windows.borrow();
            windows.filter_map(|id| open.get(&id).and_then(|w| w.view.clone())).collect()
        };
        for view in views {
            view.update(cx, |_, cx| cx.notify());
        }
    }

    /// Opens a GPUI window for every app window that has none and closes those of closed ones.
    fn sync_windows(self: &Rc<Self>, cx: &mut gpui::App) {
        type Wanted = (WindowId, SharedString, Size, Option<SharedString>);
        let wanted: Vec<Wanted> = {
            let Ok(app) = self.app.try_borrow() else { return };
            app.window_ids()
                .filter_map(|id| {
                    let w = app.window(id)?;
                    Some((id, w.title().clone(), w.viewport_size(), w.options().tab_group.clone()))
                })
                .collect()
        };
        let gone: Vec<(WindowId, Option<AnyWindowHandle>)> = self
            .windows
            .borrow()
            .iter()
            .filter(|(id, _)| !wanted.iter().any(|(w, ..)| w == *id))
            .map(|(id, w)| (*id, w.handle))
            .collect();
        for (id, handle) in gone {
            // Notifying a group window below can pump again and close it there first.
            if self.windows.borrow_mut().remove(&id).is_none() {
                continue;
            }
            self.closed(id);
            let tab = self.groups.borrow_mut().detach(id);
            match tab {
                Some((surface, true)) => {
                    self.groups.borrow_mut().surfaces.remove(&surface);
                }
                Some((surface, false)) => {
                    self.refresh_hidden();
                    self.notify_surface(surface, cx);
                    continue;
                }
                None => {}
            }
            // The canvas outlives the app windows drawn into it, as a group window does its tabs.
            let shared = |h: &AnyWindowHandle| self.windows.borrow().values().any(|w| w.handle == Some(*h));
            if let Some(handle) = handle.filter(|h| !self.single_canvas && !shared(h)) {
                let _ = handle.update(cx, |_, window, _| window.remove_window());
            }
        }
        for (id, title, size, group) in wanted {
            self.backend.state.borrow_mut().windows.entry(id).or_default().title = title.clone();
            if self.windows.borrow().contains_key(&id) {
                continue;
            }
            self.open_window(cx, id, title, size, group);
        }
        if self.quit_when_empty && self.had_windows.get() && self.windows.borrow().is_empty() {
            cx.quit();
        }
    }

    fn open_window(
        self: &Rc<Self>,
        cx: &mut gpui::App,
        id: WindowId,
        title: SharedString,
        size: Size,
        group: Option<SharedString>,
    ) {
        // A tab joins the group window of its group the user used last, selected.
        let joined = group.as_ref().and_then(|g| {
            let mut groups = self.groups.borrow_mut();
            let surface = groups.active(g)?;
            let s = groups.surfaces.get(&surface)?;
            let (handle, view) = (s.handle?, s.view.clone()?);
            let end = s.tabs.len();
            groups.attach(surface, id, end);
            Some((surface, handle, view))
        });
        if let Some((surface, handle, view)) = joined {
            self.windows.borrow_mut().insert(id, self.host_window(Some(handle), Some(view), size));
            self.backend.set_window_handle(id, handle);
            self.had_windows.set(true);
            self.refresh_hidden();
            self.notify_surface(surface, cx);
            return;
        }
        let surface = group.map(|g| self.groups.borrow_mut().create(g, id));
        let canvas = self
            .single_canvas
            .then(|| self.windows.borrow().values().find_map(|w| Some((w.handle?, w.view.clone()?))))
            .flatten();
        let (handle, view) = canvas.map_or((None, None), |(h, v)| (Some(h), Some(v)));
        self.windows.borrow_mut().insert(id, self.host_window(handle, view.clone(), size));
        if let Some(handle) = handle {
            if let Some(s) = surface
                && let Some(s) = self.groups.borrow_mut().surfaces.get_mut(&s)
            {
                s.handle = Some(handle);
                s.view = view;
            }
            self.backend.set_window_handle(id, handle);
            self.had_windows.set(true);
            let all: Vec<WindowId> = self.windows.borrow().keys().copied().collect();
            self.notify(cx, all.into_iter());
            return;
        }
        let bounds = Bounds::centered(None, gpui::size(px(size.width), px(size.height)), cx);
        if self.open_gpui_window(cx, id, title, bounds, surface).is_none() {
            self.windows.borrow_mut().remove(&id);
            if let Some(s) = surface {
                self.groups.borrow_mut().surfaces.remove(&s);
            }
            return;
        }
        self.had_windows.set(true);
    }

    fn host_window(
        &self,
        handle: Option<AnyWindowHandle>,
        view: Option<gpui::Entity<NdView>>,
        size: Size,
    ) -> HostWindow {
        HostWindow {
            view,
            handle,
            clock: DisplayClock::new(self.origin),
            reported: None,
            resized: None,
            requested: size,
        }
    }

    /// Opens a GPUI window for the app window `id`, or for the group window `surface` showing
    /// it, and points both at it.
    pub(crate) fn open_gpui_window(
        self: &Rc<Self>,
        cx: &mut gpui::App,
        id: WindowId,
        title: SharedString,
        bounds: Bounds<gpui::Pixels>,
        surface: Option<SurfaceId>,
    ) -> Option<AnyWindowHandle> {
        let mut options = gpui::WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions { title: Some(title), ..Default::default() }),
            // Only hidden windows are throttled (5.3): a visible unfocused window keeps its
            // display's rate while it animates.
            inactive_frame_interval: None,
            ..Default::default()
        };
        options.focus = self.app.try_borrow().ok().and_then(|app| app.window(id).map(|w| w.options().activate)).unwrap_or(true);
        self.hooks.window_options(id, &mut options);
        let host = Rc::downgrade(self);
        let Ok(handle) = cx.open_window(options, |window, cx| {
            let view = cx.new(|_| NdView { host, window: id, surface });
            // The window's first draw can open further app windows, which must find this one as
            // their canvas, or their group window, before `open_window` returns.
            self.bind(id, surface, window.window_handle(), view.clone());
            view
        }) else {
            eprintln!("nd: GPUI could not open window {id:?}");
            return None;
        };
        let any: AnyWindowHandle = handle.into();
        self.backend.set_window_handle(id, any);
        let weak = Rc::downgrade(self);
        let weak_appearance = weak.clone();
        let weak_close = weak.clone();
        let view = handle.update(cx, |_, window, cx| {
            self.hooks.window_opened(id, window, cx);
            // Light-dismiss overlays close when the app loses activation, checked a turn later so
            // a key change between the app's own windows does not count.
            let activation = cx.observe_window_activation(window, move |_, window, cx| {
                if window.is_window_active() {
                    if let Some(inner) = weak.upgrade()
                        && let Some(surface) = surface
                    {
                        inner.groups.borrow_mut().touch(surface);
                    }
                    return;
                }
                let weak = weak.clone();
                cx.defer(move |cx| {
                    let Some(inner) = weak.upgrade() else { return };
                    let active =
                        cx.windows().iter().any(|w| w.update(cx, |_, w, _| w.is_window_active()).unwrap_or(false));
                    if !active {
                        for window in inner.windows.borrow().keys() {
                            inner.backend.close_menu(*window);
                        }
                        if let Ok(mut app) = inner.app.try_borrow_mut() {
                            app.app_deactivated();
                        }
                        inner.pump(cx);
                    }
                });
            });
            self.subscriptions.borrow_mut().push(activation);
            let weak = weak_appearance;
            let appearance = window.observe_window_appearance(move |_, cx| {
                if let Some(inner) = weak.upgrade() {
                    inner.platform_appearance(cx);
                }
            });
            self.subscriptions.borrow_mut().push(appearance);
            let weak = Rc::downgrade(self);
            let activation = cx.observe_window_activation(window, move |_, window, cx| {
                if let Some(inner) = weak.upgrade() {
                    inner.window_activation(id, window.is_window_active(), cx);
                }
            });
            self.subscriptions.borrow_mut().push(activation);
            if !self.single_canvas {
                window.on_window_should_close(cx, move |window, cx| {
                    if let Some(inner) = weak_close.upgrade() {
                        inner.os_close(window.window_handle(), cx);
                    }
                    false
                });
            }
            cx.entity()
        });
        if let Ok(view) = view {
            self.bind(id, surface, any, view);
        }
        Some(any)
    }

    /// Points the app window and its group window at the GPUI window `handle`.
    fn bind(&self, id: WindowId, surface: Option<SurfaceId>, handle: AnyWindowHandle, view: gpui::Entity<NdView>) {
        if let Some(w) = self.windows.borrow_mut().get_mut(&id) {
            w.handle = Some(handle);
            w.view = Some(view.clone());
        }
        if let Some(s) = surface
            && let Some(s) = self.groups.borrow_mut().surfaces.get_mut(&s)
        {
            s.handle = Some(handle);
            s.view = Some(view);
        }
        self.backend.set_window_handle(id, handle);
    }

    /// Follows one window's active state. Moving focus between two app windows deactivates one
    /// before it activates the other, so the app-level answer is taken once the effects settle.
    fn window_activation(self: &Rc<Self>, id: WindowId, active: bool, cx: &mut gpui::App) {
        let was = !self.active.borrow().is_empty();
        if active {
            self.active.borrow_mut().insert(id);
        } else {
            self.active.borrow_mut().remove(&id);
        }
        let weak = Rc::downgrade(self);
        cx.defer(move |_| {
            let Some(inner) = weak.upgrade() else { return };
            let now = !inner.active.borrow().is_empty();
            let listener = inner.backend.state.borrow().activation_listener.clone();
            if now != was
                && let Some(listener) = listener
            {
                listener(now);
            }
        });
    }

    fn render_window(self: &Rc<Self>, id: WindowId, window: &mut gpui::Window, cx: &mut gpui::App) -> AnyElement {
        let size = window.viewport_size();
        let size = Size { width: size.width.as_f32(), height: size.height.as_f32() };
        let insets = self.hooks.titlebar_insets(id, window).unwrap_or_else(|| default_insets(window));
        self.render_window_at(id, size, insets, None, window, cx)
    }

    /// The app windows of a single canvas: the oldest normal window fills it, the rest float
    /// above in the order they opened.
    fn render_canvas(self: &Rc<Self>, window: &mut gpui::Window, cx: &mut gpui::App) -> AnyElement {
        let viewport = window.viewport_size();
        let viewport = Size { width: viewport.width.as_f32(), height: viewport.height.as_f32() };
        let open: Vec<(WindowId, Size)> = self.windows.borrow().iter().map(|(id, w)| (*id, w.requested)).collect();
        let kinds: Vec<(WindowId, Size, nd_ui::WindowKind, SharedString)> = {
            let Ok(app) = self.app.try_borrow() else {
                window.request_animation_frame();
                return div().into_any_element();
            };
            open.into_iter()
                .filter_map(|(id, size)| app.window(id).map(|w| (id, size, w.options().kind, w.title().clone())))
                .collect()
        };
        // A tab group is one in-canvas window showing its selected tab, where its first tab
        // opened.
        let kinds: Vec<(WindowId, Size, nd_ui::WindowKind, SharedString)> = {
            let groups = self.groups.borrow();
            let mut seen = Vec::new();
            kinds
                .iter()
                .filter_map(|k| match groups.surface_of(k.0) {
                    None => Some(k.clone()),
                    Some(s) if seen.contains(&s) => None,
                    Some(s) => {
                        seen.push(s);
                        let shown = groups.surfaces[&s].selected;
                        kinds.iter().find(|k| k.0 == shown).cloned()
                    }
                })
                .collect()
        };
        let base = kinds.iter().find(|(_, _, kind, _)| !matches!(kind, nd_ui::WindowKind::Sheet { .. })).map(|k| k.0);
        let look = crate::render::Look::of(self.backend.state.borrow().theme.app_theme());
        let (bar, dropdown) = {
            let menus = self.backend.menu_bar.borrow();
            if menus.is_empty() {
                (None, None)
            } else {
                let toggle = {
                    let menus = self.backend.menu_bar.clone();
                    move |open: Option<usize>, window: &mut gpui::Window| {
                        menus.borrow_mut().open = open;
                        window.refresh();
                    }
                };
                let choose = {
                    let (menus, backend) = (self.backend.menu_bar.clone(), self.backend.clone());
                    move |action: Box<dyn gpui::Action>, window: &mut gpui::Window| {
                        menus.borrow_mut().open = None;
                        window.refresh();
                        let target = backend.window_for(window.window_handle());
                        backend.defer(move |app| {
                            if let Some(target) = target {
                                app.dispatch_action(target, &*action);
                            }
                        });
                    }
                };
                let (bar, dropdown) = crate::canvas::menu_bar(&menus, &look, toggle, choose);
                (Some(bar), dropdown)
            }
        };
        let viewport = match bar {
            Some(_) => {
                Size { width: viewport.width, height: (viewport.height - crate::canvas::MENU_BAR_HEIGHT).max(1.) }
            }
            None => viewport,
        };
        let mut stack = div().w(px(viewport.width)).h(px(viewport.height)).relative();
        if let Some(base) = base {
            let insets = self.hooks.titlebar_insets(base, window).unwrap_or_else(|| default_insets(window));
            stack = stack.child(self.canvas_window(base, viewport, insets, window, cx));
        }
        let mut floating = 0;
        for (id, requested, kind, title) in kinds.into_iter().filter(|k| Some(k.0) != base) {
            let sheet = matches!(kind, nd_ui::WindowKind::Sheet { .. });
            let chrome = if sheet { 0. } else { crate::canvas::TITLE_HEIGHT };
            let size = crate::canvas::fit(requested, viewport, chrome);
            let element = self.canvas_window(id, size, TitlebarInsets::default(), window, cx);
            let close = {
                let weak = Rc::downgrade(self);
                move |cx: &mut gpui::App| {
                    let Some(inner) = weak.upgrade() else { return };
                    let request = match inner.groups.borrow().surface_of(id) {
                        Some(surface) => Request::CloseAll(surface),
                        None => Request::Close(id),
                    };
                    inner.groups.borrow_mut().requests.push(request);
                    inner.pump(cx);
                }
            };
            stack = stack.child(if sheet {
                crate::canvas::sheet(size, element, &look)
            } else {
                floating += 1;
                crate::canvas::floating(floating, size, viewport, title, element, &look, close)
            });
        }
        match bar {
            Some(bar) => div()
                .size_full()
                .relative()
                .flex()
                .flex_col()
                .child(bar)
                .child(stack)
                .children(dropdown)
                .into_any_element(),
            None => stack.into_any_element(),
        }
    }

    /// An in-canvas window: the app window, or the tab group it is the shown tab of.
    fn canvas_window(
        self: &Rc<Self>,
        id: WindowId,
        size: Size,
        insets: TitlebarInsets,
        window: &mut gpui::Window,
        cx: &mut gpui::App,
    ) -> AnyElement {
        let surface = self.groups.borrow().surface_of(id);
        match surface {
            Some(surface) => self.group_content(surface, size, insets, window, cx),
            None => self.render_window_at(id, size, insets, None, window, cx),
        }
    }

    /// The app window's tree at `size`. `under_title` (a group window's tab bar) goes under the
    /// tree's title row, or above the tree when it has none.
    pub(crate) fn render_window_at(
        self: &Rc<Self>,
        id: WindowId,
        size: Size,
        insets: TitlebarInsets,
        under_title: Option<AnyElement>,
        window: &mut gpui::Window,
        cx: &mut gpui::App,
    ) -> AnyElement {
        let started = Instant::now();
        let animating = self.budget.borrow_mut().animating(started);
        let mut frame_time = None;
        let mut laid_out = None;
        let (committed, needs_more) = match self.app.try_borrow_mut() {
            Ok(mut app) => {
                if app.window(id).is_some_and(|w| w.viewport_size() != size) {
                    let deferred = self.debug.defer_layout
                        && self
                            .windows
                            .borrow()
                            .get(&id)
                            .and_then(|w| w.resized)
                            .is_some_and(|at| started.duration_since(at) < Duration::from_millis(300));
                    if deferred {
                        window.request_animation_frame();
                    } else {
                        app.resize_window(id, size);
                        if let Some(w) = self.windows.borrow_mut().get_mut(&id) {
                            w.resized = Some(started);
                        }
                        if self.debug.diag {
                            eprintln!("nd: viewport {}x{}", size.width, size.height);
                        }
                    }
                }
                laid_out = app.window(id).map(|w| w.viewport_size());
                app.set_titlebar_insets(id, insets);
                if animating {
                    app.refresh_window(id);
                }
                let time = self.tick(id, started, window, cx);
                frame_time = Some(time.target);
                if app.needs_frame() {
                    app.tick_window(id, time);
                    app.frame();
                    record_frame(&app, id, time, started.elapsed().as_secs_f64() * 1000.);
                }
                if app.window_ids().count() != self.windows.borrow().len() {
                    (self.wake())(cx);
                }
                (self.backend.take_dirty(), app.needs_frame())
            }
            // An automation job holds the app; draw the tree as it stands and come back.
            Err(_) => (Default::default(), true),
        };
        // On a single canvas every app window is this view, which is drawing now, as are the
        // other tabs of a group window.
        if !self.single_canvas {
            let this = window.window_handle();
            let others: Vec<WindowId> = {
                let open = self.windows.borrow();
                committed.into_iter().filter(|w| open.get(w).is_some_and(|w| w.handle != Some(this))).collect()
            };
            self.notify(cx, others.into_iter());
        }
        self.backend.set_window_state(id, window_state(window));
        if needs_more || animating {
            window.request_animation_frame();
        }
        self.after_draw(id, started, window);
        self.backend.prepare_drawn(id, window, cx);

        // Flings, springs and gesture timers advance on this window's frames.
        let (outputs, more) = self.backend.gestures.step(id);
        let events = self.backend.state.borrow().events.clone();
        if crate::gestures::deliver(&events, outputs).0 {
            (self.wake())(cx);
        }
        if more {
            window.request_animation_frame();
        }

        let state = self.backend.state.borrow();
        let Some(root) = state.windows.get(&id).and_then(|w| w.root) else { return div().into_any_element() };
        let mut build = Build::new(&self.backend, &state.nodes, &state.theme, insets, self.wake(), &self.hooks, id);
        build.time = frame_time.unwrap_or_else(|| self.origin.elapsed().as_secs_f64());
        *build.under_title.borrow_mut() = under_title;
        let element = build.node(root, window, cx);
        let element = match build.under_title.take() {
            Some(bar) => div()
                .size_full()
                .flex()
                .flex_col()
                .child(bar)
                .child(div().flex_1().min_h_0().w_full().child(element))
                .into_any_element(),
            None => element,
        };
        let menu = state.menus.get(&id).map(|m| build.menu(m, window));
        let body_font = state.theme.window_theme(id).fonts.body.clone();
        let look = crate::render::Look::of(state.theme.window_theme(id));
        let overlay = self.hooks.window_overlay(id, look, build.title_bottom.get(), window);
        let paints = build.take_paints();
        drop(state);
        self.backend.state.borrow_mut().paints.insert(id, paints);
        // Building gave a drawn target GPUI focus; a request left over means nd focus is
        // somewhere no drawn control is.
        if self.backend.focus_requests.borrow_mut().remove(&id).is_some() {
            window.blur(cx);
        }
        let origin = self.origin;
        let (down, up, moved, wheel, pinch, pressure) = (
            Rc::downgrade(self),
            Rc::downgrade(self),
            Rc::downgrade(self),
            Rc::downgrade(self),
            Rc::downgrade(self),
            Rc::downgrade(self),
        );
        let expose = self.debug.expose;
        let (backend, fresh) = (self.backend.clone(), self.backend.clone());
        let drops = self.backend.clone();
        div()
            .size_full()
            .font(body_font)
            // A new press starts clean: a release owed to an earlier consumed press that never
            // arrived (the compositor dropped it) must not eat this press's release.
            .capture_any_mouse_down(move |_, _, _| fresh.clear_swallow(id))
            .capture_any_mouse_up(move |_, _, cx| {
                if backend.take_swallow(id) {
                    cx.stop_propagation();
                }
            })
            .when(expose, |d| d.bg(gpui::rgb(0xff00ff)))
            .on_drop(move |paths: &gpui::ExternalPaths, window, _cx| {
                let listener = drops.state.borrow().file_drop_listener.clone();
                if let Some(listener) = listener {
                    let at = window.mouse_position();
                    listener(id, paths.paths().to_vec(), (at.x.as_f32(), at.y.as_f32()));
                }
            })
            .capture_any_mouse_down(move |event, window, cx| {
                if let Some(diag) = nd_diag::global() {
                    diag.record_input(InputRecord {
                        t: origin.elapsed().as_secs_f64(),
                        window: id.0 as i64,
                        kind: "mouse_down".into(),
                        position: Some([event.position.x.as_f32().into(), event.position.y.as_f32().into()]),
                        ..InputRecord::default()
                    });
                }
                let Some(inner) = down.upgrade() else { return };
                if event.button != gpui::MouseButton::Left {
                    return;
                }
                if let Ok(mut app) = inner.app.try_borrow_mut() {
                    app.pointer_down(id);
                }
                let at = point_of(event.position);
                inner.gesture_input(id, window, cx, |g, tree| g.pointer(id, nd_gesture::PointerPhase::Down, at, tree));
            })
            .capture_any_mouse_up(move |event, window, cx| {
                let Some(inner) = up.upgrade() else { return };
                if event.button == gpui::MouseButton::Left {
                    let at = point_of(event.position);
                    inner
                        .gesture_input(id, window, cx, |g, tree| g.pointer(id, nd_gesture::PointerPhase::Up, at, tree));
                }
            })
            .on_mouse_move(move |event, window, cx| {
                let Some(inner) = moved.upgrade() else { return };
                if event.pressed_button == Some(gpui::MouseButton::Left) {
                    let at = point_of(event.position);
                    inner.gesture_input(id, window, cx, |g, tree| {
                        g.pointer(id, nd_gesture::PointerPhase::Move, at, tree)
                    });
                }
            })
            .on_scroll_wheel(move |event, window, cx| {
                let Some(inner) = wheel.upgrade() else { return };
                inner.gesture_input(id, window, cx, |g, tree| g.scroll(id, event, tree));
            })
            .capture_pinch(move |event, window, cx| {
                let Some(inner) = pinch.upgrade() else { return };
                let gesture = crate::gestures::pinch_event(event);
                let at = point_of(event.position);
                inner
                    .gesture_input(id, window, cx, |g, tree| g.os_gesture(id, gesture, at, tree).into_iter().collect());
            })
            .on_mouse_pressure(move |event, window, cx| {
                let Some(inner) = pressure.upgrade() else { return };
                if inner.backend.gestures.pressure(id, event.stage) {
                    let at = point_of(event.position);
                    let gesture = crate::gestures::force_click(at);
                    inner.gesture_input(id, window, cx, |g, tree| {
                        g.os_gesture(id, gesture, at, tree).into_iter().collect()
                    });
                }
            })
            .map(|d| match laid_out {
                // The drawn tree flexes to the GPUI window, so the deferred layout's stale size
                // is applied here, where the nd layout would have put it.
                Some(size) if self.debug.defer_layout => {
                    d.child(div().w(px(size.width)).h(px(size.height)).child(element))
                }
                _ => d.child(element),
            })
            .children(overlay)
            .children(menu)
            .into_any_element()
    }

    /// Stamps the frame from the window's display clock, or from the OS crate's measured one.
    fn tick(&self, id: WindowId, started: Instant, window: &gpui::Window, cx: &gpui::App) -> FrameTime {
        let mut windows = self.windows.borrow_mut();
        let Some(w) = windows.get_mut(&id) else { return FrameTime::default() };
        w.clock.set_interval(window.refresh_interval(cx));
        let request = FrameRequest {
            window: id,
            now: started,
            now_seconds: w.clock.seconds(started),
            interval: w.clock.interval(),
            last: w.clock.last(),
        };
        let time = match self.hooks.frame_time(&request, window) {
            Some(measured) => {
                let t = w.clock.seconds(started);
                w.clock.accept(measured, t)
            }
            None => w.clock.tick(started),
        };
        if self.pacing && w.reported.is_none_or(|r| started.duration_since(r) >= Duration::from_secs(2)) {
            w.reported = Some(started);
            let p = w.clock.pacing();
            eprintln!(
                "nd-pacing: window={} display_hz={:.1} measured_hz={:.1} frames={} visible={}",
                id.0,
                p.display_hz,
                p.measured_hz,
                p.frames,
                window.is_visible()
            );
        }
        time
    }

    /// Runs one input through the window's gesture router with the current tree and the bounds
    /// of the last paint, then queues the events it produced and redraws for moved scroll views.
    fn gesture_input(
        &self,
        id: WindowId,
        window: &mut gpui::Window,
        cx: &mut gpui::App,
        f: impl FnOnce(&crate::gestures::Gestures, crate::gestures::Tree<'_>) -> Vec<nd_gesture::Output>,
    ) {
        if self.backend.palette_covers(id) {
            return;
        }
        let outputs = {
            let state = self.backend.state.borrow();
            // An open context menu's or palette's backdrop takes every press.
            if state.menus.contains_key(&id) {
                return;
            }
            let Some(root) = state.windows.get(&id).and_then(|w| w.root) else { return };
            let bounds = self.backend.bounds.borrow();
            f(&self.backend.gestures, (&state.nodes, &bounds, root))
        };
        let events = self.backend.state.borrow().events.clone();
        let (queued, scrolled) = crate::gestures::deliver(&events, outputs);
        if queued {
            (self.wake())(cx);
        }
        if scrolled || self.backend.gestures.needs_frame(id) {
            window.refresh();
        }
    }

    /// First-frame bookkeeping and the budget knobs, once GPUI has drawn this frame.
    fn after_draw(self: &Rc<Self>, id: WindowId, started: Instant, window: &mut gpui::Window) {
        let first = !self.first_frame.replace(true);
        if !first && !self.budget.borrow().wants_present() {
            return;
        }
        let weak = Rc::downgrade(self);
        window.on_next_frame(move |window, cx| {
            let Some(inner) = weak.upgrade() else { return };
            if first {
                if let Some(diag) = nd_diag::global() {
                    diag.clean_up_later();
                }
                inner.budget.borrow_mut().first_frame(inner.origin);
            }
            let refresh = window.refresh_interval(cx);
            let done = inner.budget.borrow_mut().presented(id, started, refresh);
            if done {
                cx.quit();
            } else if first && inner.budget.borrow().clicks > 0 {
                inner.start_clicking(cx);
            }
        });
    }

    /// `ND_BUDGET_CLICKS`: clicks the first button through the window's input path, 150 ms apart.
    fn start_clicking(self: &Rc<Self>, cx: &mut gpui::App) {
        let weak = Rc::downgrade(self);
        let task = cx.spawn(async move |cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(150)).await;
                let Some(inner) = weak.upgrade() else { break };
                if inner.budget.borrow().clicks == 0 {
                    break;
                }
                inner.budget.borrow_mut().clicks -= 1;
                let target = {
                    let state = inner.backend.state.borrow();
                    state.nodes.iter().filter(|(_, n)| is_button(n.kind)).map(|(id, _)| *id).min()
                };
                let Some(node) = target else { break };
                let Some(b) = inner.backend.bounds(node) else { continue };
                let Ok(mut app) = inner.app.try_borrow_mut() else { continue };
                let Some(window) = app.window_of(node) else { continue };
                let at = Point { x: b.origin.x + b.size.width / 2., y: b.origin.y + b.size.height / 2. };
                for phase in [PointerPhase::Down, PointerPhase::Up] {
                    let input = SyntheticInput::Pointer { phase, at, button: MouseButton::Left, click_count: 1 };
                    let _ = app.synthesize(window, &input);
                }
                drop(app);
                cx.update(|cx| inner.pump(cx));
            }
        });
        self.tasks.borrow_mut().push(task);
    }
}

/// Builds the app with `init` (which opens its windows) and runs GPUI's event loop until the last
/// window closes.
pub fn run(hooks: impl OsHooks, init: impl FnOnce(&mut App) + 'static) {
    run_with(hooks, init, |_| {})
}

/// As [`run`], handing `started` an [`AppSender`] once the windows are up.
pub fn run_with(hooks: impl OsHooks, init: impl FnOnce(&mut App) + 'static, started: impl FnOnce(AppSender) + 'static) {
    nd_gpui::gpui_platform::application().run(move |cx: &mut gpui::App| {
        let host = GpuiHost::builder().hooks(hooks).start(cx, init);
        started(host.sender());
        cx.activate(true);
        // GPUI's app owns the host from here, and with it the subscriptions and tasks.
        cx.set_global(HostGlobal(host));
    });
}

struct HostGlobal(#[allow(dead_code)] GpuiHost);

impl gpui::Global for HostGlobal {}
