//! The live settings of one app: current value, where each key came from, which keys the user
//! may edit, and edits that keep the file's comments.

use std::collections::BTreeSet;
use std::fmt;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

use serde::Serialize;
use toml::{Table, Value};

use crate::edit::{FileState, edit_document, file_state, write_atomic};
use crate::layers::{self, Context, LayerKind, LayerRef, LoadError, Loaded, Paths, candidate_files};
use crate::schema::SettingsSchema;
use crate::{Reload, Settings, nix};

/// How to find and read one app's settings.
#[derive(Clone, Debug)]
pub struct Options {
    app: String,
    paths: Paths,
    env_prefix: String,
    env: Option<Vec<(String, String)>>,
    cli: Vec<(String, String)>,
    store_roots: Vec<PathBuf>,
    plugin_defaults: Table,
}

impl Options {
    /// Files in the platform config directory (`layers::config_dir`), environment variables
    /// `<APP>_*`.
    pub fn new(app: &str) -> Self {
        Self {
            app: app.to_owned(),
            paths: Paths::for_app(app),
            env_prefix: format!("{}_", app.to_uppercase().replace('-', "_")),
            env: None,
            cli: Vec::new(),
            store_roots: vec![PathBuf::from("/nix/store")],
            plugin_defaults: Table::new(),
        }
    }

    pub fn paths(mut self, paths: Paths) -> Self {
        self.paths = paths;
        self
    }

    /// Command-line overrides as `(dotted key, raw text)`, highest precedence.
    pub fn cli(mut self, overrides: Vec<(String, String)>) -> Self {
        self.cli = overrides;
        self
    }

    /// Replaces the process environment, for tests.
    pub fn env(mut self, vars: Vec<(String, String)>) -> Self {
        self.env = Some(vars);
        self
    }

    /// `PluginHost::default_sections()`. Plugin sections load over these, and the settings
    /// schema gains one typed field per plugin key.
    pub fn plugin_defaults(mut self, defaults: Table) -> Self {
        self.plugin_defaults = defaults;
        self
    }

    /// Directories whose contents count as Nix-managed. Defaults to `/nix/store`.
    pub fn store_roots(mut self, roots: Vec<PathBuf>) -> Self {
        self.store_roots = roots;
        self
    }
}

/// Who may edit a key, and why not.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyState {
    Editable,
    /// Set by a Nix-managed file. `option` is the home-manager option path.
    ManagedByNix {
        option: String,
        file: Option<PathBuf>,
    },
    /// A higher layer wins over anything an edit could write.
    Overridden {
        by: LayerRef,
    },
}

/// The file an edit lands in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WriteTarget {
    pub path: PathBuf,
    /// The writable overlay, because the main file is Nix-managed or read-only.
    pub overlay: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// The app's own write.
    Own,
    /// The watcher saw someone else change a file.
    External,
}

/// A reload that changed something.
#[derive(Clone, Debug)]
pub struct Change<T> {
    pub old: Arc<T>,
    pub new: Arc<T>,
    /// Dotted keys whose value differs.
    pub changed: Vec<String>,
    /// The subset of `changed` declared `reload = "restart"`.
    pub restart_required: Vec<String>,
    pub origin: Origin,
}

#[derive(Clone, Debug)]
pub enum Event<T> {
    Changed(Change<T>),
    /// The files no longer load. The last good settings stay in force.
    Rejected(String),
}

#[derive(Debug)]
pub enum WriteError {
    /// The key is set by a Nix-managed file; edit `option` in the Nix configuration instead.
    ManagedByNix {
        option: String,
    },
    Overridden {
        by: LayerRef,
    },
    UnknownKey(String),
    /// The edit would leave settings that do not load; nothing was written.
    Invalid(LoadError),
    Io(std::io::Error),
}

impl fmt::Display for WriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ManagedByNix { option } => write!(f, "managed by Nix, change `{option}`"),
            Self::Overridden { by } => write!(f, "overridden by {by}"),
            Self::UnknownKey(key) => write!(f, "no setting named `{key}`"),
            Self::Invalid(e) => write!(f, "{e}"),
            Self::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for WriteError {}

