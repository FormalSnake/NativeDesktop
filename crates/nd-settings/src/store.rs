//! `Store<T>`: versioned machine state (window frames, recents, last-used folder). JSON on disk
//! as `{"version": N, "data": ...}`, written through a debounce by a background thread with a
//! temp file and rename, migrated on load, flushed on drop.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::edit::write_atomic;

const DEBOUNCE: Duration = Duration::from_millis(250);
/// A store that keeps changing still reaches disk this often.
const MAX_DELAY: Duration = Duration::from_secs(2);

#[derive(Debug)]
pub enum StoreError {
    Io(std::io::Error),
    Json(serde_json::Error),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => e.fmt(f),
            Self::Json(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for StoreError {}

/// Upgrades the stored JSON from `from_version` to the current layout, or fails to reset.
pub type Migrate = fn(from_version: u32, data: Value) -> Result<Value, String>;

struct Shared<T> {
    path: PathBuf,
    version: u32,
    state: Mutex<Pending<T>>,
    wake: Condvar,
    /// Serializes file writes between the thread and `flush`.
    write: Mutex<()>,
}

struct Pending<T> {
    value: Arc<T>,
    dirty: bool,
    first_dirty: Option<Instant>,
    last_change: Instant,
    stop: bool,
}

pub struct Store<T: Serialize + DeserializeOwned + Default + Clone + Send + Sync + 'static> {
    shared: Arc<Shared<T>>,
    thread: Option<JoinHandle<()>>,
    load_error: Option<String>,
}

impl<T: Serialize + DeserializeOwned + Default + Clone + Send + Sync + 'static> Store<T> {
    /// Opens `path`. A missing file gives `T::default()`. A file that cannot be read, migrated
    /// or understood is moved aside as `<name>.corrupt-<unix time>`, defaults win, and
    /// [`Store::load_error`] says why.
    pub fn open(path: impl Into<PathBuf>, version: u32, migrate: Option<Migrate>) -> Self {
        let path = path.into();
        let (value, load_error, migrated) = match read::<T>(&path, version, migrate) {
            Ok(Some((value, migrated))) => (value, None, migrated),
            Ok(None) => (T::default(), None, false),
            Err(why) => {
                quarantine(&path);
                (T::default(), Some(why), false)
            }
        };
        let now = Instant::now();
        let shared = Arc::new(Shared {
            path,
            version,
            state: Mutex::new(Pending {
                value: Arc::new(value),
                dirty: migrated,
                first_dirty: migrated.then_some(now),
                last_change: now,
                stop: false,
            }),
            wake: Condvar::new(),
            write: Mutex::new(()),
        });
        let worker = Arc::clone(&shared);
        let thread = std::thread::Builder::new()
            .name("nd-store-writer".into())
            .spawn(move || writer(&worker))
            .expect("spawn store writer");
        Self { shared, thread: Some(thread), load_error }
    }

    pub fn get(&self) -> Arc<T> {
        Arc::clone(&self.shared.state.lock().expect("store lock").value)
    }

    /// Mutates the state and schedules a write.
    pub fn update<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        let mut state = self.shared.state.lock().expect("store lock");
        let result = f(Arc::make_mut(&mut state.value));
        let now = Instant::now();
        state.first_dirty.get_or_insert(now);
        state.dirty = true;
        state.last_change = now;
        self.shared.wake.notify_all();
        result
    }

    /// Why the file on disk was not used, if it was not.
    pub fn load_error(&self) -> Option<&str> {
        self.load_error.as_deref()
    }

    pub fn path(&self) -> &Path {
        &self.shared.path
    }

    /// Writes pending changes now. Call before the process exits on a path that skips `Drop`.
    pub fn flush(&self) -> Result<(), StoreError> {
        flush(&self.shared)
    }
}

impl<T: Serialize + DeserializeOwned + Default + Clone + Send + Sync + 'static> Drop for Store<T> {
    fn drop(&mut self) {
        self.shared.state.lock().expect("store lock").stop = true;
        self.shared.wake.notify_all();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        let _ = flush(&self.shared);
    }
}

fn flush<T: Serialize>(shared: &Shared<T>) -> Result<(), StoreError> {
    let _write = shared.write.lock().expect("store write lock");
    let value = {
        let mut state = shared.state.lock().expect("store lock");
        if !state.dirty {
            return Ok(());
        }
        state.dirty = false;
        state.first_dirty = None;
        Arc::clone(&state.value)
    };
    let text = serde_json::to_string_pretty(&serde_json::json!({ "version": shared.version, "data": &*value }))
        .map_err(StoreError::Json)?;
    let result = write_atomic(&shared.path, &(text + "\n")).map_err(StoreError::Io);
    if result.is_err() {
        shared.state.lock().expect("store lock").dirty = true;
    }
    result
}

fn writer<T: Serialize>(shared: &Shared<T>) {
    loop {
        let mut state = shared.state.lock().expect("store lock");
        loop {
            if state.stop {
                return;
            }
            if let Some(first) = state.first_dirty.filter(|_| state.dirty) {
                let due = (state.last_change + DEBOUNCE).min(first + MAX_DELAY);
                let now = Instant::now();
                if now >= due {
                    break;
                }
                state = shared.wake.wait_timeout(state, due - now).expect("store lock").0;
            } else {
                state = shared.wake.wait(state).expect("store lock");
            }
        }
        drop(state);
        if let Err(e) = flush(shared) {
            eprintln!("store {}: {e}", shared.path.display());
            // Back off so a read-only disk does not spin.
            std::thread::sleep(Duration::from_secs(1));
        }
    }
}

/// `Ok(None)`: no file. `Ok(Some((value, migrated)))`. `Err`: unusable file.
fn read<T: DeserializeOwned>(path: &Path, version: u32, migrate: Option<Migrate>) -> Result<Option<(T, bool)>, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    let mut doc: Value = serde_json::from_str(&text).map_err(|e| format!("not JSON: {e}"))?;
    let stored = doc.get("version").and_then(Value::as_u64).ok_or("no version field")? as u32;
    let mut data = doc.get_mut("data").map(Value::take).ok_or("no data field")?;
    let mut migrated = false;
    if stored > version {
        return Err(format!("written by a newer version ({stored} > {version})"));
    }
    if stored < version {
        let migrate = migrate.ok_or_else(|| format!("version {stored} needs a migration to {version}"))?;
        data = migrate(stored, data).map_err(|e| format!("migration from {stored} failed: {e}"))?;
        migrated = true;
    }
    let value = serde_json::from_value(data).map_err(|e| format!("data does not fit: {e}"))?;
    Ok(Some((value, migrated)))
}

fn quarantine(path: &Path) {
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let mut name = path.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(format!(".corrupt-{stamp}"));
    let _ = std::fs::rename(path, path.with_file_name(name));
}
