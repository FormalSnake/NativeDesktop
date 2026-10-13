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

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant};

use futures::StreamExt;
use futures::channel::mpsc;
use nd_core::{FrameTime, Point, Rect, Size, WindowId};
use nd_diag::{FrameRecord, InputRecord, Watchdog};
use nd_gpui::gpui::prelude::FluentBuilder as _;
use nd_gpui::gpui::{
    self, AnyElement, AnyWindowHandle, AppContext, Bounds, Context, Decorations, InteractiveElement, IntoElement,
    ParentElement, Render, SharedString, Styled, TitlebarOptions, WindowBounds, div, px,
};
use nd_ui::dispatch::KeyDispatch;
use nd_ui::semantics::{MouseButton, PointerPhase, SyntheticInput, WindowState};
use nd_ui::{App, TitlebarInsets};

use crate::backend::{GpuiBackend, Wake, is_button};
use crate::budget::Budget;
use crate::clock::{DisplayClock, Pacing};
use crate::os::{FrameRequest, NoOsHooks, OsHooks};
use crate::render::{Build, CONTROL_HEIGHT, CONTROL_WIDTH, Look};

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

/// A window's GPUI root view.
pub struct NdView {
    host: Weak<Inner>,
    window: WindowId,
}

impl Render for NdView {
    fn render(&mut self, window: &mut gpui::Window, cx: &mut Context<Self>) -> impl IntoElement {
        match self.host.upgrade() {
            Some(host) => host.render_window(self.window, window, cx),
            None => div().into_any_element(),
        }
    }
}

struct HostWindow {
    view: Option<gpui::Entity<NdView>>,
    handle: Option<AnyWindowHandle>,
    clock: DisplayClock,
    /// When the window last printed its pacing line.
    reported: Option<Instant>,
    /// When the app last laid the window out at a new size.
    resized: Option<Instant>,
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

struct Inner {
    app: RefCell<App>,
    backend: GpuiBackend,
    windows: RefCell<BTreeMap<WindowId, HostWindow>>,
    hooks: Rc<dyn OsHooks>,
    origin: Instant,
    first_frame: Cell<bool>,
    had_windows: Cell<bool>,
    quit_when_empty: bool,
    sender: AppSender,
    budget: RefCell<Budget>,
    pacing: bool,
    debug: DebugFlags,
    subscriptions: RefCell<Vec<gpui::Subscription>>,
    tasks: RefCell<Vec<gpui::Task<()>>>,
    watchdog: RefCell<Option<Watchdog>>,
}

/// Builds a [`GpuiHost`].
pub struct HostBuilder {
    hooks: Rc<dyn OsHooks>,
    quit_when_empty: bool,
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

