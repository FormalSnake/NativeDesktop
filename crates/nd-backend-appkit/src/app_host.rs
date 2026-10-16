//! Runs an `nd-ui` [`App`] on AppKit: one [`WindowHost`] per app window, frames inside AppKit's
//! layout pass (so a live resize presents the frame it laid out), the display clock waking
//! layout while the app has work, native events drained on the run loop turn after they were
//! queued, and every window's key-downs routed through [`App::dispatch_keystroke`] before
//! AppKit's first responder sees them.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, VecDeque};
use std::rc::{Rc, Weak};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use nd_core::{FrameTime, WindowId};
use nd_diag::{FrameRecord, Watchdog};
use nd_ui::dispatch::KeyDispatch;
use nd_ui::{App, Keystroke, WindowKind};
use objc2::rc::Retained;
use objc2_app_kit::{NSApplication, NSWindow, NSWindowOrderingMode, NSWindowTabbingMode};
use objc2_foundation::{MainThreadMarker, NSString};

use crate::dispatch::node_for_responder;
use crate::{AppKitBackend, KeyRouter, NdAppKitPlatform, NdWindow, WindowHost};

struct Host {
    app: RefCell<App>,
    windows: RefCell<BTreeMap<WindowId, WindowHost>>,
    platform: NdAppKitPlatform,
    backend: AppKitBackend,
    first_frame: Cell<bool>,
    _watchdog: Option<Watchdog>,
}

fn record_frame(app: &App, id: WindowId, time: FrameTime, started: Instant) {
    let Some(diag) = nd_diag::global() else { return };
    let work_ms = started.elapsed().as_secs_f64() * 1000.;
    let interval_ms = time.interval() * 1000.;
    let frame =
        FrameRecord { frame: time.frame, target: time.target, interval_ms, work_ms, missed: work_ms > interval_ms };
    diag.record_frame(id.0, frame, app.semantic_window(id).as_ref());
}

impl Host {
    /// Opens a host for every app window that has none, and drops hosts of closed ones.
    fn sync_windows(self: &Rc<Self>) {
        let Ok(app) = self.app.try_borrow() else { return };
        let ids: Vec<WindowId> = app.window_ids().collect();
        let mut windows = self.windows.borrow_mut();
        windows.retain(|id, _| {
            let open = ids.contains(id);
            if !open && let Some(diag) = nd_diag::global() {
                diag.close_window(id.0);
            }
            open
        });
        for id in ids {
            if windows.contains_key(&id) {
                continue;
            }
            let Some(window) = app.window(id) else { continue };
            let size = window.viewport_size();
            let options = window.options().clone();
            let ns_window = self.platform.open_window(window.title(), size.width.into(), size.height.into());
            let host = WindowHost::new(&self.backend, id, &ns_window);
            host.apply_options(&options);
            let this = Rc::downgrade(self);
            let clock = host.clock().clone();
            let window = ns_window.clone();
            host.on_layout(move |size| {
                let Some(this) = this.upgrade() else { return };
                let Ok(mut app) = this.app.try_borrow_mut() else { return };
                app.resize_window(id, size);
                let time = clock.next_frame_time();
                app.tick_window(id, time);
                let started = Instant::now();
                app.frame();
                record_frame(&app, id, time, started);
                drop(app);
                if !this.first_frame.replace(true) {
                    if let Some(diag) = nd_diag::global() {
                        diag.clean_up_later();
                    }
                    crate::diag::after_first_frame(&window);
                }
                this.after_frame();
            });
            let this = Rc::downgrade(self);
            host.clock().on_frame(move |_| tick(&this));
            ns_window.set_router(Some(Rc::new(AppRouter { host: Rc::downgrade(self), window: id })));
            let sheet_parent = match options.kind {
                WindowKind::Sheet { parent } => windows.get(&parent),
                WindowKind::Normal | WindowKind::Settings => None,
            };
            match (sheet_parent, &options.tab_group) {
                (Some(parent), _) => parent.window().beginSheet_completionHandler(&ns_window, None),
                (None, Some(group)) if !matches!(options.kind, WindowKind::Sheet { .. }) => {
                    join_tab_group(&ns_window, group);
                    self.show(&ns_window, &options);
                }
                (None, _) => {
                    ns_window.center();
                    self.show(&ns_window, &options);
                }
            }
            windows.insert(id, host);
        }
    }

    /// Shows a new window: key and the app active, or behind what has focus when the window
    /// opens in the background.
    fn show(&self, window: &NdWindow, options: &nd_ui::WindowOptions) {
        if options.activate {
            self.platform.show(window);
        } else {
            window.orderFront(None);
        }
    }

    fn after_frame(self: &Rc<Self>) {
        let opened = self.app.borrow().window_ids().count() != self.windows.borrow().len();
        if opened {
            let this = self.clone();
            // A frame can open or close windows; hosts follow on the next turn, outside layout.
            crate::events::defer(move || this.sync_windows());
        }
        self.kick();
    }

    /// Starts the display clocks while the app has a frame to draw.
    fn kick(&self) {
        if self.app.try_borrow().is_ok_and(|app| app.needs_frame()) {
            for host in self.windows.borrow().values() {
                host.clock().set_animating(true);
            }
        }
    }
}

