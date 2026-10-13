//! System backdrops through DWM (ARCHITECTURE.md 3.10, 4: materials `Mica`, `MicaAlt`, `Acrylic`).
//! The OS draws the material; the backend clears to transparent where it should show.

use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Dwm::{
    DWM_SYSTEMBACKDROP_TYPE, DWMSBT_MAINWINDOW, DWMSBT_NONE, DWMSBT_TABBEDWINDOW, DWMSBT_TRANSIENTWINDOW,
    DWMWA_SYSTEMBACKDROP_TYPE, DWMWA_USE_IMMERSIVE_DARK_MODE, DwmExtendFrameIntoClientArea, DwmSetWindowAttribute,
};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::Win32::UI::Controls::MARGINS;
use windows::core::{BOOL, w};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backdrop {
    None,
    /// The main window material.
    Mica,
    /// Mica with more of the wallpaper's tint, for tabbed windows.
    MicaAlt,
    /// For transient surfaces: flyouts, popups.
    Acrylic,
}

impl Backdrop {
    fn system_type(self) -> DWM_SYSTEMBACKDROP_TYPE {
        match self {
            Self::None => DWMSBT_NONE,
            Self::Mica => DWMSBT_MAINWINDOW,
            Self::MicaAlt => DWMSBT_TABBEDWINDOW,
            Self::Acrylic => DWMSBT_TRANSIENTWINDOW,
        }
    }
}

/// Sets `backdrop` behind the whole window and matches the frame to `dark`. Needs Windows 11
/// 22H2 (build 22621) for `DWMWA_SYSTEMBACKDROP_TYPE`; earlier builds return the DWM error.
pub fn apply(hwnd: HWND, backdrop: Backdrop, dark: bool) -> windows::core::Result<()> {
    let dark = BOOL::from(dark);
    unsafe {
        DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, (&raw const dark).cast(), size_of::<BOOL>() as u32)?;
        // The material shows through the client area only where the frame is extended over it;
        // -1 on every side is DWM's "sheet of glass".
        let margins = if backdrop == Backdrop::None {
            MARGINS::default()
        } else {
            MARGINS { cxLeftWidth: -1, cxRightWidth: -1, cyTopHeight: -1, cyBottomHeight: -1 }
        };
        DwmExtendFrameIntoClientArea(hwnd, &margins)?;
        let kind = backdrop.system_type();
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_SYSTEMBACKDROP_TYPE,
            (&raw const kind).cast(),
            size_of::<DWM_SYSTEMBACKDROP_TYPE>() as u32,
        )
    }
}

/// Whether apps are set to dark (Settings > Personalization > Colors).
pub fn apps_use_dark() -> bool {
    let mut value = 1u32;
    let mut size = size_of::<u32>() as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some((&raw mut value).cast()),
            Some(&mut size),
        )
    };
    status.is_ok() && value == 0
}
