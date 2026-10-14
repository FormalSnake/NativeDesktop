//! Where an app keeps its files. `ND_APP_DATA_DIR` overrides the root for the whole launch, so
//! tests and agents get isolated data, logs and diagnostics (ARCHITECTURE.md 10.3).

use std::path::PathBuf;

/// `ND_APP_DATA_DIR`, else `~/Library/Application Support/<app>` on macOS, `%APPDATA%\<app>` on
/// Windows and `$XDG_DATA_HOME/<app>` (`~/.local/share/<app>`) elsewhere.
pub fn app_data_dir(app: &str) -> PathBuf {
    if let Some(dir) = std::env::var_os("ND_APP_DATA_DIR").filter(|d| !d.is_empty()) {
        return PathBuf::from(dir);
    }
    let home = || PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    let base = if cfg!(target_os = "macos") {
        home().join("Library/Application Support")
    } else if cfg!(windows) {
        PathBuf::from(std::env::var_os("APPDATA").unwrap_or_default())
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .filter(|d| !d.is_empty())
            .map_or_else(|| home().join(".local/share"), PathBuf::from)
    };
    base.join(app)
}

/// Where an app keeps files it can rebuild (downloaded artwork, thumbnails): `cache` under
/// `ND_APP_DATA_DIR` when that is set, else `~/Library/Caches/<app>` on macOS,
/// `%LOCALAPPDATA%\<app>\cache` on Windows and `$XDG_CACHE_HOME/<app>` (`~/.cache/<app>`)
/// elsewhere.
pub fn app_cache_dir(app: &str) -> PathBuf {
    if let Some(dir) = std::env::var_os("ND_APP_DATA_DIR").filter(|d| !d.is_empty()) {
        return PathBuf::from(dir).join("cache");
    }
    let home = || PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    if cfg!(target_os = "macos") {
        home().join("Library/Caches").join(app)
    } else if cfg!(windows) {
        PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_default()).join(app).join("cache")
    } else {
        std::env::var_os("XDG_CACHE_HOME")
            .filter(|d| !d.is_empty())
            .map_or_else(|| home().join(".cache"), PathBuf::from)
            .join(app)
    }
}
