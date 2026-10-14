//! macOS: AppKit panels and alerts, `NSWorkspace`, `NSDocumentController`, UserNotifications
//! (legacy `NSUserNotificationCenter` for an unbundled binary), `NSStatusItem`, Now Playing
//! and remote commands, `AVPlayer` with an audio tap. AppKit objects live on the main thread:
//! calls from another thread are posted to the main queue, calls on it run in place.

mod audio;
mod dialogs;
mod events;
mod media;
mod notify;
mod tray;

use std::path::PathBuf;
use std::sync::OnceLock;

use objc2_app_kit::{NSDocumentController, NSWorkspace};
use objc2_foundation::{MainThreadMarker, NSArray, NSString, NSURL};

use super::desktop::{self, ClipboardWorker};
use crate::backend::{AudioControl, Backend, Emitter, Reply};
use crate::{
    AppIdentity, ClipboardImage, Error, MessageOptions, Notification, NotificationId, NowPlaying, OpenOptions,
    PlayOptions, PlayerId, Result, SaveOptions, Tray, TrayId,
};

/// Runs `f` on the main thread: now when already there, else on a later main queue turn.
pub(crate) fn on_main(f: impl FnOnce(MainThreadMarker) + Send + 'static) {
    match MainThreadMarker::new() {
        Some(mtm) => f(mtm),
        None => dispatch2::DispatchQueue::main().exec_async(move || f(MainThreadMarker::new().expect("main queue"))),
    }
}

/// Runs `f` on a later turn of the main run loop, in common modes. Unlike a block on the main
/// dispatch queue it does not occupy that serial queue, so a nested run loop started inside it
/// (`runModal`) still drains the main queue.
pub(crate) fn later_on_run_loop(f: impl FnOnce() + 'static) {
    let f = std::cell::Cell::new(Some(f));
    let block = block2::RcBlock::new(move || {
        if let Some(f) = f.take() {
            f();
        }
    });
    // SAFETY: a framework constant and a block the run loop copies.
    unsafe {
        let modes = NSArray::from_slice(&[objc2_foundation::NSRunLoopCommonModes]);
        objc2_foundation::NSRunLoop::mainRunLoop().performInModes_block(&modes, &block);
    }
}

/// Where events go once [`Backend::attach`] ran; main-thread observers read it.
pub(crate) fn emitter() -> Emitter {
    EMITTER.get().cloned().unwrap_or_else(Emitter::detached)
}

static EMITTER: OnceLock<Emitter> = OnceLock::new();

pub struct MacBackend {
    clipboard: ClipboardWorker,
}

impl MacBackend {
    pub fn new(_app: &AppIdentity) -> Self {
        Self { clipboard: ClipboardWorker::new() }
    }
}

pub(crate) fn file_url(path: &std::path::Path) -> objc2::rc::Retained<NSURL> {
    NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()))
}

impl Backend for MacBackend {
    fn attach(&self, events: Emitter) {
        let _ = EMITTER.set(events);
        on_main(events::install);
    }

    fn open_dialog(&self, options: OpenOptions, reply: Reply<Option<Vec<PathBuf>>>) {
        on_main(move |mtm| dialogs::open(mtm, options, reply));
    }

    fn save_dialog(&self, options: SaveOptions, reply: Reply<Option<PathBuf>>) {
        on_main(move |mtm| dialogs::save(mtm, options, reply));
    }

    fn message(&self, options: MessageOptions, reply: Reply<usize>) {
        on_main(move |mtm| dialogs::message(mtm, options, reply));
    }

    fn clipboard_read_text(&self, reply: Reply<Option<String>>) {
        self.clipboard.read_text(reply)
    }

    fn clipboard_write_text(&self, text: String, reply: Reply<()>) {
        self.clipboard.write_text(text, reply)
    }

    fn clipboard_read_image(&self, reply: Reply<Option<ClipboardImage>>) {
        self.clipboard.read_image(reply)
    }

    fn clipboard_write_image(&self, image: ClipboardImage, reply: Reply<()>) {
        self.clipboard.write_image(image, reply)
    }

