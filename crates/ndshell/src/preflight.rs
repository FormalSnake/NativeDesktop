//! Checks a tier 3 test runs before posting real input, each failing with what to do about it
//! instead of a test that silently sees no clicks.

use std::process::Command;

use crate::input;

/// Window managers that grab synthetic clicks and drags, by executable name.
const INTERCEPTING_WMS: [&str; 4] = ["OmniWM", "yabai", "AeroSpace", "Amethyst"];

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGPreflightPostEventAccess() -> bool;
}

/// The running window managers from [`INTERCEPTING_WMS`].
pub fn intercepting_window_managers() -> Vec<&'static str> {
    let Ok(out) = Command::new("/bin/ps").args(["-axco", "comm="]).output() else { return Vec::new() };
    let names = String::from_utf8_lossy(&out.stdout).to_lowercase();
    let running: Vec<&str> = names.lines().map(str::trim).collect();
    INTERCEPTING_WMS.into_iter().filter(|wm| running.contains(&wm.to_lowercase().as_str())).collect()
}

/// Ready to post real input: no intercepting window manager, the right to post events, and no
/// modifier left held (released here first, since an earlier run that crashed mid-chord leaves
/// one held machine-wide).
pub fn real_input() -> Result<(), String> {
    let wms = intercepting_window_managers();
    if !wms.is_empty() {
        return Err(format!(
            "{} is running and intercepts synthetic clicks and drags; quit it before running real-input tests",
            wms.join(", ")
        ));
    }
    if !unsafe { CGPreflightPostEventAccess() } {
        return Err("this process may not post events: grant Accessibility to the terminal or ssh session that \
                    launched it (System Settings, or `ndshell grant <its binary>` with SIP off)"
            .into());
    }
    input::release_modifiers();
    let held = input::held_modifiers();
    if !held.is_empty() {
        return Err(format!("modifier keys still held after releasing them: {held:?}; let go of the keyboard"));
    }
    Ok(())
}

/// [`real_input`] under the machine-wide lock: waits for any other real-input run, here or in
/// another worktree, then checks. Hold the returned lock for as long as the test posts input.
pub fn real_input_locked(purpose: &str) -> Result<crate::lock::MachineLock, String> {
    let lock = crate::lock::acquire_bounded(purpose).map_err(|e| format!("real-input lock: {e}"))?;
    real_input()?;
    Ok(lock)
}
