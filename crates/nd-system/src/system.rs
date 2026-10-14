use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use futures_channel::oneshot;

use crate::backend::{AudioControl, Backend, Emitter, Reply};
use crate::bus::{Bus, Events};
use crate::fake::{FakeBackend, FakeHandle};
use crate::script::DialogScript;
use crate::{
    ClipboardImage, Error, MessageOptions, Notification, NotificationId, NowPlaying, OpenOptions, PlayOptions,
    PlayerId, Result, SaveOptions, SystemEvent, Tray, TrayId,
};

/// A call in flight. Resolves once the OS answers; dropping it abandons the answer, not the
/// request (a dialog stays open).
pub type Pending<T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'static>>;

struct Inner {
    backend: Box<dyn Backend>,
    bus: Arc<Bus>,
    script: Option<DialogScript>,
    next_id: AtomicU64,
}

/// The app's OS services. Cheap to clone, `Send` and `Sync`.
#[derive(Clone)]
pub struct System(Arc<Inner>);

static NATIVE: OnceLock<System> = OnceLock::new();
static APP: OnceLock<AppIdentity> = OnceLock::new();

/// Who the app is to the OS services that need a name: the MPRIS bus name, the recent
/// documents entry, the notification sender on Linux.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppIdentity {
    /// Reverse DNS, `dev.formalsnake.notes`.
    pub id: String,
    /// The `Cargo.toml` package name; the app's directories are named after it.
    pub name: String,
    pub display_name: String,
}

impl AppIdentity {
    fn from_process() -> Self {
        let exe = std::env::current_exe().ok();
        let stem = exe
            .as_deref()
            .and_then(Path::file_stem)
            .map_or_else(|| "nd-app".into(), |s| s.to_string_lossy().into_owned());
        let id = std::env::var("ND_APP_ID").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| format!("dev.nd.{stem}"));
        Self { id, display_name: stem.clone(), name: stem }
    }
}

/// Names the app for [`System::native`]; the `nd` facade calls this from the manifest before
/// the app's `init`. Only the first call counts.
pub fn set_app_identity(identity: AppIdentity) {
    let _ = APP.set(identity);
}

/// The identity from [`set_app_identity`], else one derived from the executable's name and
/// `ND_APP_ID`.
pub fn app_identity() -> &'static AppIdentity {
    APP.get_or_init(AppIdentity::from_process)
}

impl System {
    /// The process's OS services.
    pub fn native() -> System {
        NATIVE.get_or_init(|| System::with_backend(crate::native::backend(app_identity()))).clone()
    }

    /// Services over `backend`. Dialogs answer from `ND_AUTOMATION_DIALOG_SCRIPT` when it is
    /// set, as on the native system.
    pub fn with_backend(backend: impl Backend) -> System {
        let bus = Arc::new(Bus::new());
        backend.attach(Emitter(Arc::downgrade(&bus)));
        let script = DialogScript::from_env();
        System(Arc::new(Inner { backend: Box::new(backend), bus, script, next_id: AtomicU64::new(1) }))
    }

    /// Services over a [`FakeBackend`], and the handle that scripts and inspects it.
    pub fn fake() -> (System, FakeHandle) {
        let backend = FakeBackend::new();
        let handle = backend.handle();
        let system = System::with_backend(backend);
        (system, handle)
    }

    /// Replaces the dialog script (`None` shows real dialogs again).
    pub fn with_dialog_script(self, script: Option<DialogScript>) -> System {
        let inner = Arc::try_unwrap(self.0).unwrap_or_else(|_| panic!("with_dialog_script on a shared System"));
        System(Arc::new(Inner { script, ..inner }))
    }

    fn call<T: Send + 'static>(&self, f: impl FnOnce(&dyn Backend, Reply<T>)) -> Pending<T> {
        let (reply, rx) = Reply::new();
        f(&*self.0.backend, reply);
        Box::pin(answer(rx))
    }

    fn next_id(&self) -> u64 {
        self.0.next_id.fetch_add(1, Ordering::Relaxed)
    }

    /// Subscribes to OS events. Open requests from before the first subscription are replayed
    /// to it.
    pub fn events(&self) -> Events {
        self.0.bus.subscribe()
    }

    /// Delivers `event` to subscribers as if the OS had sent it. Backends that learn of events
    /// elsewhere (the window hosts' file drops) report through here.
    pub fn emit(&self, event: SystemEvent) {
        self.0.bus.emit(event);
    }

    /// Reports what a command line asks the app to open, as Linux and Windows deliver a
    /// registered URL scheme or document type (`app %u`): arguments that parse as a URL with a
    /// scheme become [`SystemEvent::OpenUrls`], existing paths [`SystemEvent::OpenFiles`];
    /// flags and anything else are left to the app. The `nd` facade passes the launch's own
    /// arguments; a single-instance app passes the ones a second launch forwarded.
    pub fn open_from_args<I, S>(&self, args: I)
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        let (mut urls, mut files) = (Vec::new(), Vec::new());
        for arg in args {
            let arg = arg.as_ref();
            let text = arg.to_string_lossy();
            if text.starts_with('-') {
                continue;
            }
            if is_url(&text) {
                urls.push(text.into_owned());
            } else if let Ok(path) = std::fs::canonicalize(arg) {
                files.push(path);
            }
        }
        if !urls.is_empty() {
            self.emit(SystemEvent::OpenUrls(urls));
        }
        if !files.is_empty() {
            self.emit(SystemEvent::OpenFiles(files));
        }
    }

    /// Whether the app is the active app, as of the last activation event. True until the
    /// first one arrives.
    pub fn is_active(&self) -> bool {
        self.0.bus.is_active()
    }

    pub fn dialogs(&self) -> Dialogs<'_> {
        Dialogs(self)
    }

    pub fn clipboard(&self) -> Clipboard<'_> {
        Clipboard(self)
    }

    pub fn notifications(&self) -> Notifications<'_> {
        Notifications(self)
    }

    pub fn recent_documents(&self) -> RecentDocuments<'_> {
        RecentDocuments(self)
    }

    pub fn credentials(&self) -> Credentials<'_> {
        Credentials(self)
    }

    pub fn shell(&self) -> Shell<'_> {
        Shell(self)
    }

    pub fn tray(&self) -> Trays<'_> {
        Trays(self)
    }

    pub fn media(&self) -> Media<'_> {
        Media(self)
    }

    pub fn audio(&self) -> Audio<'_> {
        Audio(self)
    }
}

