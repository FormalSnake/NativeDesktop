//! Diagnostics every nd app gets without code (ARCHITECTURE.md 9): tracing into an in-memory
//! ring and rolling log files, a flight recorder of inputs, frame timings, trees and layout
//! diffs, incident bundles on panic, native crash, hang or `report_issue`, retention, and tracy
//! on demand behind the `tracy` feature.
//!
//! Layout under the app data dir (`ND_APP_DATA_DIR` overrides it):
//! `logs/<app>.log` (10 MB rolls, 7 kept, gzipped, deleted after 7 days) and
//! `diagnostics/<utc timestamp>-<kind>/` (newest 20, 14 days, 200 MB).
//!
//! Environment: `ND_LOG` (an `EnvFilter`, default `info`), `ND_DIAG_HANG_MS` (default 5000),
//! `ND_DIAG_CRASH_HANDLER=0` (no crash helper process), `ND_DIAG_KEEP_TEXT=1` (keep typed text
//! in release builds).
//!
//! On the web only the panic hook is installed (to the browser console); the rest needs files
//! and threads a tab does not have.
#![cfg_attr(target_arch = "wasm32", allow(dead_code, unused_imports))]

mod bundle;
#[cfg(not(target_arch = "wasm32"))]
mod crash;
#[cfg(target_arch = "wasm32")]
#[path = "crash_web.rs"]
mod crash;
mod hang;
mod log;
mod record;

use std::cell::RefCell;
use std::collections::VecDeque;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use nd_core::semantic::SemanticWindow;
use serde_json::{Value, json};
use tracing_subscriber::layer::SubscriberExt;

pub use bundle::{BundleInfo, Kind, Retention, Snapshot, enforce_retention, list_bundles};
pub use hang::{PostToMain, Watchdog};
pub use record::{FrameRecord, FrameSummary, InputRecord};

const LOG_KEEP: u64 = 7;
const LOG_MAX_AGE: Duration = Duration::from_secs(7 * 24 * 3600);
/// How often the crash helper's snapshot is refreshed while something changed.
const SNAPSHOT_EVERY: Duration = Duration::from_secs(1);

#[derive(Clone, Debug)]
pub struct Config {
    pub app: String,
    pub version: String,
    /// `appkit`, `gpui`, `test`.
    pub backend: &'static str,
    pub data_dir: PathBuf,
    pub hang_threshold: Duration,
    pub crash_handler: bool,
}

impl Config {
    pub fn new(app: &str, version: &str, backend: &'static str) -> Self {
        let env_ms = std::env::var("ND_DIAG_HANG_MS").ok().and_then(|v| v.parse().ok());
        Self {
            app: app.to_owned(),
            version: version.to_owned(),
            backend,
            data_dir: nd_core::paths::app_data_dir(app),
            hang_threshold: Duration::from_millis(env_ms.unwrap_or(5000)),
            crash_handler: std::env::var_os("ND_DIAG_CRASH_HANDLER").is_none_or(|v| v != "0"),
        }
    }
}

/// Where the app named `app` keeps its incident bundles: what [`Diag::new`] writes into.
pub fn diagnostics_dir(app: &str) -> PathBuf {
    nd_core::paths::app_data_dir(app).join("diagnostics")
}

pub(crate) struct Shared {
    config: Config,
    diagnostics: PathBuf,
    logs: PathBuf,
    ring: log::Ring,
    recorder: Mutex<record::Recorder>,
    /// Bumped on every record, so the snapshot writer skips idle periods.
    generation: AtomicU64,
    crash: OnceLock<crash::CrashClient>,
    cleaned: AtomicBool,
}

thread_local! {
    static NATIVE_TREE: RefCell<Option<Box<dyn Fn() -> Value>>> = const { RefCell::new(None) };
}

/// Sets the backend's native view hierarchy dump for this thread (the main thread). Bundles
/// written on this thread include it.
pub fn set_native_tree(dump: impl Fn() -> Value + 'static) {
    NATIVE_TREE.with(|n| *n.borrow_mut() = Some(Box::new(dump)));
}

