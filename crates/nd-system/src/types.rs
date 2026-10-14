//! The values the services take and answer with. Plain data, so the fake backend can record
//! them and tests can compare them.

use std::path::PathBuf;
use std::time::Duration;

use nd_core::WindowId;
use serde::{Deserialize, Serialize};

/// A file type filter in an open or save dialog.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileFilter {
    pub name: String,
    /// Extensions without the dot.
    pub extensions: Vec<String>,
}

/// What an open dialog picks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PickKind {
    #[default]
    Files,
    Folders,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenOptions {
    pub title: Option<String>,
    pub directory: Option<PathBuf>,
    pub filters: Vec<FileFilter>,
    pub multiple: bool,
    pub pick: PickKind,
}

impl OpenOptions {
    pub fn file() -> Self {
        Self::default()
    }

    pub fn folder() -> Self {
        Self { pick: PickKind::Folders, ..Self::default() }
    }

    pub fn multiple(mut self) -> Self {
        self.multiple = true;
        self
    }

    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn starting_in(mut self, dir: impl Into<PathBuf>) -> Self {
        self.directory = Some(dir.into());
        self
    }

    pub fn filter(mut self, name: impl Into<String>, extensions: &[&str]) -> Self {
        self.filters
            .push(FileFilter { name: name.into(), extensions: extensions.iter().map(|e| (*e).into()).collect() });
        self
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SaveOptions {
    pub title: Option<String>,
    pub directory: Option<PathBuf>,
    pub file_name: Option<String>,
    pub filters: Vec<FileFilter>,
}

impl SaveOptions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn starting_in(mut self, dir: impl Into<PathBuf>) -> Self {
        self.directory = Some(dir.into());
        self
    }

    pub fn file_name(mut self, name: impl Into<String>) -> Self {
        self.file_name = Some(name.into());
        self
    }

    pub fn filter(mut self, name: impl Into<String>, extensions: &[&str]) -> Self {
        self.filters
            .push(FileFilter { name: name.into(), extensions: extensions.iter().map(|e| (*e).into()).collect() });
        self
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageLevel {
    #[default]
    Info,
    Warning,
    Error,
}

/// An app-level alert. The answer is the index of the button picked; with no buttons the
/// platform's OK is shown and answers 0. Dismissing the alert without a button (Escape on a
/// platform that allows it) answers the last button's index.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageOptions {
    pub title: String,
    pub detail: Option<String>,
    pub level: MessageLevel,
    /// At most three: every platform's native alert shows that many.
    pub buttons: Vec<String>,
}

impl MessageOptions {
    pub fn new(title: impl Into<String>) -> Self {
        Self { title: title.into(), ..Self::default() }
    }

    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn level(mut self, level: MessageLevel) -> Self {
        self.level = level;
        self
    }

    pub fn button(mut self, label: impl Into<String>) -> Self {
        self.buttons.push(label.into());
        self
    }
}

/// An image on the clipboard, as straight RGBA rows.
#[derive(Clone, PartialEq, Eq)]
pub struct ClipboardImage {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

impl std::fmt::Debug for ClipboardImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ClipboardImage({}x{}, {} bytes)", self.width, self.height, self.rgba.len())
    }
}

/// A notification's id, used to correlate its click.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct NotificationId(pub String);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationAction {
    pub id: String,
    pub title: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Notification {
    pub title: String,
    pub subtitle: Option<String>,
    pub body: Option<String>,
    /// Buttons on the notification; a click on one arrives with its id.
    pub actions: Vec<NotificationAction>,
    /// Echoed on the click event for this notification, for clicks in this process.
    #[serde(skip)]
    pub data: Option<serde_json::Value>,
    pub silent: bool,
}

impl Notification {
    pub fn new(title: impl Into<String>) -> Self {
        Self { title: title.into(), ..Self::default() }
    }

    pub fn subtitle(mut self, subtitle: impl Into<String>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }

    pub fn body(mut self, body: impl Into<String>) -> Self {
        self.body = Some(body.into());
        self
    }

    pub fn action(mut self, id: impl Into<String>, title: impl Into<String>) -> Self {
        self.actions.push(NotificationAction { id: id.into(), title: title.into() });
        self
    }

    pub fn data(mut self, data: serde_json::Value) -> Self {
        self.data = Some(data);
        self
    }