/// Makes `window` a native tab of `group` (ARCHITECTURE.md 5.5): next to the group's key window,
/// else its most recent visible one, at the trailing end of that window's tabs. Distinct groups
/// carry distinct identifiers, so AppKit never merges them.
fn join_tab_group(window: &NdWindow, group: &str) {
    let identifier = NSString::from_str(&format!("nd.{group}"));
    window.setTabbingMode(NSWindowTabbingMode::Preferred);
    window.setTabbingIdentifier(&identifier);
    let mtm = MainThreadMarker::from(window);
    let this: &NSWindow = window;
    let candidates: Vec<Retained<NSWindow>> = NSApplication::sharedApplication(mtm)
        .windows()
        .iter()
        .filter(|w| !std::ptr::eq::<NSWindow>(&**w, this) && w.isVisible() && *w.tabbingIdentifier() == *identifier)
        .collect();
    let parent = candidates.iter().find(|w| w.isKeyWindow()).or(candidates.last());
    let Some(parent) = parent else {
        window.center();
        return;
    };
    let group = parent.tabGroup();
    // AppKit throws from addTabbedWindow while the overview shows.
    if let Some(group) = &group
        && group.isOverviewVisible()
    {
        group.setOverviewVisible(false);
    }
    let last = group.and_then(|g| g.windows().lastObject()).unwrap_or_else(|| parent.clone());
    last.addTabbedWindow_ordered(window, NSWindowOrderingMode::Above);
}

/// On each display refresh: a layout pass (where the frame runs) while there is work, else the
/// clock pauses.
fn tick(host: &Weak<Host>) {
    let Some(host) = host.upgrade() else { return };
    let busy = host.app.try_borrow().map_or(true, |app| app.needs_frame());
    for window in host.windows.borrow().values() {
        if busy {
            window.request_layout();
        } else {
            window.clock().set_animating(false);
        }
    }
}

/// One window's key router: nd dispatch first, AppKit's first responder mirrored back.
struct AppRouter {
    host: Weak<Host>,
    window: WindowId,
}

impl KeyRouter for AppRouter {
    fn key_down(&self, keystroke: &Keystroke) -> KeyDispatch {
        let Some(host) = self.host.upgrade() else { return KeyDispatch::Unhandled };
        let Ok(mut app) = host.app.try_borrow_mut() else { return KeyDispatch::Unhandled };
        let result = app.dispatch_keystroke(self.window, keystroke);
        // What the key queued (Return's click on the default button, Escape's dismissal) runs
        // now, and a listener's focus move reaches AppKit now, so the next key in the same event
        // batch already goes to the new first responder.
        app.drain_events();
        drop(app);
        // A key that opened a window (Cmd-T's new tab) needs that window's host before the
        // frame draws it.
        host.sync_windows();
        if let Ok(mut app) = host.app.try_borrow_mut()
            && app.needs_frame()
        {
            app.frame();
        }
        host.after_frame();
        result
    }

    fn pointer_down(&self) {
        let Some(host) = self.host.upgrade() else { return };
        if let Ok(mut app) = host.app.try_borrow_mut() {
            app.pointer_down(self.window);
        }
    }

    fn files_dropped(&self, paths: Vec<std::path::PathBuf>, at: (f32, f32)) {
        if let Some(listener) = FILE_DROP.with(|l| l.borrow().clone()) {
            listener(self.window, paths, at);
        }
    }

    fn responder_changed(&self, window: &NdWindow) {
        let Some(host) = self.host.upgrade() else { return };
        let Some(node) = node_for_responder(&host.backend, window.firstResponder().as_deref()) else { return };
        if let Ok(mut app) = host.app.try_borrow_mut() {
            app.platform_focused(self.window, node);
        }
    }
}

/// Work for the main thread that needs the app, sent from any thread.
pub type AppJob = Box<dyn FnOnce(&mut App) + Send>;

type Jobs = Arc<Mutex<VecDeque<AppJob>>>;

thread_local! {
    static HOST: RefCell<Weak<Host>> = const { RefCell::new(Weak::new()) };
    static FILE_DROP: RefCell<Option<FileDropListener>> = const { RefCell::new(None) };
}

/// Told of files dropped onto an app window, at a point in window coordinates from the top left.
pub type FileDropListener = Rc<dyn Fn(WindowId, Vec<std::path::PathBuf>, (f32, f32))>;

/// Sets the one listener for files dropped onto any app window (`nd` routes them to
/// `nd-system`'s events). Main thread only.
pub fn on_file_drop(listener: impl Fn(WindowId, Vec<std::path::PathBuf>, (f32, f32)) + 'static) {
    FILE_DROP.with(|l| *l.borrow_mut() = Some(Rc::new(listener)));
}

/// Runs [`AppJob`]s on the main thread, between frames and outside every entity lease. Cheap to
/// clone and `Send`; the automation server holds one.
#[derive(Clone)]
pub struct AppSender {
    jobs: Jobs,
}

