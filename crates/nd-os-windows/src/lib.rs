//! Windows services for the GPUI backend (ARCHITECTURE.md 2, 5.1, 5.4): system backdrops, the
//! caption hit-testing that gives app-drawn caption buttons Snap Layouts, drag regions,
//! per-monitor DPI v2 and the `WM_POINTER` pen and touch path. It works on an HWND the backend
//! created, through `raw-window-handle`, and names no GPUI type. It also re-exports the backend
//! for `nd`, which reaches it only through this crate so that an app's build script (`nd` with
//! `build` alone) compiles no GPUI.
//!
//! `caption` and `pointer` are plain data and logic, built and tested on every OS.

pub mod caption;
pub mod pointer;

#[cfg(windows)]
pub mod backdrop;
#[cfg(windows)]
pub mod chrome;
#[cfg(windows)]
pub mod colors;
#[cfg(windows)]
pub mod dpi;
#[cfg(windows)]
mod hooks;

#[cfg(windows)]
pub use hooks::WindowsHooks;

#[cfg(windows)]
pub use nd_backend_gpui;
#[cfg(windows)]
pub use windows::Win32::Foundation::HWND;

/// Windows' "Animation effects" (Settings, Accessibility, Visual effects): whether apps animate
/// their client area, as WinUI's controls ask before they do.
#[cfg(windows)]
pub(crate) fn client_area_animation() -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{
        SPI_GETCLIENTAREAANIMATION, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
    };
    use windows::core::BOOL;
    let mut on = BOOL(1);
    // SAFETY: SPI_GETCLIENTAREAANIMATION writes one BOOL through the pointer.
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            Some(&mut on as *mut BOOL as *mut _),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    ok.is_err() || on.as_bool()
}

/// Windows' "Smooth-scroll list boxes" (Performance Options, Visual Effects): whether a notched
/// wheel eases its steps.
#[cfg(windows)]
pub(crate) fn list_smooth_scrolling() -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{
        SPI_GETLISTBOXSMOOTHSCROLLING, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
    };
    use windows::core::BOOL;
    let mut on = BOOL(1);
    // SAFETY: SPI_GETLISTBOXSMOOTHSCROLLING writes one BOOL through the pointer.
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETLISTBOXSMOOTHSCROLLING,
            0,
            Some(&mut on as *mut BOOL as *mut _),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    ok.is_err() || on.as_bool()
}

#[cfg(all(test, windows))]
mod tests {
    #[test]
    fn the_animation_effects_setting_reads() {
        // The value is the machine's; the call has to answer without failing.
        println!("client area animation: {}", super::client_area_animation());
        println!("list smooth scrolling: {}", super::list_smooth_scrolling());
    }
}