fn redact(text: String) -> String {
    let keep = cfg!(debug_assertions) || std::env::var_os("ND_DIAG_KEEP_TEXT").is_some_and(|v| v == "1");
    if keep { text } else { "\u{2022}".repeat(text.chars().count()) }
}

impl Shared {
    fn touch(&self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
    }

    fn recorder(&self) -> std::sync::MutexGuard<'_, record::Recorder> {
        self.recorder.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn meta(&self, kind: Option<Kind>) -> Value {
        json!({
            "app": self.config.app,
            "version": self.config.version,
            "sha": option_env!("ND_GIT_SHA"),
            "kind": kind.map(Kind::as_str),
            "time": chrono::Utc::now().to_rfc3339(),
            "backend": self.config.backend,
            "os": std::env::consts::OS,
            "os_version": os_version(),
            "arch": std::env::consts::ARCH,
            "pid": std::process::id(),
            "debug": cfg!(debug_assertions),
        })
    }

    fn snapshot(&self, kind: Option<Kind>) -> Snapshot {
        let native = NATIVE_TREE.with(|n| n.try_borrow().ok().and_then(|n| n.as_ref().map(|dump| dump())));
        let log = self.ring.lock().unwrap_or_else(|e| e.into_inner()).iter().cloned().collect();
        let recorder = self.recorder();
        Snapshot {
            meta: self.meta(kind),
            log,
            trees: recorder.trees(),
            native,
            inputs: recorder.inputs(),
            frames: recorder.frames(),
            layout: recorder.layout(),
        }
    }

    /// Writes a bundle: `primary` under `name`, the snapshot's files, and a minidump when the
    /// crash helper runs. Applies retention afterwards.
    fn write_incident(&self, kind: Kind, name: &str, primary: &[u8]) -> io::Result<PathBuf> {
        let dir = bundle::create_dir(&self.diagnostics, kind)?;
        fs::write(dir.join(name), primary)?;
        bundle::write_snapshot(&dir, &self.snapshot(Some(kind)))?;
        if let Some(crash) = self.crash.get() {
            crash.dump_into(&dir);
        }
        bundle::enforce_retention(&self.diagnostics, Retention::default())?;
        Ok(dir)
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn write_crash_snapshot(&self) -> io::Result<()> {
        let path = self.diagnostics.join(crash::SNAPSHOT);
        let tmp = path.with_extension("tmp");
        fs::write(&tmp, serde_json::to_vec(&self.snapshot(Some(Kind::Crash)))?)?;
        fs::rename(tmp, path)
    }
}

fn os_version() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        let mut buf = [0u8; 64];
        let mut len = buf.len();
        // SAFETY: the name is NUL-terminated and `len` is the buffer's size.
        let ok = unsafe {
            libc::sysctlbyname(
                c"kern.osproductversion".as_ptr(),
                buf.as_mut_ptr().cast(),
                &mut len,
                std::ptr::null_mut(),
                0,
            )
        } == 0;
        ok.then(|| String::from_utf8_lossy(&buf[..len.saturating_sub(1)]).into_owned())
    }
    #[cfg(not(target_os = "macos"))]
    None
}

/// The app's diagnostics. Cheap to clone.
#[derive(Clone)]
pub struct Diag(Arc<Shared>);

static GLOBAL: OnceLock<Diag> = OnceLock::new();

/// The diagnostics [`init`] installed, if any.
pub fn global() -> Option<&'static Diag> {
    GLOBAL.get()
}

/// Sets up diagnostics for the app process: the tracing subscriber, the panic hook, the crash
/// helper and the global [`Diag`]. When this process was launched as the crash helper it runs
/// the helper and exits instead. Platform hosts call it first thing.
pub fn init(config: Config) -> Option<&'static Diag> {
    #[cfg(target_arch = "wasm32")]
    {
        // No files, processes or threads in a browser tab: panics go to the console, which is
        // where a web app's diagnostics live.
        let _ = config;
        console_error_panic_hook::set_once();
        None
    }
    #[cfg(not(target_arch = "wasm32"))]
    init_native(config)
}

