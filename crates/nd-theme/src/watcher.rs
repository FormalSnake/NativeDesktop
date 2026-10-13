//! Watches one theme file (`theme.json`, `theme.toml`) and hands each new, parseable version to
//! a callback on the watcher's thread. The parent directory is watched, not the file: atomic
//! rename writers replace the inode and a watch on the file would die with it. The first event
//! of a burst is read at once (a recolour lands in one frame), and the burst is read again
//! [`DEBOUNCE`] later, so a writer that was still writing is picked up whole; events in between
//! cost nothing. An mtime stat every [`POLL`] catches anything the OS dropped. A file that
//! fails to parse (half written, mid-edit) is skipped, so the last good theme stays.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use notify::{RecursiveMode, Watcher};

pub const DEBOUNCE: Duration = Duration::from_millis(50);
pub const POLL: Duration = Duration::from_secs(1);

/// Stops the watch when dropped.
pub struct ThemeFileWatcher {
    stop: Option<mpsc::Sender<Msg>>,
    thread: Option<thread::JoinHandle<()>>,
}

enum Msg {
    Changed,
    Stop,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Stamp {
    modified: Option<SystemTime>,
    len: u64,
}

fn stamp(path: &Path) -> Option<Stamp> {
    let meta = std::fs::metadata(path).ok()?;
    Some(Stamp { modified: meta.modified().ok(), len: meta.len() })
}

impl ThemeFileWatcher {
    /// Reads `path` now (when it exists) and on every change after. `parse` turns its text
    /// into a theme; `on_change` gets each parsed version that differs from the last.
    pub fn spawn<T, E>(
        path: impl Into<PathBuf>,
        parse: impl Fn(&str) -> Result<T, E> + Send + 'static,
        on_change: impl Fn(T) + Send + 'static,
    ) -> std::io::Result<ThemeFileWatcher>
    where
        T: Send + 'static,
        E: std::fmt::Display,
    {
        let path: PathBuf = path.into();
        let dir = path.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."));
        std::fs::create_dir_all(&dir)?;
        let (tx, rx) = mpsc::channel();
        let file_name = path.file_name().map(|n| n.to_owned());
        let events = tx.clone();
        let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            let Ok(event) = event else { return };
            if event.paths.iter().any(|p| p.file_name() == file_name.as_deref()) {
                let _ = events.send(Msg::Changed);
            }
        })
        .map_err(std::io::Error::other)?;
        watcher.watch(&dir, RecursiveMode::NonRecursive).map_err(std::io::Error::other)?;

        let thread = thread::Builder::new().name("nd-theme-watch".into()).spawn(move || {
            let _watcher = watcher;
            let mut last_text: Option<String> = None;
            let mut read_now = false;
            let read = |last_text: &mut Option<String>| {
                let Ok(text) = std::fs::read_to_string(&path) else { return };
                if last_text.as_deref() == Some(text.as_str()) {
                    return;
                }
                match parse(&text) {
                    Ok(theme) => {
                        *last_text = Some(text);
                        on_change(theme);
                    }
                    Err(e) => eprintln!("nd: {} kept the last theme: {e}", path.display()),
                }
            };
            let mut last_stamp = None;
            let mut deadline: Option<Instant> = Some(Instant::now());
            let mut next_poll = Instant::now() + POLL;
            loop {
                let now = Instant::now();
                let wake = deadline.map_or(next_poll, |d| d.min(next_poll));
                match rx.recv_timeout(wake.saturating_duration_since(now)) {
                    Ok(Msg::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => return,
                    Ok(Msg::Changed) => {
                        if deadline.is_none() {
                            // Leading edge now, trailing edge after the debounce.
                            read_now = true;
                            deadline = Some(Instant::now() + DEBOUNCE);
                        }
                        if !read_now {
                            continue;
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                }
                let now = Instant::now();
                if std::mem::take(&mut read_now) {
                    read(&mut last_text);
                    continue;
                }
                if now >= next_poll {
                    next_poll = now + POLL;
                    let current = stamp(&path);
                    if current != last_stamp {
                        deadline.get_or_insert(now);
                    }
                }
                if deadline.is_none_or(|d| now < d) {
                    continue;
                }
                deadline = None;
                last_stamp = stamp(&path);
                read(&mut last_text);
            }
        })?;
        Ok(ThemeFileWatcher { stop: Some(tx), thread: Some(thread) })
    }
}

impl Drop for ThemeFileWatcher {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(Msg::Stop);
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
