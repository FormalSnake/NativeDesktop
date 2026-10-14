//! Requests for schemes registered with `nd_webview::register_scheme`. Chromium asks on its IO
//! thread; the request crosses to the main thread as a `SchemeRequest`, and the app's answer
//! comes back through shared state the IO thread reads.

use std::os::raw::c_int;
use std::sync::{Arc, Mutex};

use cef::*;
use dispatch2::DispatchQueue;
use nd_webview::{Once, SchemeRequest, SchemeResponder, SchemeResponse};

use super::page;

#[derive(Default)]
struct Pending {
    answer: Option<Result<SchemeResponse, String>>,
    callback: Option<Callback>,
    offset: usize,
    cancelled: bool,
}

/// Shared between the IO thread and the main thread.
#[derive(Clone, Default)]
struct Shared(Arc<Mutex<Pending>>);

// SAFETY: CEF callbacks are thread-safe reference-counted objects, and every access goes
// through the mutex.
unsafe impl Send for Shared {}
unsafe impl Sync for Shared {}

impl Shared {
    fn lock(&self) -> std::sync::MutexGuard<'_, Pending> {
        self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

pub(crate) fn register_global() {
    for scheme in nd_webview::schemes() {
        let mut factory = NdSchemes::new();
        register_scheme_handler_factory(Some(&CefString::from(scheme.as_str())), None, Some(&mut factory));
    }
}

pub(crate) fn register_on(context: &RequestContext) {
    for scheme in nd_webview::schemes() {
        let mut factory = NdSchemes::new();
        context.register_scheme_handler_factory(Some(&CefString::from(scheme.as_str())), None, Some(&mut factory));
    }
}

/// On the main thread: hands the request to the page that made it.
fn deliver(browser: c_int, url: String, scheme: String, method: String, shared: Shared) {
    let answer = {
        let shared = shared.clone();
        move |answer: Result<SchemeResponse, String>| {
            let callback = {
                let mut p = shared.lock();
                if p.cancelled {
                    return;
                }
                p.answer = Some(answer);
                p.callback.take()
            };
            if let Some(callback) = callback {
                callback.cont();
            }
        }
    };
    match page::by_browser(browser) {
        Some(page) => {
            let responder = SchemeResponder(Once::new(Err("no handler answered the request".into()), answer));
            page.page.emit(SchemeRequest { url, scheme, method, responder });
        }
        None => answer(Err("no page made the request".into())),
    }
}

wrap_scheme_handler_factory! {
    struct NdSchemes;

    impl SchemeHandlerFactory {
        fn create(
            &self,
            browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            scheme_name: Option<&CefString>,
            request: Option<&mut Request>,
        ) -> Option<ResourceHandler> {
            let browser = browser.map(|b| b.identifier())?;
            let request = request?;
            let url = super::text(&request.url());
            let method = super::text(&request.method());
            let scheme = scheme_name.map(|s| s.to_string()).unwrap_or_default();
            let shared = Shared::default();
            let sent = shared.clone();
            DispatchQueue::main().exec_async(move || deliver(browser, url, scheme, method, sent));
            Some(NdResource::new(shared))
        }
    }
}

wrap_resource_handler! {
    struct NdResource {
        shared: Shared,
    }

    impl ResourceHandler {
        fn open(&self, _request: Option<&mut Request>, handle_request: Option<&mut c_int>, callback: Option<&mut Callback>) -> c_int {
            let mut p = self.shared.lock();
            let ready = p.answer.is_some();
            if !ready {
                p.callback = callback.map(|c| c.clone());
            }
            if let Some(handle) = handle_request {
                *handle = c_int::from(ready);
            }
            1
        }

        fn response_headers(&self, response: Option<&mut Response>, response_length: Option<&mut i64>, _redirect_url: Option<&mut CefString>) {
            let p = self.shared.lock();
            let Some(response) = response else { return };
            match &p.answer {
                Some(Ok(answer)) => {
                    response.set_status(c_int::from(answer.status));
                    response.set_mime_type(Some(&CefString::from(answer.mime.as_str())));
                    for (name, value) in &answer.headers {
                        response.set_header_by_name(Some(&CefString::from(name.as_str())), Some(&CefString::from(value.as_str())), 1);
                    }
                    if let Some(len) = response_length {
                        *len = answer.body.len() as i64;
                    }
                }
                Some(Err(_)) | None => {
                    response.set_error(Errorcode::FAILED);
                    if let Some(len) = response_length {
                        *len = 0;
                    }
                }
            }
        }

        fn read(&self, data_out: *mut u8, bytes_to_read: c_int, bytes_read: Option<&mut c_int>, _callback: Option<&mut ResourceReadCallback>) -> c_int {
            let mut p = self.shared.lock();
            let offset = p.offset;
            let Some(Ok(answer)) = &p.answer else { return 0 };
            let rest = answer.body.len().saturating_sub(offset);
            let n = rest.min(usize::try_from(bytes_to_read).unwrap_or(0));
            if n == 0 || data_out.is_null() {
                return 0;
            }
            // SAFETY: CEF hands a buffer of `bytes_to_read` bytes.
            unsafe { std::ptr::copy_nonoverlapping(answer.body[offset..].as_ptr(), data_out, n) };
            p.offset += n;
            if let Some(out) = bytes_read {
                *out = n as c_int;
            }
            1
        }

        fn cancel(&self) {
            self.shared.lock().cancelled = true;
        }
    }
}
