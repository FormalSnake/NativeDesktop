use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use tokio::runtime::Handle;
use tokio::task::AbortHandle;

/// App-published values, fetched by type. The app inserts before plugins start; typical
/// entries are a `tokio::sync::watch::Receiver<NowPlaying>` to observe or an
/// `mpsc::Sender<AppCommand>` to drive the app.
#[derive(Clone, Default)]
pub struct Services {
    map: Arc<Mutex<HashMap<TypeId, Arc<dyn Any + Send + Sync>>>>,
}

fn locked<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Services {
    pub fn new() -> Self {
        Self::default()
    }

    /// Replaces any earlier value of the same type.
    pub fn insert<T: Any + Send + Sync>(&self, value: T) {
        locked(&self.map).insert(TypeId::of::<T>(), Arc::new(value));
    }

    pub fn get<T: Any + Send + Sync>(&self) -> Option<Arc<T>> {
        let any = locked(&self.map).get(&TypeId::of::<T>()).cloned()?;
        any.downcast::<T>().ok()
    }
}

/// What the host hands a plugin: identity of the app, the async runtime, a private data
/// directory and the app's [`Services`]. One per plugin.
#[derive(Clone)]
pub struct PluginContext {
    app: String,
    plugin: &'static str,
    handle: Handle,
    services: Services,
    tasks: Arc<Mutex<Vec<AbortHandle>>>,
}

impl PluginContext {
    pub(crate) fn new(app: &str, plugin: &'static str, handle: Handle, services: Services) -> Self {
        Self { app: app.to_owned(), plugin, handle, services, tasks: Arc::default() }
    }

    pub fn app_name(&self) -> &str {
        &self.app
    }

    pub fn plugin_name(&self) -> &'static str {
        self.plugin
    }

    pub fn services(&self) -> &Services {
        &self.services
    }

    pub fn runtime(&self) -> &Handle {
        &self.handle
    }

    /// `<app data dir>/plugins/<name>`, created on first call.
    pub fn data_dir(&self) -> std::io::Result<PathBuf> {
        let dir = nd_core::paths::app_data_dir(&self.app).join("plugins").join(self.plugin);
        std::fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    /// Runs `fut` on the app's runtime. The host aborts it on shutdown and on a config reload.
    pub fn spawn(&self, fut: impl Future<Output = ()> + Send + 'static) {
        let abort = self.handle.spawn(fut).abort_handle();
        let mut tasks = locked(&self.tasks);
        tasks.retain(|t| !t.is_finished());
        tasks.push(abort);
    }

    pub fn stop_tasks(&self) {
        for task in locked(&self.tasks).drain(..) {
            task.abort();
        }
    }
}
