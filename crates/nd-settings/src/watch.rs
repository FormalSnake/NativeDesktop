//! Live reload: watch the config directories, debounce, reload once.

use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread::JoinHandle;
use std::time::Duration;

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use crate::Settings;
use crate::handle::SettingsHandle;

const DEBOUNCE: Duration = Duration::from_millis(120);

/// Keeps the watcher running; dropping it stops reloads.
pub struct WatchGuard {
    watcher: Option<RecommendedWatcher>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for WatchGuard {
    fn drop(&mut self) {
        // Dropping the watcher closes the channel, which ends the thread.
        self.watcher.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl<T: Settings> SettingsHandle<T> {
    /// Reloads when a config file changes. Directories are watched rather than files, because
    /// editors and home-manager replace files by rename and swap symlinks. The reload compares
    /// file contents with what is loaded, so the app's own saves and repeated events do nothing.
    /// Listeners registered with `on_event` see the outcome.
    pub fn watch(&self) -> notify::Result<WatchGuard> {
        // Only this tab writes its keys, and its writes already reload.
        #[cfg(target_arch = "wasm32")]
        return Ok(WatchGuard { watcher: None, thread: None });
        #[cfg(not(target_arch = "wasm32"))]
        self.watch_files()
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn watch_files(&self) -> notify::Result<WatchGuard> {
        let (tx, rx) = mpsc::channel::<notify::Result<notify::Event>>();
        let mut watcher = notify::recommended_watcher(move |res| {
            let _ = tx.send(res);
        })?;
        for dir in self.watch_dirs() {
            if let Err(e) = std::fs::create_dir_all(&dir) {
                tracing_free_warn(&format!("settings: cannot create {}: {e}", dir.display()));
                continue;
            }
            watcher.watch(&dir, RecursiveMode::NonRecursive)?;
        }
        let handle = self.clone();
        let thread = std::thread::Builder::new()
            .name("nd-settings-watch".into())
            .spawn(move || {
                let relevant = |res: &notify::Result<notify::Event>| match res {
                    Ok(event) => {
                        !matches!(event.kind, EventKind::Access(_))
                            && event.paths.iter().any(|p| handle.is_config_path(p))
                    }
                    Err(_) => true,
                };
                while let Ok(first) = rx.recv() {
                    let mut dirty = relevant(&first);
                    loop {
                        match rx.recv_timeout(DEBOUNCE) {
                            Ok(more) => dirty |= relevant(&more),
                            Err(RecvTimeoutError::Timeout) => break,
                            Err(RecvTimeoutError::Disconnected) => return,
                        }
                    }
                    if dirty {
                        handle.reload();
                    }
                }
            })
            .map_err(notify::Error::io)?;
        Ok(WatchGuard { watcher: Some(watcher), thread: Some(thread) })
    }
}

fn tracing_free_warn(message: &str) {
    eprintln!("{message}");
}
