//! Services every desktop backend shares: the clipboard through `arboard`, credentials through
//! `keyring` (Keychain, the Secret Service over zbus, Credential Manager), and on Linux and
//! Windows the file and message dialogs through `rfd` (the portal, `IFileDialog`).

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::mpsc;

use crate::backend::Reply;
use crate::{ClipboardImage, Error, Result};

type Job = Box<dyn FnOnce(&mut Option<arboard::Clipboard>) + Send>;

/// One thread owns the clipboard for the life of the process. X11 and Wayland serve what the
/// app copied only while the owner lives, and macOS's pasteboard handle is not `Send`.
pub struct ClipboardWorker(Mutex<mpsc::Sender<Job>>);

impl ClipboardWorker {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("nd-clipboard".into())
            .spawn(move || {
                let mut clipboard = None;
                for job in rx {
                    if clipboard.is_none() {
                        clipboard = arboard::Clipboard::new().map_err(|e| eprintln!("nd-system: clipboard: {e}")).ok();
                    }
                    job(&mut clipboard);
                }
            })
            .expect("clipboard thread");
        Self(Mutex::new(tx))
    }

    fn run<T: Send + 'static>(
        &self,
        reply: Reply<T>,
        f: impl FnOnce(&mut arboard::Clipboard) -> Result<T> + Send + 'static,
    ) {
        let job: Job = Box::new(move |clipboard| match clipboard {
            Some(c) => reply.send(f(c)),
            None => reply.err(Error::Platform("no clipboard (no display connection)".into())),
        });
        // A send fails only when the thread died; the dropped reply answers `Dropped`.
        let _ = self.0.lock().expect("clipboard queue").send(job);
    }

    pub fn read_text(&self, reply: Reply<Option<String>>) {
        self.run(reply, |c| absent(c.get_text()))
    }

    pub fn write_text(&self, text: String, reply: Reply<()>) {
        self.run(reply, move |c| c.set_text(text).map_err(Error::platform))
    }

    pub fn read_image(&self, reply: Reply<Option<ClipboardImage>>) {
        self.run(reply, |c| {
            absent(c.get_image())
                .map(|i| i.map(|i| ClipboardImage { width: i.width, height: i.height, rgba: i.bytes.into_owned() }))
        })
    }

    pub fn write_image(&self, image: ClipboardImage, reply: Reply<()>) {
        self.run(reply, move |c| {
            let data = arboard::ImageData { width: image.width, height: image.height, bytes: image.rgba.into() };
            c.set_image(data).map_err(Error::platform)
        })
    }

    pub fn read_files(&self, reply: Reply<Option<Vec<PathBuf>>>) {
        self.run(reply, |c| absent(c.get().file_list()).map(|f| f.filter(|f| !f.is_empty())))
    }

    pub fn write_files(&self, files: Vec<PathBuf>, reply: Reply<()>) {
        self.run(reply, move |c| c.set().file_list(&files).map_err(Error::platform))
    }
}

/// An empty clipboard, or one holding another kind, is `None`, not an error.
fn absent<T>(result: std::result::Result<T, arboard::Error>) -> Result<Option<T>> {
    match result {
        Ok(v) => Ok(Some(v)),
        Err(arboard::Error::ContentNotAvailable) => Ok(None),
        Err(e) => Err(Error::platform(e)),
    }
}

/// Runs a blocking credential store call off the caller's thread. The Secret Service may show
/// an unlock prompt, and Keychain may ask for access, so these can take as long as the user does.
fn credential<T: Send + 'static>(
    reply: Reply<T>,
    service: String,
    account: String,
    f: impl FnOnce(keyring::Entry) -> keyring::Result<T> + Send + 'static,
) {
    std::thread::spawn(move || {
        let result = keyring::Entry::new(&service, &account).and_then(f);
        reply.send(result.map_err(credential_error));
    });
}

fn credential_error(e: keyring::Error) -> Error {
    match e {
        keyring::Error::NoStorageAccess(e) => Error::Denied(e.to_string()),
        keyring::Error::NoDefaultStore => Error::Platform("no credential store (is a Secret Service running?)".into()),
        e => Error::platform(e),
    }
}

