//! OS services for the app (`nd-system`): dialogs, clipboard, notifications, recent documents,
//! credentials, shell helpers, directories, the tray, the media session, audio, and the OS's
//! events (activation, open URL and file, file drops with the window they landed on).
//!
//! `nd` wires it into the run: the app is named from its manifest, the window hosts report
//! file drops and (on Linux and Windows) activation, and a launch's command line becomes open
//! requests. Calls return futures; [`spawn`] runs one and hands its output back to the app on
//! the main thread, and [`on_event`] delivers OS events there. A window's own alerts and pickers (a sheet on macOS) are in [`window`].

use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::future::Future;
use std::path::Path;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

pub use nd_system::*;
use nd_ui::App;

type Job = Box<dyn FnOnce(&mut App) + Send>;
type JobSender = Box<dyn Fn(Job) + Send + Sync>;

/// Where jobs go once a host runs; until then they wait in `EARLY`.
static JOBS: OnceLock<JobSender> = OnceLock::new();
static EARLY: Mutex<Vec<Job>> = Mutex::new(Vec::new());

fn post(job: Job) {
    match JOBS.get() {
        Some(send) => send(job),
        None => EARLY.lock().expect("early jobs").push(job),
    }
}

type Done = Box<dyn FnOnce(Box<dyn Any + Send>, &mut App)>;
type Handler = Rc<RefCell<dyn FnMut(&SystemEvent, &mut App)>>;

thread_local! {
    /// Main-thread continuations of [`spawn`], by key; they may hold entities, which are not
    /// `Send`, so only the key and the output cross threads.
    static PENDING: RefCell<HashMap<u64, Done>> = RefCell::new(HashMap::new());
    static HANDLERS: RefCell<Vec<Handler>> = const { RefCell::new(Vec::new()) };
}

static NEXT_KEY: AtomicU64 = AtomicU64::new(1);

/// The app's OS services.
pub fn system() -> System {
    System::native()
}

/// Runs `future` off the main thread, then `done` with its output and the app on the main
/// thread. Call it on the main thread (from a handler); `done` may hold entities. Nothing runs
/// `done` while no host runs (a tier 1 test without one).
pub fn spawn<T: Send + 'static>(
    future: impl Future<Output = T> + Send + 'static,
    done: impl FnOnce(T, &mut App) + 'static,
) {
    let key = NEXT_KEY.fetch_add(1, Ordering::Relaxed);
    let done: Done = Box::new(move |value, app| {
        if let Ok(value) = value.downcast::<T>() {
            done(*value, app);
        }
    });
    PENDING.with(|p| p.borrow_mut().insert(key, done));
    spawn_detached(async move {
        let value: Box<dyn Any + Send> = Box::new(future.await);
        post(Box::new(move |app| {
            if let Some(done) = PENDING.with(|p| p.borrow_mut().remove(&key)) {
                done(value, app);
            }
        }));
    });
}

/// Calls `handler` on the main thread with every event `system` reports from now on (open
/// requests from the launch included, when this is the first subscription). Call it on the
/// main thread; every handler sees the events of the first subscription's system.
pub fn on_event(system: &System, handler: impl FnMut(&SystemEvent, &mut App) + 'static) {
    let first = HANDLERS.with(|h| {
        let mut handlers = h.borrow_mut();
        handlers.push(Rc::new(RefCell::new(handler)));
        handlers.len() == 1
    });
    if !first {
        return;
    }
    let mut events = system.events();
    spawn_detached(async move {
        while let Some(event) = events.next().await {
            post(Box::new(move |app| {
                let handlers = HANDLERS.with(|h| h.borrow().clone());
                for handler in handlers {
                    (handler.borrow_mut())(&event, app);
                }
            }));
        }
    });
}

/// Runs the [`spawn`] continuations and event deliveries that are waiting for a host, now, on
/// this thread. For tier 1 tests, which have no host; a running app never needs it.
pub fn run_pending(app: &mut App) {
    let jobs = std::mem::take(&mut *EARLY.lock().expect("early jobs"));
    for job in jobs {
        job(app);
    }
}

/// Opens `url` in the default browser or the app registered for its scheme.
pub fn open_url(url: &str) -> Result<()> {
    system().shell().open_url(url)
}

/// Opens `path` in its default app.
#[nd_macros::unavailable(web)]
pub fn open_path(path: impl AsRef<Path>) -> Result<()> {
    system().shell().open_path(path)
}