#[cfg(not(target_arch = "wasm32"))]
fn init_native(config: Config) -> Option<&'static Diag> {
    let args: Vec<_> = std::env::args_os().collect();
    if args.get(1).is_some_and(|a| a == crash::ARG) && args.len() == 4 {
        crash::run_server(Path::new(&args[2]), Path::new(&args[3]));
        std::process::exit(0);
    }
    if let Some(diag) = GLOBAL.get() {
        return Some(diag);
    }
    let diag = match Diag::new(config) {
        Ok(diag) => diag,
        Err(e) => {
            eprintln!("nd: diagnostics disabled: {e}");
            return None;
        }
    };
    let file =
        logroller::LogRollerBuilder::new(diag.0.logs.as_path(), Path::new(&format!("{}.log", diag.0.config.app)))
            .rotation(logroller::Rotation::SizeBased(logroller::RotationSize::MB(10)))
            .max_keep_files(LOG_KEEP)
            .compression(logroller::Compression::Gzip)
            .build()
            .map_err(|e| eprintln!("nd: log file disabled: {e}"))
            .ok()
            .map(Mutex::new);
    let filter = tracing_subscriber::EnvFilter::try_from_env("ND_LOG")
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    let stderr = cfg!(debug_assertions).then(|| tracing_subscriber::fmt::layer().with_writer(io::stderr));
    #[cfg(feature = "tracy")]
    let tracy = Some(tracing_tracy::TracyLayer::default());
    #[cfg(not(feature = "tracy"))]
    let tracy: Option<tracing_subscriber::layer::Identity> = None;
    let subscriber = tracing_subscriber::registry()
        .with(filter)
        .with(log::RecorderLayer { ring: diag.0.ring.clone(), file })
        .with(stderr)
        .with(tracy);
    if tracing::subscriber::set_global_default(subscriber).is_err() {
        eprintln!("nd: a tracing subscriber was already set; logs go there");
    }
    let diag = GLOBAL.get_or_init(|| diag);
    diag.install_panic_hook();
    if diag.0.config.crash_handler {
        let shared = Arc::downgrade(&diag.0);
        std::thread::Builder::new()
            .name("nd-crash-snapshot".into())
            .spawn(move || {
                let Some(dir) = shared.upgrade().map(|s| s.diagnostics.clone()) else { return };
                match crash::CrashClient::start(&dir) {
                    Ok(client) => {
                        if let Some(s) = shared.upgrade() {
                            let _ = s.crash.set(client);
                        }
                    }
                    Err(e) => tracing::warn!("crash helper unavailable: {e}"),
                }
                let mut written = u64::MAX;
                while let Some(s) = shared.upgrade() {
                    let generation = s.generation.load(Ordering::Relaxed);
                    if generation != written && s.write_crash_snapshot().is_ok() {
                        written = generation;
                    }
                    drop(s);
                    std::thread::sleep(SNAPSHOT_EVERY);
                }
            })
            .ok()?;
    }
    tracing::info!(app = %diag.0.config.app, version = %diag.0.config.version, "diagnostics ready");
    Some(diag)
}

impl Diag {
    /// Diagnostics rooted at `config.data_dir`, with nothing installed process-wide. Tests and
    /// [`init`] build on it.
    pub fn new(config: Config) -> io::Result<Self> {
        let diagnostics = config.data_dir.join("diagnostics");
        let logs = config.data_dir.join("logs");
        fs::create_dir_all(&diagnostics)?;
        fs::create_dir_all(&logs)?;
        Ok(Self(Arc::new(Shared {
            config,
            diagnostics,
            logs,
            ring: Arc::new(Mutex::new(VecDeque::new())),
            recorder: Mutex::default(),
            generation: AtomicU64::new(0),
            crash: OnceLock::new(),
            cleaned: AtomicBool::new(false),
        })))
    }

    pub fn diagnostics_dir(&self) -> &Path {
        &self.0.diagnostics
    }

    pub fn logs_dir(&self) -> &Path {
        &self.0.logs
    }

    /// Adds a structured line to the log ring directly, for events from outside `tracing`.
    pub fn log(&self, line: Value) {
        log::push(&self.0.ring, line);
        self.0.touch();
    }

    pub fn record_input(&self, mut input: InputRecord) {
        input.text = input.text.map(redact);
        self.0.recorder().input(input);
        self.0.touch();
    }

