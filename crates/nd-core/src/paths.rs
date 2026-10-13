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