impl AppSender {
    pub fn send(&self, job: AppJob) {
        self.jobs.lock().expect("job queue").push_back(job);
        let jobs = self.jobs.clone();
        dispatch2::DispatchQueue::main().exec_async(move || run_jobs(&jobs));
    }
}

fn run_jobs(jobs: &Jobs) {
    let Some(host) = HOST.with(|h| h.borrow().upgrade()) else { return };
    loop {
        let Some(job) = jobs.lock().expect("job queue").pop_front() else { break };
        let Ok(mut app) = host.app.try_borrow_mut() else {
            // A frame or a handler holds the app (a nested run loop); try again next turn.
            jobs.lock().expect("job queue").push_front(job);
            let jobs = jobs.clone();
            crate::events::defer(move || run_jobs(&jobs));
            return;
        };
        job(&mut app);
        app.flush_effects();
    }
    host.after_frame();
}

type AppTask = Box<dyn FnOnce(&mut App)>;

/// Runs `f` with the app on a later main run loop turn: outside the native callback that asks
/// for it, outside every entity lease (3.13). Menus, alerts and panels report through here.
/// Dropped when no app is running.
pub fn defer_with_app(f: impl FnOnce(&mut App) + 'static) {
    defer_task(Box::new(f));
}

fn defer_task(task: AppTask) {
    crate::events::defer(move || {
        let Some(host) = HOST.with(|h| h.borrow().upgrade()) else { return };
        let Ok(mut app) = host.app.try_borrow_mut() else {
            // A nested run loop holds the app; come back next turn.
            defer_task(task);
            return;
        };
        task(&mut app);
        app.flush_effects();
        drop(app);
        host.after_frame();
    });
}

/// The app window whose NSWindow is AppKit's key window.
pub(crate) fn key_window() -> Option<nd_core::WindowId> {
    let host = HOST.with(|h| h.borrow().upgrade())?;
    let mtm = MainThreadMarker::new()?;
    let key = objc2_app_kit::NSApplication::sharedApplication(mtm).keyWindow()?;
    host.backend.window_id_of(&key)
}

/// An app on AppKit, before or while its run loop runs. [`run`] is the usual entry; tests build
/// one directly and pump events through [`AppHost::platform`].
pub struct AppHost(Rc<Host>);

impl AppHost {
    /// Builds the app with `init` (which opens its windows) and shows them.
    pub fn new(init: impl FnOnce(&mut App)) -> Self {
        let mtm = MainThreadMarker::new().expect("nd apps run on the main thread");
        let platform = NdAppKitPlatform::new(mtm);
        let backend = AppKitBackend::new(mtm);
        let mut app = App::with_backend(Box::new(backend.clone()));
        init(&mut app);
        let watchdog = nd_diag::global().map(|diag| {
            nd_diag::set_native_tree(move || crate::diag::native_tree(mtm));
            diag.start_watchdog(crate::diag::post_to_main())
        });
        let host = Rc::new(Host {
            app: RefCell::new(app),
            windows: RefCell::new(BTreeMap::new()),
            platform,
            backend,
            first_frame: Cell::new(false),
            _watchdog: watchdog,
        });
        let weak = Rc::downgrade(&host);
        host.backend.on_wake(move || {
            let Some(host) = weak.upgrade() else { return };
            if let Ok(mut app) = host.app.try_borrow_mut() {
                app.drain_events();
            }
            host.kick();
        });
        host.sync_windows();
        HOST.with(|h| *h.borrow_mut() = Rc::downgrade(&host));
        Self(host)
    }

    /// A handle for running jobs on the app from other threads.
    pub fn sender(&self) -> AppSender {
        AppSender { jobs: Jobs::default() }
    }

    pub fn platform(&self) -> &NdAppKitPlatform {
        &self.0.platform
    }

    pub fn backend(&self) -> &AppKitBackend {
        &self.0.backend
    }

    /// The app window's NSWindow, once its host is open.
    pub fn ns_window(&self, id: WindowId) -> Option<Retained<NdWindow>> {
        use objc2::Message;
        self.0.windows.borrow().get(&id).map(|h| h.window().retain())
    }

    /// Runs `f` on the app, then opens or closes window hosts and wakes the clocks as a frame
    /// would. Panics when called from inside the app (a handler or a frame).
    pub fn update<R>(&self, f: impl FnOnce(&mut App) -> R) -> R {
        let result = f(&mut self.0.app.borrow_mut());
        self.0.sync_windows();
        self.0.kick();
        result
    }

    /// Runs AppKit's event loop. Never returns; AppKit exits the process when the last window
    /// closes.
    pub fn run(self) -> ! {
        self.0.platform.run();
        std::process::exit(0)
    }
}

/// Builds the app with `init` (which opens its windows) and runs AppKit's event loop. Never
/// returns.
pub fn run(init: impl FnOnce(&mut App)) -> ! {
    run_with(init, |_| {})
}

/// As [`run`], handing `started` an [`AppSender`] once the windows are up and before the event
/// loop starts.
pub fn run_with(init: impl FnOnce(&mut App), started: impl FnOnce(AppSender)) -> ! {
    let host = AppHost::new(init);
    started(host.sender());
    host.run()
}
