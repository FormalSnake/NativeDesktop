//! What the running target can do (ARCHITECTURE.md 12), asked at runtime instead of with `cfg`:
//! `if nd::platform::has(Feature::MultiWindow) { .. }`. An API that cannot work on a target
//! returns [`Unsupported`] there and carries `#[nd::unavailable(web)]`, which
//! `nd check --target web` reports at every call site that has no `has(..)` check around it.

use std::fmt;

/// The platform family a binary was built for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Target {
    MacOs,
    Windows,
    Linux,
    Web,
}

/// The target this binary was built for.
pub const fn target() -> Target {
    if cfg!(target_arch = "wasm32") {
        Target::Web
    } else if cfg!(target_os = "macos") {
        Target::MacOs
    } else if cfg!(target_os = "windows") {
        Target::Windows
    } else {
        Target::Linux
    }
}

/// A capability an app may branch on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Feature {
    /// Separate OS windows. On the web extra windows open inside the one canvas instead.
    MultiWindow,
    /// Paths the app can read and write directly. The web has file pickers and downloads only.
    FileSystem,
    /// A global menu bar. The web and GPUI desktops fold menus into the palette.
    MenuBar,
    /// The system tray or menu bar extras.
    Tray,
    /// Native notifications beyond the browser's Notification API.
    Notifications,
    /// The OS keychain or secret service.
    Credentials,
    /// The Chromium engine (`nd::cef`).
    Cef,
    /// An embedded web engine (`webview()`). Not on the web yet: the iframe host waits for the
    /// native surface seam of the overlays milestone.
    Webview,
    /// Background update checks and installs (`nd::update`).
    Updates,
    /// Sockets and named pipes to other processes (`nd::ipc`).
    Ipc,
    /// OS threads (`std::thread`). The web runs on the page's one thread.
    Threads,
    /// Reading the clipboard without a user gesture.
    ClipboardReadAnytime,
    /// Native widgets without a drawn implementation (`NativeWidget::backends`).
    NativeOnlyWidgets,
}

/// Whether `feature` works on the running target.
pub const fn has(feature: Feature) -> bool {
    match target() {
        Target::Web => false,
        Target::MacOs => true,
        Target::Windows | Target::Linux => !matches!(feature, Feature::MenuBar | Feature::NativeOnlyWidgets),
    }
}

/// The error an API returns on a target it cannot work on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Unsupported {
    pub feature: Feature,
    pub target: Target,
}

impl Unsupported {
    pub const fn here(feature: Feature) -> Self {
        Self { feature, target: target() }
    }
}

impl fmt::Display for Unsupported {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?} is not available on {:?}", self.feature, self.target)
    }
}

impl std::error::Error for Unsupported {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_lacks_desktop_features() {
        if target() == Target::Web {
            assert!(!has(Feature::MultiWindow));
            assert!(!has(Feature::FileSystem));
        } else {
            assert!(has(Feature::MultiWindow));
            assert!(has(Feature::FileSystem));
        }
    }
}
