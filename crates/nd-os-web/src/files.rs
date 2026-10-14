//! Files on the web. Opening shows the browser's picker through a hidden `<input type=file>`
//! and reads each chosen file into memory under a path of its own (`/picked/<n>/<name>`), so
//! the app gets paths as on desktop and reads them back with [`read`]. Saving answers with a
//! path under `/downloads/`; [`write`] to such a path downloads the bytes under that name.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use nd_backend_gpui::GpuiBackend;
use nd_components::services::{OpenCallback, SaveCallback};
use nd_components::{OpenDialog, SaveDialog};
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use wasm_bindgen_futures::JsFuture;

const DOWNLOADS: &str = "/downloads";

thread_local! {
    static PICKED: RefCell<HashMap<PathBuf, Vec<u8>>> = RefCell::new(HashMap::new());
    static NEXT: Cell<u64> = const { Cell::new(1) };
}

fn js_error(what: &str, error: wasm_bindgen::JsValue) -> io::Error {
    io::Error::other(format!("{what}: {error:?}"))
}

/// The bytes of a file the user picked, or of one [`write`] kept.
pub fn read(path: &Path) -> io::Result<Vec<u8>> {
    PICKED
        .with(|p| p.borrow().get(path).cloned())
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, path.display().to_string()))
}

/// Downloads `bytes` when `path` came from a save dialog, else keeps them in memory for
/// [`read`] until the page closes.
pub fn write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if path.starts_with(DOWNLOADS) {
        let name = path.file_name().map_or_else(|| "download".into(), |n| n.to_string_lossy().into_owned());
        return download(&name, bytes);
    }
    PICKED.with(|p| p.borrow_mut().insert(path.to_owned(), bytes.to_vec()));
    Ok(())
}

fn download(name: &str, bytes: &[u8]) -> io::Result<()> {
    let document = web_sys::window()
        .and_then(|w| w.document())
        .ok_or_else(|| io::Error::new(io::ErrorKind::Unsupported, "no document"))?;
    let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(bytes));
    let options = web_sys::BlobPropertyBag::new();
    options.set_type("application/octet-stream");
    let blob =
        web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &options).map_err(|e| js_error("Blob", e))?;
    let url = web_sys::Url::create_object_url_with_blob(&blob).map_err(|e| js_error("object URL", e))?;
    let anchor: web_sys::HtmlAnchorElement =
        document.create_element("a").map_err(|e| js_error("anchor", e))?.unchecked_into();
    anchor.set_href(&url);
    anchor.set_download(name);
    anchor.click();
    let _ = web_sys::Url::revoke_object_url(&url);
    Ok(())
}

/// Shows the browser's file picker. Folders cannot be picked on the web; a folder dialog
/// answers `None`.
pub fn open(backend: &GpuiBackend, dialog: &OpenDialog, done: OpenCallback) {
    let backend = backend.clone();
    if !dialog.files {
        backend.defer(move |app| done(None, app));
        return;
    }
    let Some(document) = web_sys::window().and_then(|w| w.document()) else {
        backend.defer(move |app| done(None, app));
        return;
    };
    let Ok(input) = document.create_element("input").map(|e| e.unchecked_into::<web_sys::HtmlInputElement>()) else {
        backend.defer(move |app| done(None, app));
        return;
    };
    input.set_type("file");
    input.set_multiple(dialog.multiple);
    if !dialog.extensions.is_empty() {
        let accept: Vec<String> = dialog.extensions.iter().map(|e| format!(".{}", e.trim_start_matches('.'))).collect();
        input.set_accept(&accept.join(","));
    }
    // Exactly one of `change` and `cancel` fires; whichever does takes the callback.
    let done = Rc::new(RefCell::new(Some(done)));
    let on_change = {
        let (done, backend, input) = (done.clone(), backend.clone(), input.clone());
        Closure::once_into_js(move || {
            let Some(done) = done.borrow_mut().take() else { return };
            wasm_bindgen_futures::spawn_local(async move {
                let paths = read_picked(&input).await;
                backend.defer(move |app| done(paths, app));
            });
        })
    };
    let on_cancel = {
        let (done, backend) = (done.clone(), backend.clone());
        Closure::once_into_js(move || {
            if let Some(done) = done.borrow_mut().take() {
                backend.defer(move |app| done(None, app));
            }
        })
    };
    let _ = input.add_event_listener_with_callback("change", on_change.unchecked_ref());
    let _ = input.add_event_listener_with_callback("cancel", on_cancel.unchecked_ref());
    input.click();
}

async fn read_picked(input: &web_sys::HtmlInputElement) -> Option<Vec<PathBuf>> {
    let files = input.files()?;
    let batch = NEXT.with(|n| n.replace(n.get() + 1));
    let mut paths = Vec::new();
    for index in 0..files.length() {
        let Some(file) = files.get(index) else { continue };
        let Ok(buffer) = JsFuture::from(file.array_buffer()).await else { continue };
        let bytes = js_sys::Uint8Array::new(&buffer).to_vec();
        let path = PathBuf::from(format!("/picked/{batch}/{}", file.name()));
        PICKED.with(|p| p.borrow_mut().insert(path.clone(), bytes));
        paths.push(path);
    }
    (!paths.is_empty()).then_some(paths)
}

/// The web has no save panel: the answer is the suggested name under `/downloads/`, and the
/// browser asks where to put it when the app writes it.
pub fn save(backend: &GpuiBackend, dialog: &SaveDialog, done: SaveCallback) {
    let name = dialog.file_name.as_ref().map_or("download", |n| n.as_ref()).to_owned();
    let path = Path::new(DOWNLOADS).join(name);
    backend.defer(move |app| done(Some(path), app));
}
