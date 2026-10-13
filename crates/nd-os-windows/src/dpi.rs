//! Per-monitor DPI v2 (ARCHITECTURE.md 5.1). Binaries linking GPUI get it from GPUI's embedded
//! manifest; anything else that measures or posts screen coordinates (the `ndshell` agent) sets
//! it here before creating a window, or Windows scales its coordinates behind its back.

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::HiDpi::{
    AreDpiAwarenessContextsEqual, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForWindow,
    GetThreadDpiAwarenessContext, SetProcessDpiAwarenessContext,
};

/// Makes the process per-monitor v2 aware. True when it is afterwards, including when a manifest
/// already made it so (the call then fails with access denied, which is fine).
pub fn enable_per_monitor_v2() -> bool {
    let _ = unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    is_per_monitor_v2()
}

pub fn is_per_monitor_v2() -> bool {
    unsafe { AreDpiAwarenessContextsEqual(GetThreadDpiAwarenessContext(), DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) }
        .as_bool()
}

/// Physical pixels per logical pixel for the monitor `hwnd` is on.
pub fn scale(hwnd: HWND) -> f32 {
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    if dpi == 0 { 1. } else { dpi as f32 / 96. }
}