struct State<T> {
    loaded: Loaded,
    value: Arc<T>,
    /// Hash of every config file's content and state when `loaded` was built. Equal fingerprint
    /// means nothing to reload, which is also how the watcher ignores the app's own saves.
    fingerprint: u64,
}

type Listener<T> = Box<dyn Fn(&Event<T>) + Send + Sync>;

pub(crate) struct Inner<T: Settings> {
    opts: Options,
    schema: SettingsSchema,
    state: RwLock<State<T>>,
    /// Serializes writes and reloads.
    gate: Mutex<()>,
    listeners: Mutex<Vec<Listener<T>>>,
}

pub struct SettingsHandle<T: Settings> {
    pub(crate) inner: Arc<Inner<T>>,
}

impl<T: Settings> Clone for SettingsHandle<T> {
    fn clone(&self) -> Self {
        Self { inner: Arc::clone(&self.inner) }
    }
}

impl<T: Settings> SettingsHandle<T> {
    /// Reads every layer. Fails with the file, key and line when something does not load; apps
    /// that prefer to start anyway fall back to `T::default()`.
    pub fn load(opts: Options) -> Result<Self, LoadError> {
        let schema = SettingsSchema::of::<T>().with_plugins(&opts.plugin_defaults);
        let env = opts.env.clone().unwrap_or_else(|| std::env::vars().collect());
        let ctx = context(&opts, &env);
        let loaded = layers::load::<T>(&ctx, &schema, None)?;
        let fingerprint = fingerprint(&ctx);
        let value = Arc::new(from_table(&loaded.table));
        Ok(Self {
            inner: Arc::new(Inner {
                opts,
                schema,
                state: RwLock::new(State { loaded, value, fingerprint }),
                gate: Mutex::new(()),
                listeners: Mutex::new(Vec::new()),
            }),
        })
    }

    /// The settings now in force.
    pub fn get(&self) -> Arc<T> {
        Arc::clone(&self.inner.state.read().expect("settings lock").value)
    }

    /// The metadata a settings form renders.
    pub fn schema(&self) -> &SettingsSchema {
        &self.inner.schema
    }

    /// A root table holding only `plugins`, in the shape `PluginHost::load` and `reload` take:
    /// defaults first, then the Nix-managed file, the user file and the rest.
    pub fn plugins_root(&self) -> Table {
        let mut root = self.inner.state.read().expect("settings lock").loaded.combined();
        root.retain(|k, _| k == "plugins");
        root
    }

    /// The `[plugins.<name>]` section, split off before the app's struct parses.
    pub fn plugin_section(&self, name: &str) -> Option<Table> {
        self.inner.state.read().expect("settings lock").loaded.plugins.get(name).cloned()
    }

    /// The layer whose value wins for `key`; `None` when only the default applies.
    pub fn source(&self, key: &str) -> Option<LayerRef> {
        self.inner.state.read().expect("settings lock").loaded.sources.get(key).cloned()
    }

    /// The config files that exist, lowest precedence first.
    pub fn files(&self) -> Vec<LayerRef> {
        self.inner.state.read().expect("settings lock").loaded.files.clone()
    }

    /// Where an edit would be written right now.
    pub fn write_target(&self) -> WriteTarget {
        let ctx_paths = &self.inner.opts.paths;
        match file_state(&ctx_paths.user, &self.inner.opts.store_roots) {
            FileState::Missing | FileState::Writable => WriteTarget { path: ctx_paths.user.clone(), overlay: false },
            FileState::NixManaged | FileState::ReadOnly => WriteTarget { path: ctx_paths.local.clone(), overlay: true },
        }
    }

    /// Whether the main config file is a Nix-managed symlink (or otherwise read-only).
    pub fn main_file_read_only(&self) -> bool {
        self.write_target().overlay
    }

