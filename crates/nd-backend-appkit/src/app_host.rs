//! Runs an `nd-ui` [`App`] on AppKit: one [`WindowHost`] per app window, frames inside AppKit's
//! layout pass (so a live resize presents the frame it laid out), the display clock waking
//! layout while the app has work, and native events drained on the run loop turn after they
//! were queued.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, VecDeque};
use std::rc::{Rc, Weak};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use nd_core::{FrameTime, WindowId};
use nd_diag::{FrameRecord, Watchdog};
use nd_ui::App;
use objc2_foundation::MainThreadMarker;

use crate::{AppKitBackend, NdAppKitPlatform, WindowHost};

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
            let ns_window = self.platform.open_window(window.title(), size.width.into(), size.height.into());
            let host = WindowHost::new(&self.backend, id, &ns_window);
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
            ns_window.center();
            self.platform.show(&ns_window);
            windows.insert(id, host);
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

/// Work for the main thread that needs the app, sent from any thread.
pub type AppJob = Box<dyn FnOnce(&mut App) + Send>;

type Jobs = Arc<Mutex<VecDeque<AppJob>>>;

thread_local! {
    static HOST: RefCell<Weak<Host>> = const { RefCell::new(Weak::new()) };
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

/// Builds the app with `init` (which opens its windows) and runs AppKit's event loop. Never
/// returns; AppKit exits the process when the last window closes.
pub fn run(init: impl FnOnce(&mut App)) -> ! {
    run_with(init, |_| {})
}

/// As [`run`], handing `started` an [`AppSender`] once the windows are up and before the event
/// loop starts.
pub fn run_with(init: impl FnOnce(&mut App), started: impl FnOnce(AppSender)) -> ! {
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
    started(AppSender { jobs: Jobs::default() });
    host.platform.run();
    std::process::exit(0)
}
