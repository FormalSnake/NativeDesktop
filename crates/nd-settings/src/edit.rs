//! Writing: classify a file as editable or Nix-managed, edit a TOML document in place with
//! `toml_edit`, and replace files atomically.

use serde::Serialize as _;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FileState {
    Missing,
    Writable,
    /// A symlink that resolves into the Nix store.
    NixManaged,
    /// Present but not writable, for another reason.
    ReadOnly,
}

/// `store_roots` are the store directories (normally `/nix/store`), compared canonically.
pub(crate) fn file_state(path: &Path, store_roots: &[PathBuf]) -> FileState {
    let Ok(meta) = fs::symlink_metadata(path) else { return FileState::Missing };
    let resolved = fs::canonicalize(path).unwrap_or_else(|_| path.to_owned());
    let in_store = store_roots.iter().any(|root| {
        let root = fs::canonicalize(root).unwrap_or_else(|_| root.clone());
        resolved.starts_with(&root)
    });
    if in_store {
        return FileState::NixManaged;
    }
    let readonly = fs::metadata(&resolved).map_or(true, |m| m.permissions().readonly());
    if readonly || (meta.is_symlink() && !resolved.exists()) { FileState::ReadOnly } else { FileState::Writable }
}

/// Sets `key` (dotted) to `value`, or removes it when `value` is `None`. Comments, key order and
/// the trailing comment of an edited value stay.
pub(crate) fn edit_document(text: &str, key: &str, value: Option<&toml::Value>) -> Result<String, String> {
    let mut doc: toml_edit::DocumentMut = text.parse().map_err(|e| format!("{e}"))?;
    let segments: Vec<&str> = key.split('.').collect();
    let (last, parents) = segments.split_last().ok_or("empty key")?;
    let mut table: &mut dyn toml_edit::TableLike = doc.as_table_mut();
    for segment in parents {
        if table.get(segment).is_none() {
            if value.is_none() {
                return Ok(text.to_owned());
            }
            let mut created = toml_edit::Table::new();
            created.set_implicit(true);
            table.insert(segment, toml_edit::Item::Table(created));
        }
        table = table
            .get_mut(segment)
            .and_then(toml_edit::Item::as_table_like_mut)
            .ok_or_else(|| format!("`{segment}` is not a table"))?;
    }
    match value {
        None => {
            table.remove(last);
        }
        Some(value) => {
            let mut new = value.serialize(toml_edit::ser::ValueSerializer::new()).map_err(|e| format!("{e}"))?;
            match table.get_mut(last) {
                Some(item) => {
                    if let Some(old) = item.as_value() {
                        *new.decor_mut() = old.decor().clone();
                    }
                    *item = toml_edit::Item::Value(new);
                }
                None => {
                    table.insert(last, toml_edit::Item::Value(new));
                }
            }
        }
    }
    Ok(doc.to_string())
}

static TMP: AtomicU64 = AtomicU64::new(0);

/// Writes `text` through a temp file beside the target and renames it over. A symlink to an
/// ordinary file is followed, so a dotfiles link keeps pointing where it did.
pub(crate) fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    let real = fs::canonicalize(path).unwrap_or_else(|_| path.to_owned());
    if let Some(dir) = real.parent() {
        fs::create_dir_all(dir)?;
    }
    let name = real.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = real.with_file_name(format!(".{name}.tmp{}-{}", std::process::id(), TMP.fetch_add(1, Ordering::Relaxed)));
    let result = (|| {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        if let Ok(meta) = fs::metadata(&real) {
            fs::set_permissions(&tmp, meta.permissions())?;
        }
        fs::rename(&tmp, &real)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}
