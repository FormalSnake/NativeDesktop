//! The page's JavaScript dialogs and file choosers, as sheets on its window.

use std::cell::Cell;
use std::path::PathBuf;

use block2::RcBlock;
use cef::FileDialogMode;
use objc2::MainThreadOnly;
use objc2::rc::Retained;
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSModalResponse, NSModalResponseOK, NSOpenPanel, NSSavePanel, NSTextField,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

use super::view::OsrView;

/// Whether the view is in a window a sheet can attach to.
pub(crate) fn available(view: &OsrView) -> bool {
    view.window().is_some()
}

/// An alert, confirm or prompt as a sheet; `done` gets the field's text (empty without one), or
/// `None` when dismissed.
pub(crate) fn message(
    view: &OsrView,
    title: &str,
    message: &str,
    buttons: &[&str],
    field: Option<String>,
    done: impl FnOnce(Option<String>) + 'static,
) {
    let Some(window) = view.window() else { return done(None) };
    let mtm = view.mtm();
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str(title));
    alert.setInformativeText(&NSString::from_str(message));
    for b in buttons {
        alert.addButtonWithTitle(&NSString::from_str(b));
    }
    let input = field.map(|initial| {
        let input = NSTextField::initWithFrame(
            NSTextField::alloc(mtm),
            NSRect::new(NSPoint::new(0., 0.), NSSize::new(260., 24.)),
        );
        input.setStringValue(&NSString::from_str(&initial));
        alert.setAccessoryView(Some(&input));
        input
    });
    let done = Cell::new(Some(done));
    let handler = RcBlock::new(move |response: NSModalResponse| {
        let Some(done) = done.take() else { return };
        if response == NSAlertFirstButtonReturn {
            done(Some(input.as_ref().map(|i| i.stringValue().to_string()).unwrap_or_default()));
        } else {
            done(None);
        }
    });
    alert.beginSheetModalForWindow_completionHandler(&window, Some(&handler));
}

/// An open or save panel as a sheet; `done` gets the chosen paths, or `None` when cancelled.
pub(crate) fn files(
    view: &OsrView,
    mode: FileDialogMode,
    default: String,
    _extensions: Vec<String>,
    done: impl FnOnce(Option<Vec<PathBuf>>) + 'static,
) {
    let Some(window) = view.window() else { return done(None) };
    let mtm = view.mtm();
    let panel: Retained<NSSavePanel> = if mode == FileDialogMode::SAVE {
        let panel = NSSavePanel::savePanel(mtm);
        if let Some(name) = std::path::Path::new(&default).file_name() {
            panel.setNameFieldStringValue(&NSString::from_str(&name.to_string_lossy()));
        }
        panel
    } else {
        let panel = NSOpenPanel::openPanel(mtm);
        panel.setCanChooseFiles(mode != FileDialogMode::OPEN_FOLDER);
        panel.setCanChooseDirectories(mode == FileDialogMode::OPEN_FOLDER);
        panel.setAllowsMultipleSelection(mode == FileDialogMode::OPEN_MULTIPLE);
        Retained::into_super(panel)
    };
    let chosen = panel.clone();
    let done = Cell::new(Some(done));
    let handler = RcBlock::new(move |response: NSModalResponse| {
        let Some(done) = done.take() else { return };
        if response != NSModalResponseOK {
            return done(None);
        }
        let urls: Vec<_> = match chosen.downcast_ref::<NSOpenPanel>() {
            Some(open) => open.URLs().iter().collect(),
            None => chosen.URL().into_iter().collect(),
        };
        done(Some(urls.into_iter().filter_map(|url| url.path()).map(|p| PathBuf::from(p.to_string())).collect()));
    });
    panel.beginSheetModalForWindow_completionHandler(&window, &handler);
}
