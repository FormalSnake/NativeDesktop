//! Where an app keeps its files, named after its package (`AppIdentity::name`). With
//! `ND_APP_DATA_DIR` set every directory sits inside it, so a test or agent launch is isolated
//! (ARCHITECTURE.md 10.3). The `ensure_*` forms create the directory.
//!
//! | | macOS | Linux | Windows |
//! | --- | --- | --- | --- |
//! | data | `~/Library/Application Support/<app>` | `$XDG_DATA_HOME/<app>` | `%APPDATA%\<app>` |
//! | config | as data (`nd-settings`' rule) | `$XDG_CONFIG_HOME/<app>` | `%APPDATA%\<app>` |
//! | cache | `~/Library/Caches/<app>` | `$XDG_CACHE_HOME/<app>` | `%LOCALAPPDATA%\<app>\cache` |
//! | logs | `<data>/logs` (`nd-diag`'s) | | |

use std::io;
use std::path::PathBuf;

fn root_override() -> Option<PathBuf> {
    std::env::var_os("ND_APP_DATA_DIR").filter(|d| !d.is_empty()).map(PathBuf::from)
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
}

fn xdg(var: &str, fallback: &str) -> PathBuf {
    std::env::var_os(var).filter(|d| !d.is_empty()).map_or_else(|| home().join(fallback), PathBuf::from)
}

pub fn data_dir(app: &str) -> PathBuf {
    nd_core::paths::app_data_dir(app)
}

/// `ND_APP_CONFIG_DIR` first, as `nd-settings` reads it.
pub fn config_dir(app: &str) -> PathBuf {
    if let Some(dir) = std::env::var_os("ND_APP_CONFIG_DIR").filter(|d| !d.is_empty()) {
        return PathBuf::from(dir);
    }
    if let Some(root) = root_override() {
        return root.join("config");
    }
    if cfg!(target_os = "macos") || cfg!(windows) { data_dir(app) } else { xdg("XDG_CONFIG_HOME", ".config").join(app) }
}

pub fn cache_dir(app: &str) -> PathBuf {
    if let Some(root) = root_override() {
        return root.join("cache");
    }
    if cfg!(target_os = "macos") {
        home().join("Library/Caches").join(app)
    } else if cfg!(windows) {
        PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_default()).join(app).join("cache")
    } else {
        xdg("XDG_CACHE_HOME", ".cache").join(app)
    }
}

/// Where `nd-diag` writes the app's rolling logs.
pub fn logs_dir(app: &str) -> PathBuf {
    data_dir(app).join("logs")
}

fn ensure(dir: PathBuf) -> io::Result<PathBuf> {
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub fn ensure_data_dir(app: &str) -> io::Result<PathBuf> {
    ensure(data_dir(app))
}

pub fn ensure_config_dir(app: &str) -> io::Result<PathBuf> {
    ensure(config_dir(app))
}

pub fn ensure_cache_dir(app: &str) -> io::Result<PathBuf> {
    ensure(cache_dir(app))
}

pub fn ensure_logs_dir(app: &str) -> io::Result<PathBuf> {
    ensure(logs_dir(app))
}