    /// Builds the app with `init` (which opens its windows) inside GPUI's app `cx` and opens
    /// them.
    pub fn start(self, cx: &mut gpui::App, init: impl FnOnce(&mut App)) -> GpuiHost {
        let backend = GpuiBackend::new();
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
            tasks: RefCell::default(),
            watchdog: RefCell::default(),
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
        HostBuilder { hooks: Rc::new(NoOsHooks), quit_when_empty: true }
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
        let handles: Vec<AnyWindowHandle> = self.0.windows.borrow().values().filter_map(|w| w.handle).collect();
        for handle in handles {
            let _ = handle.update(cx, |_, window, cx| window.draw(cx).clear(cx));
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

/// Where the OS controls sit when no OS crate says otherwise.
fn default_insets(window: &gpui::Window) -> TitlebarInsets {
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
    fn wake(&self) -> Wake {
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
            if inner.backend.drawn_claims_key(&event.keystroke, window) {
                return;
            }
            let Ok(mut app) = inner.app.try_borrow_mut() else { return };
            let result = app.dispatch_keystroke(id, &event.keystroke);
            drop(app);
            if result != KeyDispatch::Unhandled {
                cx.stop_propagation();
            }
            (inner.wake())(cx);
        });
        let weak = Rc::downgrade(self);
        let closed = cx.on_window_closed(move |cx, closed| {
            let Some(inner) = weak.upgrade() else { return };
            let id = inner
                .windows
                .borrow()
                .iter()
                .find(|(_, w)| w.handle.is_some_and(|h| h.window_id() == closed))
                .map(|(id, _)| *id);
            let Some(id) = id else { return };
            inner.windows.borrow_mut().remove(&id);
            inner.closed(id);
            if let Ok(mut app) = inner.app.try_borrow_mut() {
                app.close_window(id);
            }
            inner.pump(cx);
        });
        self.subscriptions.borrow_mut().extend([keys, closed]);
    }

    fn closed(&self, id: WindowId) {
        self.backend.forget_window(id);
        self.hooks.window_closed(id);
        if let Some(diag) = nd_diag::global() {
            diag.close_window(id.0);
        }
    }

    fn pump(self: &Rc<Self>, cx: &mut gpui::App) {
        for (id, event) in self.backend.take_pending() {
            if let Some(handle) = self.backend.window_handle(id) {
                self.budget.borrow_mut().input(Instant::now());
                let _ = handle.update(cx, |_, window, cx| {
                    window.dispatch_event(event, cx);
                });
            }
        }
        let needs_frame = {
            let Ok(mut app) = self.app.try_borrow_mut() else { return };
            app.flush_effects();
            app.drain_events();
            app.needs_frame()
        };
        self.sync_windows(cx);
        if needs_frame {
            self.frame_now(cx);
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
        let wanted: Vec<(WindowId, SharedString, Size)> = {
            let Ok(app) = self.app.try_borrow() else { return };
            app.window_ids()
                .filter_map(|id| app.window(id).map(|w| (id, w.title().clone(), w.viewport_size())))
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
            self.windows.borrow_mut().remove(&id);
            self.closed(id);
            if let Some(handle) = handle {
                let _ = handle.update(cx, |_, window, _| window.remove_window());
            }
        }
        for (id, title, size) in wanted {
            if self.windows.borrow().contains_key(&id) {
                continue;
            }
            self.open_window(cx, id, title, size);
        }
        if self.quit_when_empty && self.had_windows.get() && self.windows.borrow().is_empty() {
            cx.quit();
        }
    }

    fn open_window(self: &Rc<Self>, cx: &mut gpui::App, id: WindowId, title: SharedString, size: Size) {
        self.windows.borrow_mut().insert(
            id,
            HostWindow {
                view: None,
                handle: None,
                clock: DisplayClock::new(self.origin),
                reported: None,
                resized: None,
            },
        );
        let bounds = Bounds::centered(None, gpui::size(px(size.width), px(size.height)), cx);
        let mut options = gpui::WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions { title: Some(title), ..Default::default() }),
            // Only hidden windows are throttled (5.3): a visible unfocused window keeps its
            // display's rate while it animates.
            inactive_frame_interval: None,
            ..Default::default()
        };
        self.hooks.window_options(id, &mut options);
        let host = Rc::downgrade(self);
        let Ok(handle) = cx.open_window(options, |_, cx| cx.new(|_| NdView { host, window: id })) else {
            eprintln!("nd: GPUI could not open window {id:?}");
            self.windows.borrow_mut().remove(&id);
            return;
        };
        let any: AnyWindowHandle = handle.into();
        self.backend.set_window_handle(id, any);
        let view = handle.update(cx, |_, window, cx| {
            self.hooks.window_opened(id, window, cx);
            cx.entity()
        });
        if let Some(w) = self.windows.borrow_mut().get_mut(&id) {
            w.handle = Some(any);
            w.view = view.ok();
        }
        self.had_windows.set(true);
    }

    fn render_window(self: &Rc<Self>, id: WindowId, window: &mut gpui::Window, cx: &mut gpui::App) -> AnyElement {
        let started = Instant::now();
        let size = window.viewport_size();
        let size = Size { width: size.width.as_f32(), height: size.height.as_f32() };
        let insets = self.hooks.titlebar_insets(id, window).unwrap_or_else(|| default_insets(window));
        let animating = self.budget.borrow_mut().animating(started);
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
        self.notify(cx, committed.into_iter().filter(|w| *w != id));
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
        let build = Build {
            backend: &self.backend,
            nodes: &state.nodes,
            insets,
            look: Look::of(window.appearance()),
            wake: self.wake(),
            hooks: &self.hooks,
            window_id: id,
        };
        let element = build.node(root, window);
        drop(state);
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
        div()
            .size_full()
            .when(expose, |d| d.bg(gpui::rgb(0xff00ff)))
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
        let outputs = {
            let state = self.backend.state.borrow();
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
