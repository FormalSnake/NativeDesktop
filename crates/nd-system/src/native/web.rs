//! The web: clipboard text through the async Clipboard API (a read may need a user gesture
//! and the browser may ask first) and URLs opened in a new tab. Everything else answers
//! `Unsupported`; per-window file pickers are `nd-os-web`'s.

use wasm_bindgen_futures::{JsFuture, spawn_local};

use crate::backend::{Backend, Reply};
use crate::{Error, Feature, Result};

pub struct WebBackend;

// SAFETY: wasm32 without atomics has one thread; nothing here is shared across threads.
unsafe impl Send for WebBackend {}
unsafe impl Sync for WebBackend {}

fn clipboard() -> Option<web_sys::Clipboard> {
    Some(web_sys::window()?.navigator().clipboard())
}

fn js_error(e: wasm_bindgen::JsValue) -> Error {
    Error::Denied(e.as_string().unwrap_or_else(|| format!("{e:?}")))
}

impl Backend for WebBackend {
    fn clipboard_read_text(&self, reply: Reply<Option<String>>) {
        let Some(clipboard) = clipboard() else { return reply.unsupported(Feature::Clipboard) };
        spawn_local(async move {
            let text = JsFuture::from(clipboard.read_text()).await;
            reply.send(text.map(|t| t.as_string().filter(|t| !t.is_empty())).map_err(js_error));
        });
    }

    fn clipboard_write_text(&self, text: String, reply: Reply<()>) {
        let Some(clipboard) = clipboard() else { return reply.unsupported(Feature::Clipboard) };
        spawn_local(async move {
            reply.send(JsFuture::from(clipboard.write_text(&text)).await.map(drop).map_err(js_error));
        });
    }

    fn open_url(&self, url: &str) -> Result<()> {
        let window = web_sys::window().ok_or_else(|| Error::unsupported(Feature::Shell))?;
        window.open_with_url_and_target(url, "_blank").map(drop).map_err(js_error)
    }
}
