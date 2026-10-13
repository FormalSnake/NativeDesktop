//! Layered load: defaults < Nix-managed file < user `config.toml` < `config.d/*.toml` <
//! `config.local.toml` < environment < command line (ARCHITECTURE.md 11.3). Every key remembers
//! which layer set it, so the form can say "Managed by Nix" or "Set by the environment".

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use toml::{Table, Value};

use crate::Settings;
use crate::edit::{FileState, file_state};
use crate::schema::{FieldKind, SettingsSchema};

/// Where a value came from, lowest precedence first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum LayerKind {
    Defaults,
    /// A Nix-managed file: an explicit managed path, or any config file that resolves into the
    /// Nix store.
    Nix,
    User,
    DropIn,
    /// The writable overlay used when the main file is read-only.
    Local,
    Env,
    Cli,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayerRef {
    pub kind: LayerKind,
    /// The file, or the variable name for [`LayerKind::Env`].
    pub origin: Option<PathBuf>,
    pub label: String,
}

impl fmt::Display for LayerRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.label)
    }
}

/// The files that make up one app's settings.
#[derive(Clone, Debug)]
pub struct Paths {
    /// A Nix-managed or system file below the user's.
    pub managed: Option<PathBuf>,
    pub user: PathBuf,
    pub drop_in_dir: PathBuf,
    pub local: PathBuf,
}

impl Paths {
    /// `config.toml`, `config.d/` and `config.local.toml` inside `dir`.
    pub fn in_dir(dir: &Path) -> Self {
        Self {
            managed: None,
            user: dir.join("config.toml"),
            drop_in_dir: dir.join("config.d"),
            local: dir.join("config.local.toml"),
        }
    }

    pub fn for_app(app: &str) -> Self {
        Self::in_dir(&config_dir(app))
    }
}

/// `ND_APP_CONFIG_DIR`, else `config` inside `ND_APP_DATA_DIR`, else `~/Library/Application
/// Support/<app>` on macOS, `%APPDATA%\<app>` on Windows and `$XDG_CONFIG_HOME/<app>`
/// (`~/.config/<app>`) elsewhere.
pub fn config_dir(app: &str) -> PathBuf {
    if let Some(dir) = std::env::var_os("ND_APP_CONFIG_DIR").filter(|d| !d.is_empty()) {
        return PathBuf::from(dir);
    }
    if let Some(dir) = std::env::var_os("ND_APP_DATA_DIR").filter(|d| !d.is_empty()) {
        return PathBuf::from(dir).join("config");
    }
    let home = || PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    let base = if cfg!(target_os = "macos") {
        home().join("Library/Application Support")
    } else if cfg!(windows) {
        PathBuf::from(std::env::var_os("APPDATA").unwrap_or_default())
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .filter(|d| !d.is_empty())
            .map_or_else(|| home().join(".config"), PathBuf::from)
    };
    base.join(app)
}

#[derive(Debug)]
pub enum LoadError {
    Read {
        path: PathBuf,
        error: std::io::Error,
    },
    Parse {
        path: PathBuf,
        message: String,
    },
    /// A layer's values do not fit the settings struct, or break a `range`.
    Invalid {
        layer: LayerRef,
        message: String,
    },
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, error } => write!(f, "{}: {error}", path.display()),
            Self::Parse { path, message } => write!(f, "{}: {message}", path.display()),
            Self::Invalid { layer, message } => write!(f, "{layer}: {message}"),
        }
    }
}

impl std::error::Error for LoadError {}

pub(crate) struct Context<'a> {
    pub paths: &'a Paths,
    pub env_prefix: &'a str,
    pub env: &'a [(String, String)],
    pub cli: &'a [(String, String)],
    pub store_roots: &'a [PathBuf],
}

pub(crate) struct Loaded {
    /// Defaults plus every layer, merged. `plugins` is split off.
    pub table: Table,
    pub sources: BTreeMap<String, LayerRef>,
    pub plugins: BTreeMap<String, Table>,
    /// The config files that exist, lowest precedence first.
    pub files: Vec<LayerRef>,
}

/// The files that could hold settings, lowest precedence first, with their kind.
pub(crate) fn candidate_files(ctx: &Context<'_>) -> Vec<(LayerKind, PathBuf)> {
    let mut out = Vec::new();
    if let Some(managed) = &ctx.paths.managed {
        out.push((LayerKind::Nix, managed.clone()));
    }
    out.push((LayerKind::User, ctx.paths.user.clone()));
    let mut drop_ins: Vec<PathBuf> = std::fs::read_dir(&ctx.paths.drop_in_dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "toml"))
        .collect();
    drop_ins.sort();
    out.extend(drop_ins.into_iter().map(|p| (LayerKind::DropIn, p)));
    out.push((LayerKind::Local, ctx.paths.local.clone()));
    out
}

