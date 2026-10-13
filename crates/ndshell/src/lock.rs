//! The machine-wide real-input lock (ARCHITECTURE.md 10.3). Real HID input, activation and
//! screen capture share one window server, one cursor and one focus, so tier 3 runs that use them
//! hold this lock and a second run, in this worktree or another, queues behind it instead of
//! stealing focus mid-test. Tests that only post events into their own app's queue skip it.
//!
//! The lock is an advisory `flock` on `real-input.lock` under `~/Library/Caches/nd/` (macOS),
//! `$XDG_CACHE_HOME/nd/` or `~/.cache/nd/` elsewhere; `ND_LOCK_DIR` overrides the directory. The
//! holder writes its pid, worktree and purpose into the file, and a waiting run prints them. The
//! OS drops the lock when the holder exits, crashed or not.

use std::fs::{File, OpenOptions, TryLockError};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const LOCK_FILE: &str = "real-input.lock";

pub fn lock_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("ND_LOCK_DIR") {
        return PathBuf::from(dir);
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    if cfg!(target_os = "macos") {
        return home.join("Library/Caches/nd");
    }
    std::env::var_os("XDG_CACHE_HOME").map_or_else(|| home.join(".cache"), PathBuf::from).join("nd")
}

/// The git checkout the current directory is in, which names a run to the one waiting on it.
fn worktree() -> String {
    let cwd = std::env::current_dir().unwrap_or_default();
    cwd.ancestors().find(|dir| dir.join(".git").exists()).unwrap_or(&cwd).display().to_string()
}

/// Held while it lives; dropping it releases the lock.
#[derive(Debug)]
pub struct MachineLock {
    file: File,
    path: PathBuf,
}

impl MachineLock {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for MachineLock {
    fn drop(&mut self) {
        let _ = self.file.set_len(0);
        let _ = self.file.unlock();
    }
}

/// Who holds the lock, as the holder wrote it; empty when nobody does or it has not written yet.
pub fn holder() -> String {
    holder_in(&lock_dir())
}

pub fn holder_in(dir: &Path) -> String {
    let mut text = String::new();
    if let Ok(mut file) = File::open(dir.join(LOCK_FILE)) {
        let _ = file.read_to_string(&mut text);
    }
    text.trim().to_owned()
}

fn open(dir: &Path) -> io::Result<(File, PathBuf)> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(LOCK_FILE);
    // Not truncated on open: a waiting run must not erase the holder's line.
    let file = OpenOptions::new().read(true).write(true).create(true).truncate(false).open(&path)?;
    Ok((file, path))
}

fn record(file: &mut File, purpose: &str) -> io::Result<()> {
    let since = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    file.set_len(0)?;
    file.seek(SeekFrom::Start(0))?;
    writeln!(file, "pid={} worktree={} purpose={purpose} since={since}", std::process::id(), worktree())?;
    file.flush()
}

/// Takes the lock, waiting for the current holder if there is one. While waiting it prints
/// `ndshell: waiting for the real-input lock, held by <holder>` on stderr, once.
pub fn acquire(purpose: &str) -> io::Result<MachineLock> {
    acquire_in(&lock_dir(), purpose)
}

/// [`acquire`] on a lock file in `dir`.
pub fn acquire_in(dir: &Path, purpose: &str) -> io::Result<MachineLock> {
    let (mut file, path) = open(dir)?;
    match file.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => {
            let held_by = holder_in(dir);
            let held_by = if held_by.is_empty() { "another run".to_owned() } else { held_by };
            eprintln!("ndshell: waiting for the real-input lock ({}), held by {held_by}", path.display());
            file.lock()?;
        }
        Err(TryLockError::Error(e)) => return Err(e),
    }
    record(&mut file, purpose)?;
    Ok(MachineLock { file, path })
}

/// Takes the lock only if it is free.
pub fn try_acquire(purpose: &str) -> io::Result<Option<MachineLock>> {
    try_acquire_in(&lock_dir(), purpose)
}

pub fn try_acquire_in(dir: &Path, purpose: &str) -> io::Result<Option<MachineLock>> {
    let (mut file, path) = open(dir)?;
    match file.try_lock() {
        Ok(()) => {
            record(&mut file, purpose)?;
            Ok(Some(MachineLock { file, path }))
        }
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(TryLockError::Error(e)) => Err(e),
    }
}
