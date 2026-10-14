//! The macOS runtime: finding and loading the framework, initializing Chromium on the app's
//! run loop, request contexts per profile, and ordered shutdown.

mod a11y;
mod client;
mod devtools;
mod keys;
mod menu;
mod page;
mod paint;
mod pump;
mod scheme;
mod view;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::CString;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use cef::*;
use nd_webview::{Engine, EngineFactory, EngineView, Page, Profile};

const FRAMEWORK: &str = "Chromium Embedded Framework.framework";
/// Carries the app's registered schemes to subprocesses, which must register the same set.
const SCHEMES_SWITCH: &str = "nd-schemes";

thread_local! {
    static APP_NAME: RefCell<String> = RefCell::default();
    static STARTED: Cell<Option<bool>> = const { Cell::new(None) };
    static CONTEXTS: RefCell<HashMap<Profile, RequestContext>> = RefCell::default();
    /// Kept alive for as long as Chromium runs.
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

pub(crate) fn register(app_name: &str) {
    if let Some(code) = subprocess_from_args() {
        std::process::exit(code);
    }
    APP_NAME.with(|n| *n.borrow_mut() = app_name.to_owned());
    nd_webview::register_engine(Engine::Cef, Factory);
}

/// A dev run is its own helper: Chromium launches the app's executable with `--type=...`.
fn subprocess_from_args() -> Option<i32> {
    std::env::args().skip(1).any(|a| a.starts_with("--type=")).then(run_helper)
}

pub(crate) fn run_helper() -> i32 {
    let Some(framework) = framework_dir() else {
        eprintln!("nd-cef: Chromium Embedded Framework not found");
        return 1;
    };
    if !load(&framework) {
        eprintln!("nd-cef: could not load {}", framework.display());
        return 1;
    }
    let args = cef::args::Args::new();
    let mut app = client::app(schemes_from_args());
    execute_process(Some(args.as_main_args()), Some(&mut app), std::ptr::null_mut())
}

fn schemes_from_args() -> Vec<String> {
    let prefix = format!("--{SCHEMES_SWITCH}=");
    std::env::args()
        .find_map(|a| a.strip_prefix(&prefix).map(str::to_owned))
        .map(|v| v.split(',').filter(|s| !s.is_empty()).map(str::to_owned).collect())
        .unwrap_or_default()
}

/// The app bundle's `Contents`, when the executable runs inside one.
fn bundle_contents() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    (macos.file_name()? == "MacOS" && contents.file_name()? == "Contents").then(|| contents.to_owned())
}

/// The `.framework` directory: in the bundle (the app's own, or a helper's parent app),
/// `ND_CEF_FRAMEWORK_DIR` (the framework or its parent), then where cef-dll-sys unpacked it.
fn framework_dir() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(contents) = bundle_contents() {
        candidates.push(contents.join("Frameworks").join(FRAMEWORK));
        // A helper: `<App>.app/Contents/Frameworks/<App> Helper.app/Contents/MacOS/<exe>`.
        if let Some(frameworks) = contents.parent().and_then(Path::parent) {
            candidates.push(frameworks.join(FRAMEWORK));
        }
    }
    if let Some(dir) = std::env::var_os("ND_CEF_FRAMEWORK_DIR").map(PathBuf::from) {
        candidates.push(dir.join(FRAMEWORK));
        candidates.push(dir);
    }
    if let Some(dir) = option_env!("ND_CEF_BUILD_DIR") {
        candidates.push(Path::new(dir).join(FRAMEWORK));
    }
    candidates.into_iter().find(|c| c.join("Chromium Embedded Framework").is_file())
}

fn load(framework: &Path) -> bool {
    let binary = framework.join("Chromium Embedded Framework");
    let Ok(path) = CString::new(binary.as_os_str().as_encoded_bytes()) else { return false };
    // SAFETY: `path` is a NUL-terminated string that outlives the call.
    if load_library(Some(unsafe { &*path.as_ptr() })) != 1 {
        return false;
    }
    let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);
    true
}

