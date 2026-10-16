//! The page's JavaScript dialogs as nd's drawn alerts on its window, and its file choosers
//! through GPUI's path prompts (the desktop portal).

use std::path::PathBuf;

use cef::FileDialogMode;
use nd_components::{Alert, AlertResponse};
use nd_gpui::gpui::PathPromptOptions;

use super::view::OsrView;

/// Whether the page is in a window an alert can attach to.
pub(crate) fn available(view: &OsrView) -> bool {
    view.window().is_some()
}

/// An alert, confirm or prompt; `done` gets the field's text (its initial value: drawn alerts
/// have no text field yet), or `None` when dismissed.
pub(crate) fn message(
    view: &OsrView,
    title: &str,
    message: &str,
    buttons: &[&str],
    field: Option<String>,
    done: impl FnOnce(Option<String>) + 'static,
) {
    let (Some((window, backend)), Some(accept)) = (view.window(), buttons.first()) else { return done(None) };
    let mut alert = Alert::new(title.to_owned()).message(message.to_owned()).button(accept.to_string());
    if let Some(cancel) = buttons.get(1) {
        alert = alert.cancel(cancel.to_string());
    }
    backend.defer(move |app| {
        nd_components::services::alert(app, window, alert, move |response: AlertResponse, _| {
            done((response.0 == 0).then(|| field.unwrap_or_default()));
        });
    });
}

/// An open or save prompt; `done` gets the chosen paths, or `None` when cancelled.
pub(crate) fn files(
    view: &OsrView,
    mode: FileDialogMode,
    default: String,
    _extensions: Vec<String>,
    done: impl FnOnce(Option<Vec<PathBuf>>) + 'static,
) {
    let Some(app) = view.window().and_then(|(_, backend)| backend.async_app()) else { return done(None) };
    let save = mode == FileDialogMode::SAVE;
    let options = PathPromptOptions {
        files: mode != FileDialogMode::OPEN_FOLDER,
        directories: mode == FileDialogMode::OPEN_FOLDER,
        multiple: mode == FileDialogMode::OPEN_MULTIPLE,
        prompt: None,
    };
    app.spawn(async move |cx| {
        let chosen = if save {
            let default = PathBuf::from(default);
            let directory = default
                .parent()
                .filter(|d| !d.as_os_str().is_empty())
                .map_or_else(nd_webview::Page::downloads_dir, |d| d.to_owned());
            let name = default.file_name().map(|n| n.to_string_lossy().into_owned());
            let answer = cx.update(|cx| cx.prompt_for_new_path(&directory, name.as_deref()));
            answer.await.ok().and_then(Result::ok).flatten().map(|path| vec![path])
        } else {
            let answer = cx.update(|cx| cx.prompt_for_paths(options));
            answer.await.ok().and_then(Result::ok).flatten()
        };
        done(chosen);
    })
    .detach();
}
