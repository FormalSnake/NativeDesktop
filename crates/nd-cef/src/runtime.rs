//! Chromium's lifetime in the app: subprocesses, initializing on the app's main loop, request
//! contexts per profile. What differs per OS (finding the runtime, the settings that point at
//! it, the message pump) is in `platform`.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use cef::*;
use nd_webview::{Engine, EngineFactory, EngineView, Page, Profile};

use crate::platform;

/// Carries the app's registered schemes to subprocesses, which must register the same set.
pub(crate) const SCHEMES_SWITCH: &str = "nd-schemes";

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
    // No OS engine embeds in a GPUI window on Linux: Chromium is the system engine there.
    #[cfg(target_os = "linux")]
    nd_webview::register_engine(Engine::System, Factory);
}

/// A dev run is its own helper: Chromium launches the app's executable with `--type=...`.
fn subprocess_from_args() -> Option<i32> {
    std::env::args().skip(1).any(|a| a.starts_with("--type=")).then(run_helper)
}

pub(crate) fn run_helper() -> i32 {
    if let Err(error) = platform::load() {
        eprintln!("nd-cef: {error}");
        return 1;
    }
    let args = cef::args::Args::new();
    let mut app = crate::client::app(schemes_from_args());
    execute_process(Some(args.as_main_args()), Some(&mut app), std::ptr::null_mut())
}

fn schemes_from_args() -> Vec<String> {
    let prefix = format!("--{SCHEMES_SWITCH}=");
    std::env::args()
        .find_map(|a| a.strip_prefix(&prefix).map(str::to_owned))
        .map(|v| v.split(',').filter(|s| !s.is_empty()).map(str::to_owned).collect())
        .unwrap_or_default()
}

/// Starts Chromium on first use. `Err` says why it cannot run.
fn start() -> Result<(), String> {
    if let Some(ok) = STARTED.with(Cell::get) {
        return if ok { Ok(()) } else { Err("Chromium failed to start; see the log".into()) };
    }
    STARTED.with(|s| s.set(Some(false)));
    platform::load()?;
    platform::pump::prepare();
    let root = root_cache();
    let _ = std::fs::create_dir_all(&root);
    let s = |p: &std::path::Path| CefString::from(p.to_string_lossy().as_ref());
    let settings = Settings {
        no_sandbox: 1,
        external_message_pump: 1,
        windowless_rendering_enabled: 1,
        multi_threaded_message_loop: 0,
        root_cache_path: s(&root),
        cache_path: s(&root.join("Default")),
        persist_session_cookies: 1,
        log_severity: LogSeverity::WARNING,
        background_color: 0xffff_ffff,
        ..platform::settings()?
    };
    let args = cef::args::Args::new();
    let mut app = crate::client::app(nd_webview::schemes());
    if initialize(Some(args.as_main_args()), Some(&settings), Some(&mut app), std::ptr::null_mut()) != 1 {
        return Err("CefInitialize failed (another instance holding the profile, or a broken runtime)".into());
    }
    APP.with(|a| *a.borrow_mut() = Some(app));
    STARTED.with(|s| s.set(Some(true)));
    platform::pump::start();
    crate::scheme::register_global();
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
pub(crate) fn context(profile: &Profile) -> Option<RequestContext> {
    match profile {
        Profile::Default => request_context_get_global_context(),
        Profile::Private => {
            let ctx = request_context_create_context(Some(&RequestContextSettings::default()), None)?;
            crate::scheme::register_on(&ctx);
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
            crate::scheme::register_on(&ctx);
            c.borrow_mut().insert(profile.clone(), ctx.clone());
            Some(ctx)
        }),
    }
}

struct Factory;

impl EngineFactory for Factory {
    fn create(&self, page: Page) -> Result<Rc<dyn EngineView>, String> {
        start()?;
        crate::page::CefPage::create(page)
    }
}

/// Converts what CEF hands out as `CefStringUserfree` into a Rust string.
pub(crate) fn text(s: &CefStringUserfree) -> String {
    CefString::from(s).to_string()
}