pub fn credential_set(service: String, account: String, secret: String, reply: Reply<()>) {
    credential(reply, service, account, move |e| e.set_password(&secret))
}

pub fn credential_get(service: String, account: String, reply: Reply<Option<String>>) {
    credential(reply, service, account, |e| match e.get_password() {
        Ok(secret) => Ok(Some(secret)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e),
    })
}

pub fn credential_delete(service: String, account: String, reply: Reply<()>) {
    credential(reply, service, account, |e| match e.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e),
    })
}

#[cfg(any(target_os = "linux", windows))]
pub mod dialogs {
    use std::path::PathBuf;

    use rfd::{AsyncFileDialog, AsyncMessageDialog, MessageButtons, MessageDialogResult};

    use crate::backend::Reply;
    use crate::{Error, MessageLevel, MessageOptions, OpenOptions, PickKind, SaveOptions};

    fn on_thread<T: Send + 'static>(reply: Reply<T>, f: impl Future<Output = crate::Result<T>> + Send + 'static) {
        std::thread::spawn(move || reply.send(futures_executor::block_on(f)));
    }

    pub fn open(options: OpenOptions, reply: Reply<Option<Vec<PathBuf>>>) {
        on_thread(reply, async move {
            let mut d = AsyncFileDialog::new();
            if let Some(t) = options.title {
                d = d.set_title(t);
            }
            if let Some(dir) = options.directory {
                d = d.set_directory(dir);
            }
            for f in &options.filters {
                d = d.add_filter(&f.name, &f.extensions);
            }
            let picked = match (options.pick, options.multiple) {
                (PickKind::Files, false) => d.pick_file().await.map(|f| vec![f]),
                (PickKind::Files, true) => d.pick_files().await,
                (PickKind::Folders, false) => d.pick_folder().await.map(|f| vec![f]),
                (PickKind::Folders, true) => d.pick_folders().await,
            };
            Ok(picked
                .map(|files| files.iter().map(|f| f.path().to_owned()).collect())
                .filter(|p: &Vec<_>| !p.is_empty()))
        })
    }

    pub fn save(options: SaveOptions, reply: Reply<Option<PathBuf>>) {
        on_thread(reply, async move {
            let mut d = AsyncFileDialog::new();
            if let Some(t) = options.title {
                d = d.set_title(t);
            }
            if let Some(dir) = options.directory {
                d = d.set_directory(dir);
            }
            if let Some(name) = options.file_name {
                d = d.set_file_name(name);
            }
            for f in &options.filters {
                d = d.add_filter(&f.name, &f.extensions);
            }
            Ok(d.save_file().await.map(|f| f.path().to_owned()))
        })
    }

    pub fn message(options: MessageOptions, reply: Reply<usize>) {
        on_thread(reply, async move {
            let level = match options.level {
                MessageLevel::Info => rfd::MessageLevel::Info,
                MessageLevel::Warning => rfd::MessageLevel::Warning,
                MessageLevel::Error => rfd::MessageLevel::Error,
            };
            let labels = options.buttons.clone();
            let buttons = match labels.as_slice() {
                [] => MessageButtons::Ok,
                [a] => MessageButtons::OkCustom(a.clone()),
                [a, b] => MessageButtons::OkCancelCustom(a.clone(), b.clone()),
                [a, b, c] => MessageButtons::YesNoCancelCustom(a.clone(), b.clone(), c.clone()),
                more => return Err(Error::Invalid(format!("{} alert buttons", more.len()))),
            };
            let mut d = AsyncMessageDialog::new().set_level(level).set_title(&options.title).set_buttons(buttons);
            if let Some(detail) = &options.detail {
                d = d.set_description(detail);
            }
            Ok(button_index(&labels, d.show().await))
        })
    }

    /// The picked button's index; a dismissal without one is the last button.
    fn button_index(labels: &[String], result: MessageDialogResult) -> usize {
        let last = labels.len().saturating_sub(1);
        match result {
            MessageDialogResult::Custom(label) => labels.iter().position(|l| *l == label).unwrap_or(last),
            MessageDialogResult::Ok | MessageDialogResult::Yes => 0,
            MessageDialogResult::No => 1.min(last),
            MessageDialogResult::Cancel => last,
        }
    }
}
