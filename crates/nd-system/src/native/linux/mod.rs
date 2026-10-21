//! Linux: the portal file chooser (`rfd`), arboard over Wayland data-control or X11, the
//! Secret Service, freedesktop notifications, `recently-used.xbel`, `xdg-open` and the
//! `FileManager1` interface, StatusNotifierItem (`ksni`), MPRIS (`mpris-server`) and GStreamer.
//! Everything D-Bus runs on zbus's own executor threads, never the app's.

mod audio;
mod mpris;

mod tray;

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::OnceLock;

use super::desktop::{self, ClipboardWorker};
use crate::backend::{AudioControl, Backend, Emitter, Reply};
use crate::{
    AppIdentity, ClipboardImage, Error, MessageOptions, Notification, NotificationId, NowPlaying, OpenOptions,
    PlayOptions, PlayerId, Result, SaveOptions, Tray, TrayId,
};

pub struct LinuxBackend {
    app: AppIdentity,
    clipboard: ClipboardWorker,
    events: OnceLock<Emitter>,
    trays: tray::Trays,
    mpris: mpris::Mpris,
    audio: audio::Audio,
}

impl LinuxBackend {
    pub fn new(app: &AppIdentity) -> Self {
        Self {
            app: app.clone(),
            clipboard: ClipboardWorker::new(),
            events: OnceLock::new(),
            trays: tray::Trays::default(),
            mpris: mpris::Mpris::new(app),
            audio: audio::Audio::new(),
        }
    }

    fn emitter(&self) -> Emitter {
        self.events.get().cloned().unwrap_or_else(Emitter::detached)
    }
}

/// Starts `program` detached; its exit status is not waited for.
fn spawn(program: &str, arg: &std::ffi::OsStr) -> Result<()> {
    let mut child = Command::new(program)
        .arg(arg)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| Error::Platform(format!("{program}: {e}")))?;
    std::thread::spawn(move || child.wait());
    Ok(())
}

/// `org.freedesktop.FileManager1.ShowItems`, which Nautilus, Dolphin, Thunar and Nemo serve.
fn show_items(path: &std::path::Path) -> zbus::Result<()> {
    let bus = zbus::blocking::Connection::session()?;
    bus.call_method(
        Some("org.freedesktop.FileManager1"),
        "/org/freedesktop/FileManager1",
        Some("org.freedesktop.FileManager1"),
        "ShowItems",
        &(vec![crate::xbel::file_uri(path)], ""),
    )?;
    Ok(())
}

impl Backend for LinuxBackend {
    fn attach(&self, events: Emitter) {
        self.audio.set_emitter(events.clone());
        self.mpris.set_emitter(events.clone());
        let _ = self.events.set(events);
    }

    fn open_dialog(&self, options: OpenOptions, reply: Reply<Option<Vec<PathBuf>>>) {
        desktop::dialogs::open(options, reply)
    }

    fn save_dialog(&self, options: SaveOptions, reply: Reply<Option<PathBuf>>) {
        desktop::dialogs::save(options, reply)
    }

    fn message(&self, options: MessageOptions, reply: Reply<usize>) {
        desktop::dialogs::message(options, reply)
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
        super::notify_rust::show(&self.app, self.emitter(), id, notification, reply)
    }

    fn recent_add(&self, path: PathBuf, reply: Reply<()>) {
        let app = self.app.clone();
        std::thread::spawn(move || reply.send(crate::xbel::add(&crate::xbel::default_path(), &app, &path)));
    }

    fn recent_clear(&self, reply: Reply<()>) {
        let app = self.app.clone();
        std::thread::spawn(move || reply.send(crate::xbel::clear(&crate::xbel::default_path(), &app)));
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
        spawn("xdg-open", url.as_ref())
    }

    fn open_path(&self, path: PathBuf) -> Result<()> {
        spawn("xdg-open", path.as_os_str())
    }

    fn reveal_path(&self, path: PathBuf) -> Result<()> {
        // Without a file manager serving FileManager1, the folder opens without the selection.
        std::thread::spawn(move || {
            if show_items(&path).is_err() {
                let dir = if path.is_dir() { path.clone() } else { path.parent().map_or(path.clone(), Into::into) };
                let _ = spawn("xdg-open", dir.as_os_str());
            }
        });
        Ok(())
    }

    fn tray_set(&self, tray: Tray, reply: Reply<()>) {
        reply.send(self.trays.set(tray, &self.app.id, self.emitter()))
    }

    fn tray_remove(&self, id: TrayId, reply: Reply<()>) {
        reply.send(self.trays.remove(&id))
    }

    fn media_set(&self, now: Option<NowPlaying>, reply: Reply<()>) {
        self.mpris.set(now, reply)
    }

    fn audio_play(&self, id: PlayerId, options: PlayOptions, reply: Reply<()>) {
        self.audio.play(id, options, reply)
    }

    fn audio_control(&self, id: PlayerId, control: AudioControl, reply: Reply<()>) {
        self.audio.control(id, control, reply)
    }
}
