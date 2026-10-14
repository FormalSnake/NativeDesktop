//! `localStorage` for app state that is not settings or a `Store<T>`: small strings that
//! survive a reload, per origin. A private window or a disabled storage gives `Unsupported`.

use nd_core::platform::{Feature, Unsupported};

fn storage() -> Result<web_sys::Storage, Unsupported> {
    web_sys::window().and_then(|w| w.local_storage().ok().flatten()).ok_or(Unsupported::here(Feature::FileSystem))
}

pub fn get(key: &str) -> Option<String> {
    storage().ok()?.get_item(key).ok().flatten()
}

/// Fails when storage is off or full.
pub fn set(key: &str, value: &str) -> Result<(), Unsupported> {
    storage()?.set_item(key, value).map_err(|_| Unsupported::here(Feature::FileSystem))
}

pub fn remove(key: &str) {
    if let Ok(storage) = storage() {
        let _ = storage.remove_item(key);
    }
}