    /// Inputs recorded so far, up to the ring's capacity.
    pub fn input_count(&self) -> usize {
        self.0.recorder().input_count()
    }

    /// A window committed a frame. `tree` is its semantic tree as committed, kept for
    /// `tree.json` and diffed against the last one for `frames/layout.ndjson`.
    pub fn record_frame(&self, window: u64, frame: FrameRecord, tree: Option<&SemanticWindow>) {
        self.0.recorder().frame(window, frame, tree);
        self.0.touch();
    }

    /// The window's recorded frames, oldest first, as many as the recorder keeps.
    pub fn window_frames(&self, window: u64) -> Vec<FrameRecord> {
        self.0.recorder().window_frames(window)
    }

    pub fn close_window(&self, window: u64) {
        self.0.recorder().close_window(window);
        self.0.touch();
    }

    pub fn snapshot(&self) -> Snapshot {
        self.0.snapshot(None)
    }

    /// Writes an incident bundle. `nd::report_issue()` and the hooks below call it.
    pub fn write_incident(&self, kind: Kind, name: &str, primary: &[u8]) -> io::Result<PathBuf> {
        self.0.write_incident(kind, name, primary)
    }

    /// A user or app reported a problem: a `-report` bundle with `message` in `report.txt`.
    pub fn report_issue(&self, message: &str) -> io::Result<PathBuf> {
        self.write_incident(Kind::Report, "report.txt", message.as_bytes())
    }

    /// Starts watching the main thread. `post` runs a closure on it.
    pub fn start_watchdog(&self, post: PostToMain) -> Watchdog {
        hang::start(Arc::downgrade(&self.0), post, self.0.config.hang_threshold)
    }

    /// Runs bundle and log retention on a background thread, once per process. Hosts call it
    /// after the first frame.
    pub fn clean_up_later(&self) {
        if self.0.cleaned.swap(true, Ordering::SeqCst) {
            return;
        }
        let shared = self.0.clone();
        let _ = std::thread::Builder::new().name("nd-diag-cleanup".into()).spawn(move || {
            if let Err(e) = bundle::enforce_retention(&shared.diagnostics, Retention::default()) {
                tracing::warn!("bundle retention: {e}");
            }
            let live = format!("{}.log", shared.config.app);
            if let Err(e) = bundle::clean_logs(&shared.logs, &live, LOG_MAX_AGE) {
                tracing::warn!("log retention: {e}");
            }
        });
    }

    /// On panic: logs it, writes a `-panic` bundle (`panic.txt` with the backtrace, plus a
    /// minidump when the crash helper runs) and exits with status 101. One process holds the
    /// UI, so a panic anywhere ends the app; the bundle is what survives it.
    pub fn install_panic_hook(&'static self) {
        static PANICKING: AtomicBool = AtomicBool::new(false);
        std::panic::set_hook(Box::new(move |info| {
            if PANICKING.swap(true, Ordering::SeqCst) {
                return;
            }
            let thread = std::thread::current();
            let message = info.payload_as_str().unwrap_or("Box<dyn Any>");
            let location = info.location().map_or_else(|| "<unknown>".into(), ToString::to_string);
            tracing::error!(thread = thread.name().unwrap_or("<unnamed>"), %location, "panic: {message}");
            let text = format!(
                "thread '{}' panicked at {location}:\n{message}\n\n{}\n",
                thread.name().unwrap_or("<unnamed>"),
                std::backtrace::Backtrace::force_capture()
            );
            eprint!("{text}");
            match self.write_incident(Kind::Panic, "panic.txt", text.as_bytes()) {
                Ok(dir) => eprintln!("nd: incident bundle {}", dir.display()),
                Err(e) => eprintln!("nd: incident bundle failed: {e}"),
            }
            // `_exit`, not `exit`: exit runs atexit handlers and C++ static destructors, the GPU
            // driver's among them, while the main thread keeps drawing. A panic on a worker
            // thread then dies as a SIGSEGV inside vkAcquireNextImageKHR with no status 101.
            #[cfg(not(target_arch = "wasm32"))]
            unsafe {
                libc::_exit(101)
            };
            #[cfg(target_arch = "wasm32")]
            std::process::exit(101);
        }));
    }
}