    pub fn key_state(&self, key: &str) -> KeyState {
        let Some(source) = self.source(key) else { return KeyState::Editable };
        let target_rank = if self.write_target().overlay { LayerKind::Local } else { LayerKind::User };
        match source.kind {
            LayerKind::Nix => KeyState::ManagedByNix { option: self.nix_option(key), file: source.origin },
            kind if kind > target_rank => KeyState::Overridden { by: source },
            _ => KeyState::Editable,
        }
    }

    /// `programs.<app>.settings.<key>`, the home-manager option for `key`.
    pub fn nix_option(&self, key: &str) -> String {
        nix::option_path(&self.inner.opts.app, key)
    }

    /// The Nix expression that sets `key` to its current value ("Copy as Nix").
    pub fn copy_as_nix(&self, key: &str) -> Option<String> {
        let state = self.inner.state.read().expect("settings lock");
        let combined = state.loaded.combined();
        let value = layers::get_path(&combined, key)?;
        Some(format!("{} = {};", self.nix_option(key), nix::literal(value)))
    }

    /// Sets `key` to `value` in the write target, keeping comments and order.
    pub fn set<V: Serialize>(&self, key: &str, value: &V) -> Result<(), WriteError> {
        let value = Value::try_from(value)
            .map_err(|e| WriteError::Invalid(LoadError::Parse { path: PathBuf::from(key), message: e.to_string() }))?;
        self.write(key, Some(value))
    }

    /// Removes `key` from the write target, so a lower layer or the default applies again.
    pub fn unset(&self, key: &str) -> Result<(), WriteError> {
        self.write(key, None)
    }

    fn write(&self, key: &str, value: Option<Value>) -> Result<(), WriteError> {
        let known = self.inner.schema.field(key).is_some() || key.starts_with("plugins.");
        if !known {
            return Err(WriteError::UnknownKey(key.to_owned()));
        }
        match self.key_state(key) {
            KeyState::ManagedByNix { option, .. } => return Err(WriteError::ManagedByNix { option }),
            KeyState::Overridden { by } => return Err(WriteError::Overridden { by }),
            KeyState::Editable => {}
        }
        let _gate = self.inner.gate.lock().expect("settings gate");
        let target = self.write_target();
        let current = match crate::vfs::read_to_string(&target.path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(WriteError::Io(e)),
        };
        let edited = edit_document(&current, key, value.as_ref())
            .map_err(|message| WriteError::Invalid(LoadError::Parse { path: target.path.clone(), message }))?;
        let env = self.inner.opts.env.clone().unwrap_or_else(|| std::env::vars().collect());
        let ctx = context(&self.inner.opts, &env);
        let loaded =
            layers::load::<T>(&ctx, &self.inner.schema, Some((&target.path, &edited))).map_err(WriteError::Invalid)?;
        write_atomic(&target.path, &edited).map_err(WriteError::Io)?;
        let fingerprint = fingerprint(&ctx);
        self.install(loaded, fingerprint, Origin::Own);
        Ok(())
    }

    /// Re-reads the files now. The watcher calls this after a debounce; apps rarely need to.
    /// `None` when nothing changed.
    pub fn reload(&self) -> Option<Event<T>> {
        self.reload_as(Origin::External)
    }

    pub(crate) fn reload_as(&self, origin: Origin) -> Option<Event<T>> {
        let _gate = self.inner.gate.lock().expect("settings gate");
        let env = self.inner.opts.env.clone().unwrap_or_else(|| std::env::vars().collect());
        let ctx = context(&self.inner.opts, &env);
        let fingerprint = fingerprint(&ctx);
        if self.inner.state.read().expect("settings lock").fingerprint == fingerprint {
            return None;
        }
        let event = match layers::load::<T>(&ctx, &self.inner.schema, None) {
            Ok(loaded) => self.install(loaded, fingerprint, origin),
            Err(e) => {
                // Remember the bad content too, so one broken save reports once.
                self.inner.state.write().expect("settings lock").fingerprint = fingerprint;
                let event = Event::Rejected(e.to_string());
                self.emit(&event);
                event
            }
        };
        Some(event)
    }

