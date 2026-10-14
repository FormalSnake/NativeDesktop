//! `Store<T>` on the web: the same `{"version": N, "data": ...}` document, kept in
//! `localStorage` under the store's path. A tab has no writer thread and `setItem` is cheap and
//! whole, so each update writes through at once and there is nothing to debounce or flush.

use std::cell::RefCell;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::vfs;

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

pub struct Store<T: Serialize + DeserializeOwned + Default + Clone + Send + Sync + 'static> {
    path: PathBuf,
    version: u32,
    value: RefCell<Arc<T>>,
    load_error: Option<String>,
}

impl<T: Serialize + DeserializeOwned + Default + Clone + Send + Sync + 'static> Store<T> {
    /// Opens the store kept under `path`. A missing entry gives `T::default()`; one that cannot
    /// be migrated or understood is kept aside as `<path>.corrupt-<unix time>`, defaults win,
    /// and [`Store::load_error`] says why.
    pub fn open(path: impl Into<PathBuf>, version: u32, migrate: Option<Migrate>) -> Self {
        let path = path.into();
        let (value, load_error, migrated) = match read::<T>(&path, version, migrate) {
            Ok(Some((value, migrated))) => (value, None, migrated),
            Ok(None) => (T::default(), None, false),
            Err(why) => {
                let mut name = path.file_name().map(|n| n.to_os_string()).unwrap_or_default();
                name.push(format!(".corrupt-{}", (js_sys::Date::now() / 1000.) as u64));
                let _ = vfs::rename(&path, &path.with_file_name(name));
                (T::default(), Some(why), false)
            }
        };
        let store = Self { path, version, value: RefCell::new(Arc::new(value)), load_error };
        if migrated {
            let _ = store.flush();
        }
        store
    }

    pub fn get(&self) -> Arc<T> {
        Arc::clone(&self.value.borrow())
    }

    /// Mutates the state and writes it.
    pub fn update<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        let result = f(Arc::make_mut(&mut self.value.borrow_mut()));
        if let Err(e) = self.flush() {
            web_sys::console::warn_1(&format!("store {}: {e}", self.path.display()).into());
        }
        result
    }

    /// Why the stored entry was not used, if it was not.
    pub fn load_error(&self) -> Option<&str> {
        self.load_error.as_deref()
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Writes the current state. Updates already do; this exists for parity with desktop.
    pub fn flush(&self) -> Result<(), StoreError> {
        let value = self.get();
        let text = serde_json::to_string(&serde_json::json!({ "version": self.version, "data": &*value }))
            .map_err(StoreError::Json)?;
        vfs::write_atomic(&self.path, &text).map_err(StoreError::Io)
    }
}

/// `Ok(None)`: no entry. `Ok(Some((value, migrated)))`. `Err`: unusable entry.
fn read<T: DeserializeOwned>(path: &Path, version: u32, migrate: Option<Migrate>) -> Result<Option<(T, bool)>, String> {
    let text = match vfs::read_to_string(path) {
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
