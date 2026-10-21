//! The macOS side: finding and loading the framework, the settings that point Chromium at it
//! and at the helper bundles, and the AppKit view, menus and sheets a page uses.

pub(crate) mod chrome;
pub(crate) mod dialogs;
pub(crate) mod install_prompt;
pub(crate) mod keys;
pub(crate) mod menu;
mod paint;
pub(crate) mod pump;
pub(crate) mod view;

use std::cell::Cell;
use std::ffi::CString;
use std::path::{Path, PathBuf};

use cef::*;

const FRAMEWORK: &str = "Chromium Embedded Framework.framework";

/// The app bundle's `Contents`, when the executable runs inside one.
pub(crate) fn bundle_contents() -> Option<PathBuf> {
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

const MISSING: &str = "the Chromium Embedded Framework is missing: package with `[webview] engine = \"cef\"`, or set ND_CEF_FRAMEWORK_DIR";

thread_local! {
    static LOADED: Cell<bool> = const { Cell::new(false) };
}

/// Loads the framework into this process, once.
pub(crate) fn load() -> Result<(), String> {
    if LOADED.with(Cell::get) {
        return Ok(());
    }
    let framework = framework_dir().ok_or(MISSING)?;
    let binary = framework.join("Chromium Embedded Framework");
    let path = CString::new(binary.as_os_str().as_encoded_bytes()).map_err(|e| e.to_string())?;
    // SAFETY: `path` is a NUL-terminated string that outlives the call.
    if load_library(Some(unsafe { &*path.as_ptr() })) != 1 {
        return Err(format!("could not load {}", framework.display()));
    }
    let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);
    LOADED.with(|l| l.set(true));
    Ok(())
}

/// Where the framework and the helpers are, for `CefInitialize`.
pub(crate) fn settings() -> Result<Settings, String> {
    let framework = framework_dir().ok_or(MISSING)?;
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
    let s = |p: &Path| CefString::from(p.to_string_lossy().as_ref());
    Ok(Settings {
        framework_dir_path: s(&framework),
        browser_subprocess_path: s(&subprocess),
        main_bundle_path: main_bundle.as_deref().map(s).unwrap_or_default(),
        ..Settings::default()
    })
}

/// Switches for the browser process.
pub(crate) fn command_line(line: &mut CommandLine) {
    // An unsigned dev build would otherwise ask for the login keychain on every launch
    // (Chromium's cookie encryption key).
    if bundle_contents().is_none() || std::env::var_os("ND_CEF_MOCK_KEYCHAIN").is_some() {
        line.append_switch(Some(&CefString::from("use-mock-keychain")));
    }
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

/// Runs `f` on the main thread, from any thread.
pub(crate) fn on_main(f: impl FnOnce() + Send + 'static) {
    dispatch2::DispatchQueue::main().exec_async(f);
}