    fn install(&self, loaded: Loaded, fingerprint: u64, origin: Origin) -> Event<T> {
        let new = Arc::new(from_table(&loaded.table));
        let (old, changed) = {
            let mut state = self.inner.state.write().expect("settings lock");
            let changed = diff(&state.loaded.combined(), &loaded.combined());
            let old = Arc::clone(&state.value);
            *state = State { loaded, value: Arc::clone(&new), fingerprint };
            (old, changed)
        };
        let restart_required = changed
            .iter()
            .filter(|key| self.inner.schema.meta(key).is_some_and(|m| m.reload == Reload::Restart))
            .cloned()
            .collect();
        let event = Event::Changed(Change { old, new, changed, restart_required, origin });
        self.emit(&event);
        event
    }

    fn emit(&self, event: &Event<T>) {
        for listener in self.inner.listeners.lock().expect("listeners").iter() {
            listener(event);
        }
    }

    /// Runs `f` on every change and rejected reload, own writes included (`Change::origin`).
    pub fn on_event(&self, f: impl Fn(&Event<T>) + Send + Sync + 'static) {
        self.inner.listeners.lock().expect("listeners").push(Box::new(f));
    }

    pub(crate) fn watch_dirs(&self) -> Vec<PathBuf> {
        let paths = &self.inner.opts.paths;
        let mut dirs = BTreeSet::new();
        for file in [Some(&paths.user), Some(&paths.local), paths.managed.as_ref()].into_iter().flatten() {
            if let Some(dir) = file.parent() {
                dirs.insert(dir.to_owned());
            }
        }
        dirs.insert(paths.drop_in_dir.clone());
        dirs.into_iter().collect()
    }

    /// Whether a changed path can affect the settings.
    pub(crate) fn is_config_path(&self, path: &Path) -> bool {
        let paths = &self.inner.opts.paths;
        let name = path.file_name();
        let named = |p: &Path| p.file_name() == name;
        named(&paths.user)
            || named(&paths.local)
            || paths.managed.as_deref().is_some_and(named)
            || (path.extension().is_some_and(|e| e == "toml")
                && path.parent().and_then(Path::file_name) == paths.drop_in_dir.file_name())
    }
}

fn context<'a>(opts: &'a Options, env: &'a [(String, String)]) -> Context<'a> {
    Context {
        paths: &opts.paths,
        env_prefix: &opts.env_prefix,
        env,
        cli: &opts.cli,
        store_roots: &opts.store_roots,
        plugin_defaults: &opts.plugin_defaults,
    }
}

/// `T` has no `Clone` bound; the merged table already loaded once, so it loads again.
fn from_table<T: Settings>(table: &Table) -> T {
    Value::Table(table.clone()).try_into().expect("merged settings load")
}

fn fingerprint(ctx: &Context<'_>) -> u64 {
    let mut hasher = DefaultHasher::new();
    for (_, path) in candidate_files(ctx) {
        path.hash(&mut hasher);
        crate::vfs::read(&path).ok().hash(&mut hasher);
        (file_state(&path, ctx.store_roots) as u8).hash(&mut hasher);
    }
    hasher.finish()
}

fn diff(old: &Table, new: &Table) -> Vec<String> {
    let mut out = Vec::new();
    diff_into(&mut out, "", old, new);
    out
}

fn diff_into(out: &mut Vec<String>, prefix: &str, old: &Table, new: &Table) {
    let keys: BTreeSet<&String> = old.keys().chain(new.keys()).collect();
    for key in keys {
        let path = if prefix.is_empty() { key.clone() } else { format!("{prefix}.{key}") };
        match (old.get(key), new.get(key)) {
            (Some(Value::Table(a)), Some(Value::Table(b))) => diff_into(out, &path, a, b),
            (a, b) if a != b => out.push(path),
            _ => {}
        }
    }
}