/// `scheme:rest` with an RFC 3986 scheme of two or more characters, so a Windows drive letter
/// (`C:\x`) stays a path.
fn is_url(s: &str) -> bool {
    let Some((scheme, rest)) = s.split_once(':') else { return false };
    scheme.len() > 1
        && !rest.is_empty()
        && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        && scheme.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

async fn answer<T>(rx: oneshot::Receiver<Result<T>>) -> Result<T> {
    rx.await.unwrap_or(Err(Error::Dropped))
}

/// App-level file pickers and alerts, not attached to a window. A window's own (a sheet on
/// macOS) are `nd::components` dialogs.
pub struct Dialogs<'a>(&'a System);

impl Dialogs<'_> {
    /// The chosen paths, or `None` when the user cancelled.
    pub fn open(&self, options: OpenOptions) -> Pending<Option<Vec<PathBuf>>> {
        match self.0.0.script.as_ref().and_then(|s| s.open()) {
            Some(answer) => self.0.call(|_, r| r.send(answer)),
            None => self.0.call(|b, r| b.open_dialog(options, r)),
        }
    }

    /// The chosen path, or `None` when the user cancelled.
    pub fn save(&self, options: SaveOptions) -> Pending<Option<PathBuf>> {
        match self.0.0.script.as_ref().and_then(|s| s.save()) {
            Some(answer) => self.0.call(|_, r| r.send(answer)),
            None => self.0.call(|b, r| b.save_dialog(options, r)),
        }
    }

    /// The index of the button picked.
    pub fn message(&self, options: MessageOptions) -> Pending<usize> {
        if options.buttons.len() > 3 {
            let e = Error::Invalid(format!("{} alert buttons, platforms show at most 3", options.buttons.len()));
            return self.0.call(|_, r| r.err(e));
        }
        match self.0.0.script.as_ref().and_then(|s| s.message()) {
            Some(answer) => self.0.call(|_, r| r.send(answer)),
            None => self.0.call(|b, r| b.message(options, r)),
        }
    }
}

pub struct Clipboard<'a>(&'a System);

impl Clipboard<'_> {
    /// The clipboard's text, or `None` when it holds none.
    pub fn read_text(&self) -> Pending<Option<String>> {
        self.0.call(|b, r| b.clipboard_read_text(r))
    }

    pub fn write_text(&self, text: impl Into<String>) -> Pending<()> {
        let text = text.into();
        self.0.call(|b, r| b.clipboard_write_text(text, r))
    }

    /// The clipboard's image, or `None` when it holds none.
    pub fn read_image(&self) -> Pending<Option<ClipboardImage>> {
        self.0.call(|b, r| b.clipboard_read_image(r))
    }

    pub fn write_image(&self, image: ClipboardImage) -> Pending<()> {
        if image.rgba.len() != image.width * image.height * 4 {
            return self.0.call(|_, r| r.err(Error::Invalid("rgba length is not width * height * 4".into())));
        }
        self.0.call(|b, r| b.clipboard_write_image(image, r))
    }

    /// Files copied in the file manager, or `None` when the clipboard holds none.
    pub fn read_files(&self) -> Pending<Option<Vec<PathBuf>>> {
        self.0.call(|b, r| b.clipboard_read_files(r))
    }

    pub fn write_files(&self, files: Vec<PathBuf>) -> Pending<()> {
        self.0.call(|b, r| b.clipboard_write_files(files, r))
    }
}

pub struct Notifications<'a>(&'a System);

impl Notifications<'_> {
    /// Shows `notification`; its id comes back on [`SystemEvent::NotificationClicked`] with
    /// the notification's `data`.
    pub fn show(&self, mut notification: Notification) -> Pending<NotificationId> {
        let id = NotificationId(format!("nd-{}-{}", std::process::id(), self.0.next_id()));
        if let Some(data) = notification.data.take() {
            self.0.0.bus.keep_data(id.clone(), data);
        }
        let call = self.0.call(|b, r| b.notify(id.clone(), notification, r));
        Box::pin(async move { call.await.map(|()| id) })
    }
}

