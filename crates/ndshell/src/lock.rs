//! The machine-wide real-input lock (ARCHITECTURE.md 10.3). Real HID input, activation and
//! screen capture share one window server, one cursor and one focus, so tier 3 runs that use them
//! hold this lock and a second run, in this worktree or another, queues behind it instead of
//! stealing focus mid-test. Tests that only post events into their own app's queue skip it.
//!
//! The lock is an advisory `flock` on `real-input.lock` under `~/Library/Caches/nd/` (macOS),
//! `%LOCALAPPDATA%\nd\` (Windows), `$XDG_CACHE_HOME/nd/` or `~/.cache/nd/` elsewhere;
//! `ND_LOCK_DIR` overrides the directory. The
//! holder writes its pid, worktree and purpose into the file, and a waiting run prints them. The
//! OS drops the lock when the holder exits, crashed or not.

use std::fs::{File, OpenOptions, TryLockError};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::thread::ThreadId;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const LOCK_FILE: &str = "real-input.lock";

pub fn lock_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("ND_LOCK_DIR") {
        return PathBuf::from(dir);
    }
    if cfg!(windows)
        && let Some(local) = std::env::var_os("LOCALAPPDATA")
    {
        return PathBuf::from(local).join("nd");
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

/// Lock files each thread of this process holds. `flock` locks belong to the open file, so a
/// thread taking a lock it already holds would wait on itself forever.
static HELD: Mutex<Vec<(ThreadId, PathBuf)>> = Mutex::new(Vec::new());

fn held_by_this_thread(dir: &Path) -> bool {
    let path = dir.join(LOCK_FILE);
    let me = std::thread::current().id();
    HELD.lock().unwrap_or_else(|e| e.into_inner()).iter().any(|(t, p)| *t == me && *p == path)
}

fn refuse_reentry(dir: &Path) -> io::Result<()> {
    if held_by_this_thread(dir) {
        return Err(io::Error::new(
            io::ErrorKind::Deadlock,
            format!("this thread already holds the lock in {}; it is not reentrant", dir.display()),
        ));
    }
    Ok(())
}

/// Held while it lives; dropping it releases the lock.
#[derive(Debug)]
pub struct MachineLock {
    file: File,
    path: PathBuf,
    thread: ThreadId,
}

impl MachineLock {
    fn new(file: File, path: PathBuf) -> Self {
        let thread = std::thread::current().id();
        HELD.lock().unwrap_or_else(|e| e.into_inner()).push((thread, path.clone()));
        Self { file, path, thread }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for MachineLock {
    fn drop(&mut self) {
        let _ = self.file.set_len(0);
        let _ = self.file.unlock();
        let mut held = HELD.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(i) = held.iter().position(|(t, p)| *t == self.thread && *p == self.path) {
            held.swap_remove(i);
        }
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
    refuse_reentry(dir)?;
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
    Ok(MachineLock::new(file, path))
}

/// How long [`window_test`] waits for the lock before failing, in seconds. Under nextest the
/// wait counts against the test's slow-timeout, so the window-group slow-timeout in .config/nextest.toml is larger.
pub const WINDOW_WAIT_SECS: u64 = 600;

/// [`acquire_in`] that gives up after `timeout` with a `TimedOut` error naming the holder.
pub fn acquire_timeout_in(dir: &Path, purpose: &str, timeout: Duration) -> io::Result<MachineLock> {
    refuse_reentry(dir)?;
    let (mut file, path) = open(dir)?;
    let start = Instant::now();
    let mut announced = false;
    loop {
        match file.try_lock() {
            Ok(()) => break,
            Err(TryLockError::WouldBlock) => {
                if !announced {
                    announced = true;
                    let held_by = holder_in(dir);
                    let held_by = if held_by.is_empty() { "another run".to_owned() } else { held_by };
                    eprintln!("ndshell: waiting for the real-input lock ({}), held by {held_by}", path.display());
                }
                if start.elapsed() >= timeout {
                    let held_by = holder_in(dir);
                    let held_by = if held_by.is_empty() { "unknown holder".to_owned() } else { held_by };
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        format!(
                            "{purpose} waited {}s for the machine lock {}, held by {held_by}",
                            timeout.as_secs(),
                            path.display()
                        ),
                    ));
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(TryLockError::Error(e)) => return Err(e),
        }
    }
    record(&mut file, purpose)?;
    Ok(MachineLock::new(file, path))
}

/// The lock a test holds for as long as it has an on-screen window or needs key status, taken
/// once at the top of the test and kept until it ends. Every such test goes through this (the
/// AppKit harness's `main` does it for its binaries), so concurrent runs on one machine queue
/// instead of stealing focus. `test` names the test in the holder line a waiting run prints.
/// Panics after [`WINDOW_WAIT_SECS`] (`ND_WINDOW_LOCK_TIMEOUT_SECS` overrides), naming the holder.
/// Not reentrant: taking it again on the thread that holds it fails at once instead of waiting
/// on itself. Headless tests skip it.
#[track_caller]
pub fn window_test(test: &str) -> MachineLock {
    match acquire_bounded(test) {
        Ok(lock) => lock,
        Err(e) => panic!("window lock: {e}"),
    }
}

/// [`acquire`] bounded by [`WINDOW_WAIT_SECS`] (`ND_WINDOW_LOCK_TIMEOUT_SECS` overrides), for
/// callers that report the error themselves.
pub fn acquire_bounded(purpose: &str) -> io::Result<MachineLock> {
    let secs =
        std::env::var("ND_WINDOW_LOCK_TIMEOUT_SECS").ok().and_then(|v| v.parse().ok()).unwrap_or(WINDOW_WAIT_SECS);
    acquire_timeout_in(&lock_dir(), purpose, Duration::from_secs(secs))
}

/// Takes the lock only if it is free.
pub fn try_acquire(purpose: &str) -> io::Result<Option<MachineLock>> {
    try_acquire_in(&lock_dir(), purpose)
}

pub fn try_acquire_in(dir: &Path, purpose: &str) -> io::Result<Option<MachineLock>> {
    if held_by_this_thread(dir) {
        return Ok(None);
    }
    let (mut file, path) = open(dir)?;
    match file.try_lock() {
        Ok(()) => {
            record(&mut file, purpose)?;
            Ok(Some(MachineLock::new(file, path)))
        }
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(TryLockError::Error(e)) => Err(e),
    }
}
