//! The seam each platform implements. A method answers through its [`Reply`], from any thread
//! and at any later time; one it does not override answers [`Error::Unsupported`]. OS events go
//! out through the [`Emitter`] handed to [`Backend::attach`].

use std::path::PathBuf;
use std::sync::Weak;

use futures_channel::oneshot;

use crate::bus::Bus;
use crate::{
    ClipboardImage, Error, Feature, MessageOptions, Notification, NotificationId, NowPlaying, OpenOptions, PlayOptions,
    PlayerId, Result, SaveOptions, SystemEvent, Tray, TrayId,
};

/// Where a backend puts its answer. Dropping it unanswered resolves the call to
/// [`Error::Dropped`].
pub struct Reply<T>(oneshot::Sender<Result<T>>);

impl<T> Reply<T> {
    pub(crate) fn new() -> (Self, oneshot::Receiver<Result<T>>) {
        let (tx, rx) = oneshot::channel();
        (Self(tx), rx)
    }

    pub fn send(self, value: Result<T>) {
        let _ = self.0.send(value);
    }

    pub fn ok(self, value: T) {
        self.send(Ok(value));
    }

    pub fn err(self, error: Error) {
        self.send(Err(error));
    }

    pub fn unsupported(self, feature: Feature) {
        self.err(Error::unsupported(feature));
    }
}

/// Sends OS events to the app's subscribers. Cheap to clone and `Send`; outlives nothing (once
/// the [`crate::System`] is gone, events go nowhere).
#[derive(Clone)]
pub struct Emitter(pub(crate) Weak<Bus>);

impl Emitter {
    pub fn emit(&self, event: SystemEvent) {
        if let Some(bus) = self.0.upgrade() {
            bus.emit(event);
        }
    }

    /// An emitter connected to nothing, for backends used without a `System`.
    pub fn detached() -> Self {
        Self(Weak::new())
    }
}

#[allow(unused_variables)]
pub trait Backend: Send + Sync + 'static {
    /// Called once, before any other method, with where this backend's events go.
    fn attach(&self, events: Emitter) {}

    fn open_dialog(&self, options: OpenOptions, reply: Reply<Option<Vec<PathBuf>>>) {
        reply.unsupported(Feature::FileSystem)
    }
    fn save_dialog(&self, options: SaveOptions, reply: Reply<Option<PathBuf>>) {
        reply.unsupported(Feature::FileSystem)
    }
    fn message(&self, options: MessageOptions, reply: Reply<usize>) {
        reply.unsupported(Feature::Dialogs)
    }

    fn clipboard_read_text(&self, reply: Reply<Option<String>>) {
        reply.unsupported(Feature::Clipboard)
    }
    fn clipboard_write_text(&self, text: String, reply: Reply<()>) {
        reply.unsupported(Feature::Clipboard)
    }
    fn clipboard_read_image(&self, reply: Reply<Option<ClipboardImage>>) {
        reply.unsupported(Feature::ClipboardRich)
    }
    fn clipboard_write_image(&self, image: ClipboardImage, reply: Reply<()>) {
        reply.unsupported(Feature::ClipboardRich)
    }
    fn clipboard_read_files(&self, reply: Reply<Option<Vec<PathBuf>>>) {
        reply.unsupported(Feature::ClipboardRich)
    }
    fn clipboard_write_files(&self, files: Vec<PathBuf>, reply: Reply<()>) {
        reply.unsupported(Feature::ClipboardRich)
    }

    /// `id` is already chosen; the backend shows the notification under it.
    fn notify(&self, id: NotificationId, notification: Notification, reply: Reply<()>) {
        reply.unsupported(Feature::Notifications)
    }

    fn recent_add(&self, path: PathBuf, reply: Reply<()>) {
        reply.unsupported(Feature::RecentDocuments)
    }
    fn recent_clear(&self, reply: Reply<()>) {
        reply.unsupported(Feature::RecentDocuments)
    }

    fn credential_set(&self, service: String, account: String, secret: String, reply: Reply<()>) {
        reply.unsupported(Feature::Credentials)
    }
    fn credential_get(&self, service: String, account: String, reply: Reply<Option<String>>) {
        reply.unsupported(Feature::Credentials)
    }
    fn credential_delete(&self, service: String, account: String, reply: Reply<()>) {
        reply.unsupported(Feature::Credentials)
    }

    fn open_url(&self, url: &str) -> Result<()> {
        Err(Error::unsupported(Feature::Shell))
    }
    fn open_path(&self, path: PathBuf) -> Result<()> {
        Err(Error::unsupported(Feature::Shell))
    }
    fn reveal_path(&self, path: PathBuf) -> Result<()> {
        Err(Error::unsupported(Feature::Shell))
    }

    /// Creates or replaces the item with `tray.id`.
    fn tray_set(&self, tray: Tray, reply: Reply<()>) {
        reply.unsupported(Feature::Tray)
    }
    fn tray_remove(&self, id: TrayId, reply: Reply<()>) {
        reply.unsupported(Feature::Tray)
    }

    /// `None` clears the session (the app stopped playing).
    fn media_set(&self, now: Option<NowPlaying>, reply: Reply<()>) {
        reply.unsupported(Feature::MediaSession)
    }

    fn audio_play(&self, id: PlayerId, options: PlayOptions, reply: Reply<()>) {
        reply.unsupported(Feature::Audio)
    }
    fn audio_control(&self, id: PlayerId, control: AudioControl, reply: Reply<()>) {
        reply.unsupported(Feature::Audio)
    }
}

/// What [`Backend::audio_control`] asks of a player.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AudioControl {
    Pause,
    Resume,
    /// Stops and releases the player; its id names nothing afterwards.
    Stop,
    Seek(std::time::Duration),
    SetVolume(f32),
}
