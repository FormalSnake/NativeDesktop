//! The Linux side: where `libcef.so` and its resources are, the switches Chromium needs on a
//! Wayland or X11 desktop, and the GPUI element, menus and dialogs a page uses.

pub(crate) mod dialogs;
mod keys;
pub(crate) mod pump;
pub(crate) mod view;

use std::path::{Path, PathBuf};

use cef::*;

/// What Chromium reads at startup besides `libcef.so`, all from the same directory.
const RESOURCES: [&str; 4] = ["icudtl.dat", "resources.pak", "v8_context_snapshot.bin", "locales"];

/// The directory `libcef.so` was loaded from: the executable links it, so it is mapped by the
/// time any code runs.
fn library_dir() -> Option<PathBuf> {
    let maps = std::fs::read_to_string("/proc/self/maps").ok()?;
    maps.lines()
        .filter_map(|line| line.split_whitespace().nth(5))
        .find(|path| path.ends_with("/libcef.so"))
        .and_then(|path| Path::new(path).parent().map(Path::to_owned))
}

/// Where Chromium's resources are: `ND_CEF_DIR`, else beside `libcef.so`.
fn runtime_dir() -> Result<PathBuf, String> {
    let dir = std::env::var_os("ND_CEF_DIR")
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .or_else(library_dir)
        .ok_or("libcef.so is not loaded")?;
    match RESOURCES.iter().find(|name| !dir.join(name).exists()) {
        Some(missing) => Err(format!(
            "the Chromium runtime at {} is incomplete ({missing} is missing): ship the whole CEF distribution \
             beside libcef.so, or point ND_CEF_DIR at it",
            dir.display()
        )),
        None => Ok(dir),
    }
}

/// Checks the runtime and picks the API version; `libcef.so` itself is loaded with the
/// executable. Every wrapper object reports the version set here, and Chromium aborts on the
/// first one if none was.
pub(crate) fn load() -> Result<(), String> {
    runtime_dir()?;
    let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);
    Ok(())
}

/// The resources, and the app's own executable as every subprocess.
pub(crate) fn settings() -> Result<Settings, String> {
    let dir = runtime_dir()?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let s = |p: &Path| CefString::from(p.to_string_lossy().as_ref());
    Ok(Settings {
        browser_subprocess_path: s(&exe),
        resources_dir_path: s(&dir),
        locales_dir_path: s(&dir.join("locales")),
        ..Settings::default()
    })
}

/// Switches for the browser process. Off-screen pages need no window of Chromium's, but Ozone
/// still connects to a display server: the one the app runs on, or none.
pub(crate) fn command_line(line: &mut CommandLine) {
    let platform = std::env::var("ND_CEF_OZONE").ok().filter(|p| !p.is_empty()).unwrap_or_else(|| {
        let set = |name: &str| std::env::var_os(name).is_some_and(|v| !v.is_empty());
        if set("WAYLAND_DISPLAY") {
            "wayland".into()
        } else if set("DISPLAY") {
            "x11".into()
        } else {
            "headless".into()
        }
    });
    line.append_switch_with_value(Some(&CefString::from("ozone-platform")), Some(&CefString::from(platform.as_str())));
}

/// Runs `f` on a later turn of the main loop, outside whatever CEF callback is on the stack.
pub(crate) fn run_later(f: impl FnOnce() + 'static) {
    pump::later(Box::new(f));
}

/// Runs `f` on the main thread, from any thread.
pub(crate) fn on_main(f: impl FnOnce() + Send + 'static) {
    pump::send(pump::Msg::Run(Box::new(f)));
}