/// Shows `path` selected in Finder or the file manager.
#[nd_macros::unavailable(web)]
pub fn reveal_path(path: impl AsRef<Path>) -> Result<()> {
    system().shell().reveal_path(path)
}

/// The app's data, config, cache and logs directories, named after its package.
pub fn app_dirs() -> AppDirs {
    AppDirs(app_identity().name.clone())
}

pub struct AppDirs(String);

impl AppDirs {
    pub fn data(&self) -> std::path::PathBuf {
        dirs::data_dir(&self.0)
    }

    pub fn config(&self) -> std::path::PathBuf {
        dirs::config_dir(&self.0)
    }

    pub fn cache(&self) -> std::path::PathBuf {
        dirs::cache_dir(&self.0)
    }

    pub fn logs(&self) -> std::path::PathBuf {
        dirs::logs_dir(&self.0)
    }

    /// [`AppDirs::data`], created if missing.
    #[nd_macros::unavailable(web)]
    pub fn ensure_data(&self) -> std::io::Result<std::path::PathBuf> {
        dirs::ensure_data_dir(&self.0)
    }

    /// [`AppDirs::cache`], created if missing.
    #[nd_macros::unavailable(web)]
    pub fn ensure_cache(&self) -> std::io::Result<std::path::PathBuf> {
        dirs::ensure_cache_dir(&self.0)
    }
}

/// Names the app before anything asks for the native system.
pub(crate) fn identify(manifest: &crate::Manifest) {
    set_app_identity(AppIdentity {
        id: manifest.id.map_or_else(|| format!("dev.nd.{}", manifest.name), str::to_owned),
        name: manifest.name.to_owned(),
        display_name: manifest.display_name.unwrap_or(manifest.name).to_owned(),
    });
}

/// Connects the running host: jobs for [`spawn`] and [`on_event`], and the opens only a
/// command line carries.
pub(crate) fn connect(send: impl Fn(Job) + Send + Sync + 'static) {
    let _ = JOBS.set(Box::new(send));
    for job in std::mem::take(&mut *EARLY.lock().expect("early jobs")) {
        post(job);
    }
    let system = system();
    // macOS delivers opens as Apple events; elsewhere they arrive on the command line.
    if !cfg!(target_os = "macos") {
        system.open_from_args(std::env::args_os().skip(1));
    }
}

/// Reports file drops from the window host with the window they landed on.
pub(crate) fn file_dropped(window: WindowId, paths: Vec<std::path::PathBuf>, at: (f32, f32)) {
    system().emit(SystemEvent::FileDrop { window, paths, position: Some(at) });
}

/// A window's own alerts and file pickers, as futures: a sheet on the window on macOS, the
/// platform's dialog parented to it elsewhere, drawn sheets on the web. One per window at a
/// time, as the platforms allow.
pub mod window {
    use std::future::Future;
    use std::path::PathBuf;

    use futures_channel::oneshot;
    use nd_components::{Alert, AlertResponse, OpenDialog, SaveDialog};
    use nd_core::WindowId;
    use nd_ui::App;

    type Callback<T> = Box<dyn FnOnce(T, &mut App)>;

    /// A callback for the chrome services and the future it resolves; `cancelled` when the
    /// service drops the callback unanswered.
    fn bridge<T: Send + 'static>(cancelled: T) -> (Callback<T>, impl Future<Output = T> + Send + 'static) {
        let (tx, rx) = oneshot::channel();
        let done: Callback<T> = Box::new(move |value, _app| {
            let _ = tx.send(value);
        });
        (done, async move { rx.await.unwrap_or(cancelled) })
    }

    pub fn alert(cx: &mut App, window: WindowId, alert: Alert) -> impl Future<Output = AlertResponse> + Send + 'static {
        let (done, answer) = bridge(AlertResponse(0));
        nd_components::services::alert(cx, window, alert, done);
        answer
    }

    pub fn open(
        cx: &mut App,
        window: WindowId,
        dialog: OpenDialog,
    ) -> impl Future<Output = Option<Vec<PathBuf>>> + Send + 'static {
        let (done, answer) = bridge(None);
        nd_components::services::open_dialog(cx, window, dialog, done);
        answer
    }

    pub fn save(
        cx: &mut App,
        window: WindowId,
        dialog: SaveDialog,
    ) -> impl Future<Output = Option<PathBuf>> + Send + 'static {
        let (done, answer) = bridge(None);
        nd_components::services::save_dialog(cx, window, dialog, done);
        answer
    }
}