    fn clipboard_read_files(&self, reply: Reply<Option<Vec<PathBuf>>>) {
        self.clipboard.read_files(reply)
    }

    fn clipboard_write_files(&self, files: Vec<PathBuf>, reply: Reply<()>) {
        self.clipboard.write_files(files, reply)
    }

    fn notify(&self, id: NotificationId, notification: Notification, reply: Reply<()>) {
        on_main(move |mtm| notify::show(mtm, id, notification, reply));
    }

    fn recent_add(&self, path: PathBuf, reply: Reply<()>) {
        on_main(move |mtm| {
            NSDocumentController::sharedDocumentController(mtm).noteNewRecentDocumentURL(&file_url(&path));
            reply.ok(());
        });
    }

    fn recent_clear(&self, reply: Reply<()>) {
        on_main(move |mtm| {
            // SAFETY: the sender is unused by NSDocumentController's implementation.
            unsafe { NSDocumentController::sharedDocumentController(mtm).clearRecentDocuments(None) };
            reply.ok(());
        });
    }

    fn credential_set(&self, service: String, account: String, secret: String, reply: Reply<()>) {
        desktop::credential_set(service, account, secret, reply)
    }

    fn credential_get(&self, service: String, account: String, reply: Reply<Option<String>>) {
        desktop::credential_get(service, account, reply)
    }

    fn credential_delete(&self, service: String, account: String, reply: Reply<()>) {
        desktop::credential_delete(service, account, reply)
    }

    fn open_url(&self, url: &str) -> Result<()> {
        let ns = NSURL::URLWithString(&NSString::from_str(url))
            .ok_or_else(|| Error::Invalid(format!("not a URL: {url}")))?;
        opened(NSWorkspace::sharedWorkspace().openURL(&ns), url)
    }

    fn open_path(&self, path: PathBuf) -> Result<()> {
        opened(NSWorkspace::sharedWorkspace().openURL(&file_url(&path)), &path.display().to_string())
    }

    fn reveal_path(&self, path: PathBuf) -> Result<()> {
        let urls = NSArray::from_retained_slice(&[file_url(&path)]);
        NSWorkspace::sharedWorkspace().activateFileViewerSelectingURLs(&urls);
        Ok(())
    }

    fn tray_set(&self, tray: Tray, reply: Reply<()>) {
        on_main(move |mtm| reply.send(tray::set(mtm, tray)));
    }

    fn tray_remove(&self, id: TrayId, reply: Reply<()>) {
        on_main(move |mtm| reply.send(tray::remove(mtm, &id)));
    }

    fn media_set(&self, now: Option<NowPlaying>, reply: Reply<()>) {
        on_main(move |mtm| reply.send(media::set(mtm, now)));
    }

    fn audio_play(&self, id: PlayerId, options: PlayOptions, reply: Reply<()>) {
        on_main(move |mtm| reply.send(audio::play(mtm, id, options)));
    }

    fn audio_control(&self, id: PlayerId, control: AudioControl, reply: Reply<()>) {
        on_main(move |mtm| reply.send(audio::control(mtm, id, control)));
    }
}

fn opened(ok: bool, what: &str) -> Result<()> {
    if ok { Ok(()) } else { Err(Error::Platform(format!("no app opened {what}"))) }
}

/// Reads native state back for tier 2 tests. Main thread only.
pub mod probe {
    use objc2_foundation::MainThreadMarker;

    use crate::TrayId;

    fn mtm() -> MainThreadMarker {
        MainThreadMarker::new().expect("probes run on the main thread")
    }

    /// The status item's button title and its top-level menu titles.
    pub fn tray(id: &TrayId) -> Option<(String, Vec<String>)> {
        super::tray::probe(mtm(), id)
    }

    /// Chooses the top-level menu item at `index` as a click would, or clicks the status item's
    /// button when `index` is `None`.
    pub fn tray_perform(id: &TrayId, index: Option<usize>) -> bool {
        super::tray::perform(mtm(), id, index)
    }

    /// Now Playing's title and `MPNowPlayingPlaybackState` (1 playing, 2 paused, 3 stopped).
    pub fn now_playing() -> (Option<String>, isize) {
        super::media::probe(mtm())
    }
}