pub(crate) fn load<T: Settings>(
    ctx: &Context<'_>,
    schema: &SettingsSchema,
    substitute: Option<(&Path, &str)>,
) -> Result<Loaded, LoadError> {
    let defaults = Table::try_from(T::default()).map_err(|e| LoadError::Invalid {
        layer: LayerRef { kind: LayerKind::Defaults, origin: None, label: "defaults".into() },
        message: e.to_string(),
    })?;
    let mut layers: Vec<(LayerRef, Table, Option<String>)> = Vec::new();
    let mut plugins: BTreeMap<String, Table> = BTreeMap::new();
    let mut files = Vec::new();

    for (kind, path) in candidate_files(ctx) {
        let text = match substitute {
            Some((target, text)) if target == path => Some(text.to_owned()),
            _ => match std::fs::read_to_string(&path) {
                Ok(text) => Some(text),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(LoadError::Read { path, error }),
            },
        };
        let Some(text) = text else { continue };
        let mut table: Table = text
            .parse()
            .map_err(|e: toml::de::Error| LoadError::Parse { path: path.clone(), message: e.to_string() })?;
        let kind = if file_state(&path, ctx.store_roots) == FileState::NixManaged { LayerKind::Nix } else { kind };
        let layer = LayerRef { kind, origin: Some(path.clone()), label: path.display().to_string() };
        if let Some(Value::Table(sections)) = table.remove("plugins") {
            for (name, section) in sections {
                if let Value::Table(section) = section {
                    merge(plugins.entry(name).or_default(), section);
                }
            }
        }
        files.push(layer.clone());
        layers.push((layer, table, Some(text)));
    }

    let env_layer = env_table(ctx, schema)?;
    if let Some((layer, table)) = env_layer {
        layers.push((layer, table, None));
    }
    let cli_layer = cli_table(ctx, schema)?;
    if let Some((layer, table)) = cli_layer {
        layers.push((layer, table, None));
    }

    let mut merged = defaults.clone();
    let mut sources = BTreeMap::new();
    for (layer, table, _) in &layers {
        merge(&mut merged, table.clone());
        record(&mut sources, "", table, layer);
    }
    match Value::Table(merged.clone()).try_into::<T>() {
        Ok(value) => {
            check_ranges(schema, &merged, &sources)?;
            drop(value);
            Ok(Loaded { table: merged, sources, plugins, files })
        }
        Err(merged_error) => {
            for (layer, table, text) in &layers {
                let mut alone = defaults.clone();
                merge(&mut alone, table.clone());
                if let Err(e) = Value::Table(alone).try_into::<T>() {
                    let message = text
                        .as_deref()
                        .and_then(|t| toml::from_str::<T>(t).err())
                        .map_or_else(|| e.to_string(), |spanned| spanned.to_string());
                    return Err(LoadError::Invalid { layer: layer.clone(), message });
                }
            }
            let layer = LayerRef { kind: LayerKind::Defaults, origin: None, label: "merged settings".into() };
            Err(LoadError::Invalid { layer, message: merged_error.to_string() })
        }
    }
}

fn check_ranges(
    schema: &SettingsSchema,
    merged: &Table,
    sources: &BTreeMap<String, LayerRef>,
) -> Result<(), LoadError> {
    for meta in schema.all_meta() {
        let Some((lo, hi)) = meta.range else { continue };
        let number = match get_path(merged, &meta.path) {
            Some(Value::Integer(i)) => *i as f64,
            Some(Value::Float(f)) => *f,
            _ => continue,
        };
        if number < lo || number > hi {
            let layer = sources.get(&meta.path).cloned().unwrap_or(LayerRef {
                kind: LayerKind::Defaults,
                origin: None,
                label: "defaults".into(),
            });
            return Err(LoadError::Invalid {
                layer,
                message: format!("`{}` is {number}, allowed {lo} to {hi}", meta.path),
            });
        }
    }
    Ok(())
}

/// Deep merge: tables merge key by key, everything else is replaced.
pub(crate) fn merge(into: &mut Table, from: Table) {
    for (key, value) in from {
        match (into.get_mut(&key), value) {
            (Some(Value::Table(existing)), Value::Table(incoming)) => merge(existing, incoming),
            (_, value) => {
                into.insert(key, value);
            }
        }
    }
}