pub struct RecentDocuments<'a>(&'a System);

impl RecentDocuments<'_> {
    pub fn add(&self, path: impl Into<PathBuf>) -> Pending<()> {
        let path = path.into();
        self.0.call(|b, r| b.recent_add(path, r))
    }

    /// Clears the app's recent documents.
    pub fn clear(&self) -> Pending<()> {
        self.0.call(|b, r| b.recent_clear(r))
    }
}

/// Secrets in the OS store: Keychain on macOS, the Secret Service on Linux, Credential Manager
/// on Windows.
pub struct Credentials<'a>(&'a System);

impl Credentials<'_> {
    pub fn set(
        &self,
        service: impl Into<String>,
        account: impl Into<String>,
        secret: impl Into<String>,
    ) -> Pending<()> {
        let (service, account, secret) = (service.into(), account.into(), secret.into());
        self.0.call(|b, r| b.credential_set(service, account, secret, r))
    }

    /// The secret, or `None` when none is stored.
    pub fn get(&self, service: impl Into<String>, account: impl Into<String>) -> Pending<Option<String>> {
        let (service, account) = (service.into(), account.into());
        self.0.call(|b, r| b.credential_get(service, account, r))
    }

    /// Deletes the secret; deleting one that is not there succeeds.
    pub fn delete(&self, service: impl Into<String>, account: impl Into<String>) -> Pending<()> {
        let (service, account) = (service.into(), account.into());
        self.0.call(|b, r| b.credential_delete(service, account, r))
    }
}

/// Hands URLs and paths to the OS. These return once the request is handed over, not when the
/// other app has opened.
pub struct Shell<'a>(&'a System);

impl Shell<'_> {
    pub fn open_url(&self, url: &str) -> Result<()> {
        self.0.0.backend.open_url(url)
    }

    /// Opens `path` in its default app.
    pub fn open_path(&self, path: impl AsRef<Path>) -> Result<()> {
        self.0.0.backend.open_path(existing(path.as_ref())?)
    }

    /// Shows `path` selected in Finder or the file manager.
    pub fn reveal_path(&self, path: impl AsRef<Path>) -> Result<()> {
        self.0.0.backend.reveal_path(existing(path.as_ref())?)
    }
}

fn existing(path: &Path) -> Result<PathBuf> {
    if cfg!(target_arch = "wasm32") {
        return Ok(path.to_owned());
    }
    std::fs::canonicalize(path).map_err(|e| Error::Invalid(format!("{}: {e}", path.display())))
}

pub struct Trays<'a>(&'a System);

impl Trays<'_> {
    /// Shows `tray`, or updates the item already showing under its id.
    pub fn set(&self, tray: Tray) -> Pending<()> {
        self.0.call(|b, r| b.tray_set(tray, r))
    }

    pub fn remove(&self, id: TrayId) -> Pending<()> {
        self.0.call(|b, r| b.tray_remove(id, r))
    }
}

/// The OS media controls. Presses arrive as [`SystemEvent::Media`].
pub struct Media<'a>(&'a System);

impl Media<'_> {
    pub fn set(&self, now: NowPlaying) -> Pending<()> {
        self.0.call(|b, r| b.media_set(Some(now), r))
    }

    pub fn clear(&self) -> Pending<()> {
        self.0.call(|b, r| b.media_set(None, r))
    }
}

pub struct Audio<'a>(&'a System);

impl Audio<'_> {
    /// Starts playing; state changes and spectrum frames arrive as [`SystemEvent::Audio`].
    pub fn play(&self, options: PlayOptions) -> Pending<Player> {
        let id = PlayerId(self.0.next_id());
        let system = self.0.clone();
        let call = self.0.call(|b, r| b.audio_play(id, options, r));
        Box::pin(async move { call.await.map(|()| Player { id, system }) })
    }
}

/// A sound from [`Audio::play`]. Dropping the handle leaves it playing; [`Player::stop`]
/// releases it.
#[derive(Clone)]
pub struct Player {
    id: PlayerId,
    system: System,
}

impl Player {
    pub fn id(&self) -> PlayerId {
        self.id
    }

    fn control(&self, control: AudioControl) -> Pending<()> {
        let id = self.id;
        self.system.call(|b, r| b.audio_control(id, control, r))
    }

    pub fn pause(&self) -> Pending<()> {
        self.control(AudioControl::Pause)
    }

    pub fn resume(&self) -> Pending<()> {
        self.control(AudioControl::Resume)
    }

    pub fn stop(&self) -> Pending<()> {
        self.control(AudioControl::Stop)
    }

    pub fn seek(&self, position: Duration) -> Pending<()> {
        self.control(AudioControl::Seek(position))
    }

    pub fn set_volume(&self, volume: f32) -> Pending<()> {
        self.control(AudioControl::SetVolume(volume.clamp(0., 1.)))
    }
}
