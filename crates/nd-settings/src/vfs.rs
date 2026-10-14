//! The files settings and stores live in. A browser tab has no file system, so on the web each
//! path is a `localStorage` key (`nd-file:<path>`) and the layered settings, their writes and
//! `Store<T>` work unchanged on top of it.

#[cfg(not(target_arch = "wasm32"))]
pub(crate) use std::fs::{read, read_to_string};

#[cfg(target_arch = "wasm32")]
pub(crate) use web::*;

#[cfg(target_arch = "wasm32")]
mod web {
    use std::io;
    use std::path::{Path, PathBuf};

    use crate::edit::FileState;

    const PREFIX: &str = "nd-file:";

    fn storage() -> io::Result<web_sys::Storage> {
        web_sys::window()
            .and_then(|w| w.local_storage().ok().flatten())
            .ok_or_else(|| io::Error::new(io::ErrorKind::Unsupported, "localStorage is not available"))
    }

    fn key(path: &Path) -> String {
        format!("{PREFIX}{}", path.display())
    }

    fn js_error(error: wasm_bindgen::JsValue) -> io::Error {
        // A full quota is the usual cause.
        io::Error::other(format!("localStorage: {error:?}"))
    }

    pub(crate) fn read_to_string(path: impl AsRef<Path>) -> io::Result<String> {
        storage()?
            .get_item(&key(path.as_ref()))
            .map_err(js_error)?
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, path.as_ref().display().to_string()))
    }

    pub(crate) fn read(path: impl AsRef<Path>) -> io::Result<Vec<u8>> {
        read_to_string(path).map(String::into_bytes)
    }

    /// One `setItem` replaces the value whole, so there is no torn write to guard against.
    pub(crate) fn write_atomic(path: &Path, text: &str) -> io::Result<()> {
        storage()?.set_item(&key(path), text).map_err(js_error)
    }

    pub(crate) fn rename(from: &Path, to: &Path) -> io::Result<()> {
        let storage = storage()?;
        let text = read_to_string(from)?;
        storage.set_item(&key(to), &text).map_err(js_error)?;
        storage.remove_item(&key(from)).map_err(js_error)
    }

    /// Nothing under Nix's control reaches a tab: a key is there and writable, or missing.
    pub(crate) fn file_state(path: &Path, _store_roots: &[PathBuf]) -> FileState {
        match read_to_string(path) {
            Ok(_) => FileState::Writable,
            Err(_) => FileState::Missing,
        }
    }
}