/// Starts Chromium on first use. `Err` says why it cannot run.
fn start() -> Result<(), String> {
    if let Some(ok) = STARTED.with(Cell::get) {
        return if ok { Ok(()) } else { Err("Chromium failed to start; see the log".into()) };
    }
    STARTED.with(|s| s.set(Some(false)));
    let framework = framework_dir().ok_or(
        "the Chromium Embedded Framework is missing: package with `[webview] engine = \"cef\"`, or set \
         ND_CEF_FRAMEWORK_DIR",
    )?;
    if !load(&framework) {
        return Err(format!("could not load {}", framework.display()));
    }
    pump::install_app_protocol();
    let (subprocess, main_bundle) = match bundle_contents() {
        Some(contents) => {
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            let name = exe.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let helper = format!("{name} Helper");
            let path = contents.join("Frameworks").join(format!("{helper}.app/Contents/MacOS/{helper}"));
            (path, contents.parent().map(Path::to_owned))
        }
        None => (std::env::current_exe().map_err(|e| e.to_string())?, None),
    };
    let root = root_cache();
    let _ = std::fs::create_dir_all(&root);
    let s = |p: &Path| CefString::from(p.to_string_lossy().as_ref());
    let settings = Settings {
        no_sandbox: 1,
        external_message_pump: 1,
        windowless_rendering_enabled: 1,
        multi_threaded_message_loop: 0,
        framework_dir_path: s(&framework),
        browser_subprocess_path: s(&subprocess),
        main_bundle_path: main_bundle.as_deref().map(s).unwrap_or_default(),
        root_cache_path: s(&root),
        cache_path: s(&root.join("Default")),
        persist_session_cookies: 1,
        log_severity: LogSeverity::WARNING,
        background_color: 0xffff_ffff,
        ..Settings::default()
    };
    let args = cef::args::Args::new();
    let mut app = client::app(nd_webview::schemes());
    if initialize(Some(args.as_main_args()), Some(&settings), Some(&mut app), std::ptr::null_mut()) != 1 {
        return Err("CefInitialize failed (another instance holding the profile, or a broken framework)".into());
    }
    APP.with(|a| *a.borrow_mut() = Some(app));
    STARTED.with(|s| s.set(Some(true)));
    pump::start();
    scheme::register_global();
    Ok(())
}

pub(crate) fn app_name() -> String {
    APP_NAME.with(|n| n.borrow().clone())
}

fn root_cache() -> PathBuf {
    if let Some(dir) = std::env::var_os("ND_CEF_CACHE") {
        return PathBuf::from(dir);
    }
    let name = app_name();
    let name = if name.is_empty() { "nd-app".to_owned() } else { name };
    nd_core::paths::app_data_dir(&name).join("cef")
}

/// The request context a profile's pages share: the global one for `Default`, an in-memory one
/// for `Private` (each private page its own), and a persistent one per name under the root.
fn context(profile: &Profile) -> Option<RequestContext> {
    match profile {
        Profile::Default => request_context_get_global_context(),
        Profile::Private => {
            let ctx = request_context_create_context(Some(&RequestContextSettings::default()), None)?;
            scheme::register_on(&ctx);
            Some(ctx)
        }
        Profile::Named(name) => CONTEXTS.with(|c| {
            if let Some(ctx) = c.borrow().get(profile) {
                return Some(ctx.clone());
            }
            let safe: String =
                name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
            let path = root_cache().join(format!("profile-{safe}"));
            let settings = RequestContextSettings {
                cache_path: CefString::from(path.to_string_lossy().as_ref()),
                persist_session_cookies: 1,
                ..RequestContextSettings::default()
            };
            let ctx = request_context_create_context(Some(&settings), None)?;
            scheme::register_on(&ctx);
            c.borrow_mut().insert(profile.clone(), ctx.clone());
            Some(ctx)
        }),
    }
}

struct Factory;

impl EngineFactory for Factory {
    fn create(&self, page: Page) -> Result<Rc<dyn EngineView>, String> {
        start()?;
        page::CefPage::create(page)
    }
}

/// Converts what CEF hands out as `CefStringUserfree` into a Rust string.
pub(crate) fn text(s: &CefStringUserfree) -> String {
    CefString::from(s).to_string()
}

/// Runs `f` on a later turn of the main run loop, in common modes, outside whatever CEF
/// callback is on the stack.
pub(crate) fn run_later(f: impl FnOnce() + 'static) {
    use block2::RcBlock;
    use objc2_foundation::{NSRunLoop, NSRunLoopCommonModes, NSTimer};
    let f = Cell::new(Some(f));
    let block = RcBlock::new(move |_timer: std::ptr::NonNull<NSTimer>| {
        if let Some(f) = f.take() {
            f();
        }
    });
    // SAFETY: added to the main run loop, the thread `f` came from.
    let timer = unsafe { NSTimer::timerWithTimeInterval_repeats_block(0., false, &block) };
    unsafe { NSRunLoop::mainRunLoop().addTimer_forMode(&timer, NSRunLoopCommonModes) };
}
