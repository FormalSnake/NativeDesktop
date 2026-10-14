//! App-level panels and alerts. Panels open modeless (`beginWithCompletionHandler`), so the
//! app keeps running behind them; an alert with no window to attach to is app modal, as on
//! every Mac app.

use std::cell::Cell;
use std::path::PathBuf;

use block2::RcBlock;
use objc2::msg_send;
use objc2::rc::Retained;
use objc2_app_kit::{NSAlert, NSAlertStyle, NSModalResponse, NSModalResponseOK, NSOpenPanel, NSSavePanel};
use objc2_foundation::{MainThreadMarker, NSArray, NSString, NSURL};

use super::file_url;
use crate::backend::Reply;
use crate::{FileFilter, MessageLevel, MessageOptions, OpenOptions, PickKind, SaveOptions};

fn path_of(url: &NSURL) -> Option<PathBuf> {
    url.path().map(|p| PathBuf::from(p.to_string()))
}

fn configure(panel: &NSSavePanel, title: Option<&str>, directory: Option<&std::path::Path>, filters: &[FileFilter]) {
    // Modern panels show no title bar; the message line is where a title is read.
    if let Some(title) = title {
        panel.setMessage(Some(&NSString::from_str(title)));
    }
    if let Some(dir) = directory {
        panel.setDirectoryURL(Some(&file_url(dir)));
    }
    let extensions: Vec<Retained<NSString>> =
        filters.iter().flat_map(|f| &f.extensions).map(|e| NSString::from_str(e)).collect();
    if !extensions.is_empty() {
        let array = NSArray::from_retained_slice(&extensions);
        // `allowedContentTypes` takes UTTypes, which an extension list does not name.
        let _: () = unsafe { msg_send![panel, setAllowedFileTypes: &*array] };
    }
}

/// Wraps a one-shot reply in the `Fn` a completion block must be.
fn once<T: 'static>(reply: Reply<T>, f: impl Fn(NSModalResponse) -> T + 'static) -> RcBlock<dyn Fn(NSModalResponse)> {
    let reply = Cell::new(Some(reply));
    RcBlock::new(move |code: NSModalResponse| {
        if let Some(reply) = reply.take() {
            reply.ok(f(code));
        }
    })
}

pub fn open(mtm: MainThreadMarker, options: OpenOptions, reply: Reply<Option<Vec<PathBuf>>>) {
    let panel = NSOpenPanel::openPanel(mtm);
    let folders = options.pick == PickKind::Folders;
    panel.setCanChooseFiles(!folders);
    panel.setCanChooseDirectories(folders);
    panel.setCanCreateDirectories(folders);
    panel.setAllowsMultipleSelection(options.multiple);
    configure(&panel, options.title.as_deref(), options.directory.as_deref(), &options.filters);
    let read = panel.clone();
    let block = once(reply, move |code| {
        (code == NSModalResponseOK).then(|| read.URLs().iter().filter_map(|u| path_of(&u)).collect::<Vec<_>>())
    });
    panel.beginWithCompletionHandler(&block);
}

pub fn save(mtm: MainThreadMarker, options: SaveOptions, reply: Reply<Option<PathBuf>>) {
    let panel = NSSavePanel::savePanel(mtm);
    configure(&panel, options.title.as_deref(), options.directory.as_deref(), &options.filters);
    if let Some(name) = &options.file_name {
        panel.setNameFieldStringValue(&NSString::from_str(name));
    }
    let read = panel.clone();
    let block =
        once(reply, move |code| (code == NSModalResponseOK).then(|| read.URL().and_then(|u| path_of(&u))).flatten());
    panel.beginWithCompletionHandler(&block);
}

pub fn message(_mtm: MainThreadMarker, options: MessageOptions, reply: Reply<usize>) {
    // `runModal` is a nested run loop: start it on a later turn, never inside whatever asked.
    super::later_on_run_loop(move || {
        let mtm = MainThreadMarker::new().expect("main queue");
        let alert = NSAlert::new(mtm);
        alert.setMessageText(&NSString::from_str(&options.title));
        if let Some(detail) = &options.detail {
            alert.setInformativeText(&NSString::from_str(detail));
        }
        alert.setAlertStyle(match options.level {
            MessageLevel::Info => NSAlertStyle::Informational,
            MessageLevel::Warning => NSAlertStyle::Warning,
            MessageLevel::Error => NSAlertStyle::Critical,
        });
        for label in &options.buttons {
            alert.addButtonWithTitle(&NSString::from_str(label));
        }
        let last = options.buttons.len().saturating_sub(1);
        // NSAlertFirstButtonReturn is 1000; later buttons count up from it.
        let code = alert.runModal();
        reply.ok(usize::try_from(code - 1000).ok().filter(|i| *i <= last).unwrap_or(last));
    });
}
