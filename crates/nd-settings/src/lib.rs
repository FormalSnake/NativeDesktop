//! App settings and machine state (ARCHITECTURE.md 11.3).
//!
//! A settings struct is the single source: `#[derive(Settings, Serialize, Deserialize,
//! JsonSchema)]` with `#[serde(default, deny_unknown_fields)]`. From it this crate derives the
//! JSON Schema, the metadata a settings form needs ([`SettingsSchema`]) and the typed options of
//! the home-manager module ([`nix`]). At runtime [`SettingsHandle`] loads the layers (defaults,
//! Nix-managed file, user `config.toml`, `config.d/*.toml`, `config.local.toml`, environment,
//! command line), watches the files, and writes edits with `toml_edit` so comments survive.
//! [`Store`] keeps versioned machine state. On the web every file is a `localStorage` key.
#![cfg_attr(target_arch = "wasm32", allow(unused_imports, dead_code))]

mod edit;
mod handle;
mod layers;
pub mod nix;
mod schema;
#[cfg(not(target_arch = "wasm32"))]
mod store;
#[cfg(target_arch = "wasm32")]
#[path = "store_web.rs"]
mod store;
mod vfs;
mod watch;

pub use handle::{Change, Event, KeyState, Options, Origin, SettingsHandle, WriteError, WriteTarget};
pub use layers::{LayerKind, LayerRef, LoadError, Paths, config_dir};
pub use nd_settings_derive::Settings;
pub use schema::{Choice, Field, FieldKind, SectionInfo, SettingsSchema, schema_json};
pub use store::{Migrate, Store, StoreError};
pub use watch::WatchGuard;

// The derive names `::nd_settings`, which is also this crate.
extern crate self as nd_settings;

use schemars::JsonSchema;
use serde::Serialize;
use serde::de::DeserializeOwned;

/// Implemented by `#[derive(Settings)]`.
pub trait Settings: Default + Serialize + DeserializeOwned + JsonSchema + Send + Sync + 'static {
    /// One entry per field in declaration order, by dotted key (`editor.font_size`), carrying
    /// what `#[settings(..)]` declared. Nested `section` fields are expanded in place.
    fn field_meta() -> Vec<FieldMeta>;
}

/// What `#[settings(range, step, reload)]` declared for one key.
#[derive(Clone, Debug, PartialEq)]
pub struct FieldMeta {
    pub path: String,
    pub range: Option<(f64, f64)>,
    pub step: Option<f64>,
    pub reload: Reload,
}

/// How a changed value reaches the running app.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Reload {
    /// Applied as soon as the file changes.
    #[default]
    Live,
    /// Read once at startup; the settings UI says "restart required".
    Restart,
}
