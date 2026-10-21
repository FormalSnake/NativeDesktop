//! Native messaging hosts (1Password's, a password manager's, any desktop app's bridge to its
//! extension). Chromium looks for a host's manifest in `<user data>/NativeMessagingHosts` and in
//! Chrome's system directory, and has no switch for another: a desktop app installs its
//! manifest for the browsers it knows, so the manifests of those browsers are linked into ours.
//! Chromium reads the manifest when an extension connects, so linking again before an action
//! runs picks up a host installed while the app runs.

use std::path::{Path, PathBuf};

fn sources() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    #[cfg(target_os = "macos")]
    {
        let support = home.join("Library/Application Support");
        let mut dirs: Vec<PathBuf> = [
            "Google/Chrome",
            "Google/Chrome Beta",
            "Google/Chrome Canary",
            "Chromium",
            "BraveSoftware/Brave-Browser",
            "Microsoft Edge",
            "Vivaldi",
            "net.imput.helium",
        ]
        .iter()
        .map(|b| support.join(b).join("NativeMessagingHosts"))
        .collect();
        dirs.push("/Library/Google/Chrome/NativeMessagingHosts".into());
        dirs.push("/Library/Microsoft/Edge/NativeMessagingHosts".into());
        dirs
    }
    #[cfg(not(target_os = "macos"))]
    {
        let config = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".config"));
        let mut dirs: Vec<PathBuf> = [
            "google-chrome",
            "google-chrome-beta",
            "google-chrome-unstable",
            "chromium",
            "BraveSoftware/Brave-Browser",
            "microsoft-edge",
            "vivaldi",
            "net.imput.helium",
        ]
        .iter()
        .map(|b| config.join(b).join("NativeMessagingHosts"))
        .collect();
        dirs.push("/etc/opt/chrome/native-messaging-hosts".into());
        dirs.push("/etc/chromium/native-messaging-hosts".into());
        dirs.push("/etc/opt/edge/native-messaging-hosts".into());
        dirs
    }
}

/// Links every host manifest the known browsers have into `root/NativeMessagingHosts`. A
/// manifest of our own (a regular file) wins over a link; links whose target went away go.
pub(crate) fn link_hosts(root: &Path) {
    let dest = root.join("NativeMessagingHosts");
    if std::fs::create_dir_all(&dest).is_err() {
        return;
    }
    if let Ok(entries) = std::fs::read_dir(&dest) {
        for entry in entries.flatten() {
            let path = entry.path();
            let dangling = path.is_symlink() && !path.exists();
            if dangling && let Err(e) = std::fs::remove_file(&path) {
                tracing::debug!("nd-cef: {}: {e}", path.display());
            }
        }
    }
    for dir in sources() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let source = entry.path();
            if source.extension().is_none_or(|e| e != "json") {
                continue;
            }
            let Some(name) = source.file_name() else { continue };
            let link = dest.join(name);
            if link.symlink_metadata().is_ok() {
                continue;
            }
            if let Err(e) = std::os::unix::fs::symlink(&source, &link) {
                tracing::debug!("nd-cef: linking {}: {e}", source.display());
            }
        }
    }
}