fn record(sources: &mut BTreeMap<String, LayerRef>, prefix: &str, table: &Table, layer: &LayerRef) {
    for (key, value) in table {
        let path = if prefix.is_empty() { key.clone() } else { format!("{prefix}.{key}") };
        match value {
            Value::Table(inner) => record(sources, &path, inner, layer),
            _ => {
                sources.insert(path, layer.clone());
            }
        }
    }
}

pub(crate) fn get_path<'a>(table: &'a Table, path: &str) -> Option<&'a Value> {
    let mut segments = path.split('.');
    let mut value = table.get(segments.next()?)?;
    for segment in segments {
        value = value.as_table()?.get(segment)?;
    }
    Some(value)
}

fn set_path(table: &mut Table, path: &str, value: Value) {
    let segments: Vec<&str> = path.split('.').collect();
    let (last, parents) = segments.split_last().expect("non-empty path");
    let mut current = table;
    for segment in parents {
        let entry = current.entry(*segment).or_insert_with(|| Value::Table(Table::new()));
        if !entry.is_table() {
            *entry = Value::Table(Table::new());
        }
        current = entry.as_table_mut().expect("just made a table");
    }
    current.insert((*last).to_owned(), value);
}

/// `NOTES_EDITOR__FONT_SIZE` sets `editor.font_size`: `__` nests, a single `_` stays in the name.
/// Variables that name no known key are ignored, since other settings share the prefix.
fn env_table(ctx: &Context<'_>, schema: &SettingsSchema) -> Result<Option<(LayerRef, Table)>, LoadError> {
    let mut table = Table::new();
    let mut names = Vec::new();
    for field in &schema.fields {
        let wanted = field.path.replace('.', "__").replace('-', "_").to_uppercase();
        let Some((name, raw)) = ctx.env.iter().find(|(k, _)| k.strip_prefix(ctx.env_prefix) == Some(wanted.as_str()))
        else {
            continue;
        };
        let value = parse_raw(&field.kind, raw).map_err(|message| LoadError::Invalid {
            layer: LayerRef { kind: LayerKind::Env, origin: None, label: format!("environment variable {name}") },
            message,
        })?;
        set_path(&mut table, &field.path, value);
        names.push(name.clone());
    }
    if names.is_empty() {
        return Ok(None);
    }
    let label = format!("environment ({})", names.join(", "));
    Ok(Some((LayerRef { kind: LayerKind::Env, origin: None, label }, table)))
}

fn cli_table(ctx: &Context<'_>, schema: &SettingsSchema) -> Result<Option<(LayerRef, Table)>, LoadError> {
    if ctx.cli.is_empty() {
        return Ok(None);
    }
    let layer = LayerRef { kind: LayerKind::Cli, origin: None, label: "command line".into() };
    let mut table = Table::new();
    for (path, raw) in ctx.cli {
        let field = schema
            .field(path)
            .ok_or_else(|| LoadError::Invalid { layer: layer.clone(), message: format!("unknown setting `{path}`") })?;
        let value = parse_raw(&field.kind, raw)
            .map_err(|message| LoadError::Invalid { layer: layer.clone(), message: format!("`{path}`: {message}") })?;
        set_path(&mut table, path, value);
    }
    Ok(Some((layer, table)))
}

/// Reads text from the environment or command line as the type the field declares.
pub(crate) fn parse_raw(kind: &FieldKind, raw: &str) -> Result<Value, String> {
    let bad = |what: &str| format!("`{raw}` is not {what}");
    match kind {
        FieldKind::Toggle => match raw.to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" | "on" => Ok(Value::Boolean(true)),
            "false" | "0" | "no" | "off" => Ok(Value::Boolean(false)),
            _ => Err(bad("a boolean")),
        },
        FieldKind::Integer { .. } => raw.trim().parse().map(Value::Integer).map_err(|_| bad("an integer")),
        FieldKind::Number { .. } => raw.trim().parse().map(Value::Float).map_err(|_| bad("a number")),
        FieldKind::Text | FieldKind::Choice(_) => Ok(Value::String(raw.to_owned())),
        FieldKind::List(_) | FieldKind::Map | FieldKind::Raw => format!("v = {raw}")
            .parse::<Table>()
            .ok()
            .and_then(|mut t| t.remove("v"))
            .ok_or_else(|| bad("a TOML value")),
    }
}