    pub fn silent(mut self) -> Self {
        self.silent = true;
        self
    }
}

/// A tray item's id, chosen by the app.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TrayId(pub String);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrayIcon {
    /// A symbol name: SF Symbols on macOS, a freedesktop icon name on Linux.
    Named(String),
    /// PNG bytes, drawn as a template image on macOS when `template` is set.
    Png { bytes: Vec<u8>, template: bool },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrayMenuItem {
    Item { id: String, label: String, enabled: bool, checked: Option<bool> },
    Submenu { label: String, items: Vec<TrayMenuItem> },
    Separator,
}

impl TrayMenuItem {
    pub fn item(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self::Item { id: id.into(), label: label.into(), enabled: true, checked: None }
    }

    pub fn check(id: impl Into<String>, label: impl Into<String>, checked: bool) -> Self {
        Self::Item { id: id.into(), label: label.into(), enabled: true, checked: Some(checked) }
    }

    pub fn disabled(mut self) -> Self {
        if let Self::Item { enabled, .. } = &mut self {
            *enabled = false;
        }
        self
    }
}

/// A status item (macOS menu bar extra, StatusNotifierItem on Linux).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tray {
    pub id: TrayId,
    pub icon: Option<TrayIcon>,
    /// Text beside the icon (macOS) or the item's title (Linux).
    pub title: Option<String>,
    pub tooltip: Option<String>,
    pub menu: Vec<TrayMenuItem>,
}

impl Tray {
    pub fn new(id: impl Into<String>) -> Self {
        Self { id: TrayId(id.into()), icon: None, title: None, tooltip: None, menu: Vec::new() }
    }

    pub fn icon(mut self, icon: TrayIcon) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn tooltip(mut self, tooltip: impl Into<String>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }

    pub fn item(mut self, item: TrayMenuItem) -> Self {
        self.menu.push(item);
        self
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlaybackState {
    #[default]
    Stopped,
    Playing,
    Paused,
}

/// What the OS media controls show (Now Playing, MPRIS, SMTC).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct NowPlaying {
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub duration: Option<Duration>,
    pub position: Option<Duration>,
    /// An `https:` or `file:` URL.
    pub artwork_url: Option<String>,
    pub state: PlaybackState,
    pub can_next: bool,
    pub can_previous: bool,
    pub can_seek: bool,
}

/// A press on a media key or in the OS media controls.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MediaCommand {
    Play,
    Pause,
    Toggle,
    Stop,
    Next,
    Previous,
    /// Relative, signed in milliseconds.
    SeekBy(i64),
    SetPosition(Duration),
}

/// A playing sound, from [`crate::Audio::play`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PlayerId(pub u64);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AudioSource {
    Path(PathBuf),
    Url(String),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlayOptions {
    pub source: AudioSource,
    /// 0 to 1.
    pub volume: f32,
    /// Emit [`AudioEvent::Spectrum`] frames for this player.
    pub spectrum: bool,
}

impl PlayOptions {
    pub fn path(path: impl Into<PathBuf>) -> Self {
        Self { source: AudioSource::Path(path.into()), volume: 1., spectrum: false }
    }

    pub fn url(url: impl Into<String>) -> Self {
        Self { source: AudioSource::Url(url.into()), volume: 1., spectrum: false }
    }

    pub fn volume(mut self, volume: f32) -> Self {
        self.volume = volume.clamp(0., 1.);
        self
    }

    pub fn spectrum(mut self) -> Self {
        self.spectrum = true;
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AudioState {
    Playing,
    Paused,
    Ended,
    Stopped,
    Error,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum AudioEvent {
    /// A transition, not a position tick.
    State { state: AudioState, position: Duration, duration: Option<Duration>, error: Option<String> },
    /// [`crate::spectrum::BINS`] log-spaced bins in 0..=1, about 15 per second.
    Spectrum(Vec<f32>),
}

/// Something the OS told the app.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum SystemEvent {
    /// The app became the active (frontmost) app.
    Activated,
    Deactivated,
    /// URLs the OS asked the app to open (a registered scheme).
    OpenUrls(Vec<String>),
    /// Documents the OS asked the app to open (a registered type, the Dock, the command line).
    OpenFiles(Vec<PathBuf>),
    /// Files dropped onto one of the app's windows, at `position` in window points.
    FileDrop {
        window: WindowId,
        paths: Vec<PathBuf>,
        position: Option<(f32, f32)>,
    },
    /// A notification, or one of its actions, was clicked. `data` is what [`Notification::data`]
    /// held when this process showed it.
    NotificationClicked {
        id: NotificationId,
        action: Option<String>,
        data: Option<serde_json::Value>,
    },
    /// A tray item's icon was clicked (`item: None`) or one of its menu items chosen.
    Tray {
        tray: TrayId,
        item: Option<String>,
    },
    Media(MediaCommand),
    Audio {
        player: PlayerId,
        event: AudioEvent,
    },
}
