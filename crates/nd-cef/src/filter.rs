//! The app's [`nd_webview::RequestFilter`] on Chromium's IO thread. Every page's request handler
//! asks it about each request in `resource_request_handler`, and hands back a handler only for a
//! request the answer changes: cancelled, redirected, answered with the app's body, or a
//! document whose response the filter wants to see. Nothing here touches the main thread.

use std::os::raw::c_int;
use std::sync::{Arc, Mutex, PoisonError};

use cef::*;
use nd_webview::filter::{CspInsert, FilterRequest, FilterResponse, ResourceKind, Verdict, filters, request_filter};

/// The resource request handler for `request`, or `None` to let Chromium load it untouched.
pub(crate) fn handler(
    browser: Option<&mut Browser>,
    request: Option<&mut Request>,
    initiator: Option<&CefString>,
) -> Option<ResourceRequestHandler> {
    let filter = request_filter()?;
    let request = request?;
    let url = crate::text(&request.url());
    let initiator = initiator.map(|i| i.to_string()).unwrap_or_default();
    if !filters(&url, &initiator) {
        return None;
    }
    let mut first_party = crate::text(&request.first_party_for_cookies());
    if first_party.is_empty() {
        first_party = browser.and_then(|b| b.main_frame()).map(|f| crate::text(&f.url())).unwrap_or_default();
    }
    let kind = kind(request.resource_type());
    let method = crate::text(&request.method());
    let info = FilterRequest { url: &url, first_party: &first_party, initiator: &initiator, kind, method: &method };
    let verdict = filter.request(&info);
    if verdict == Verdict::Allow && !kind.is_document() {
        return None;
    }
    let owned = Owned { url, first_party, initiator, kind, method };
    Some(NdFiltered::new(Decided(Arc::new(Mutex::new(Decision { request: owned, verdict, csp: None })))))
}

fn kind(t: ResourceType) -> ResourceKind {
    match t {
        ResourceType::MAIN_FRAME | ResourceType::NAVIGATION_PRELOAD_MAIN_FRAME => ResourceKind::Document,
        ResourceType::SUB_FRAME | ResourceType::NAVIGATION_PRELOAD_SUB_FRAME => ResourceKind::Subdocument,
        ResourceType::STYLESHEET => ResourceKind::Stylesheet,
        ResourceType::SCRIPT => ResourceKind::Script,
        ResourceType::IMAGE => ResourceKind::Image,
        ResourceType::FONT_RESOURCE => ResourceKind::Font,
        ResourceType::OBJECT | ResourceType::PLUGIN_RESOURCE => ResourceKind::Object,
        ResourceType::MEDIA => ResourceKind::Media,
        ResourceType::XHR => ResourceKind::Xhr,
        ResourceType::PING => ResourceKind::Ping,
        ResourceType::CSP_REPORT => ResourceKind::CspReport,
        ResourceType::WORKER | ResourceType::SHARED_WORKER | ResourceType::SERVICE_WORKER => ResourceKind::Worker,
        ResourceType::PREFETCH => ResourceKind::Prefetch,
        ResourceType::FAVICON => ResourceKind::Favicon,
        _ => ResourceKind::Other,
    }
}

struct Owned {
    url: String,
    first_party: String,
    initiator: String,
    kind: ResourceKind,
    method: String,
}

impl Owned {
    fn borrow(&self) -> FilterRequest<'_> {
        FilterRequest {
            url: &self.url,
            first_party: &self.first_party,
            initiator: &self.initiator,
            kind: self.kind,
            method: &self.method,
        }
    }
}

struct Decision {
    request: Owned,
    verdict: Verdict,
    /// The policy the filter added to this document's response.
    csp: Option<String>,
}

/// One request's decision, shared by the handler's callbacks on the IO thread.
#[derive(Clone)]
struct Decided(Arc<Mutex<Decision>>);

impl Decided {
    fn lock(&self) -> std::sync::MutexGuard<'_, Decision> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

wrap_resource_request_handler! {
    struct NdFiltered {
        decided: Decided,
    }

    impl ResourceRequestHandler {
        fn on_before_resource_load(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            request: Option<&mut Request>,
            _callback: Option<&mut Callback>,
        ) -> ReturnValue {
            match &self.decided.lock().verdict {
                Verdict::Block => ReturnValue::CANCEL,
                Verdict::Redirect(url) => {
                    if let Some(request) = request {
                        request.set_url(Some(&CefString::from(url.as_str())));
                    }
                    ReturnValue::CONTINUE
                }
                Verdict::Allow | Verdict::Respond { .. } => ReturnValue::CONTINUE,
            }
        }

        fn resource_handler(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            _request: Option<&mut Request>,
        ) -> Option<ResourceHandler> {
            match &self.decided.lock().verdict {
                Verdict::Respond { mime, body } => Some(NdBody::new(Body(Arc::new(Mutex::new(Served {
                    mime: mime.clone(),
                    body: body.clone(),
                    offset: 0,
                }))))),
                _ => None,
            }
        }

        fn on_resource_response(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            _request: Option<&mut Request>,
            response: Option<&mut Response>,
        ) -> c_int {
            let Some(response) = response else { return 0 };
            let mut decided = self.decided.lock();
            if decided.verdict != Verdict::Allow || !decided.request.kind.is_document() {
                return 0;
            }
            let mime = crate::text(&response.mime_type());
            if !mime.eq_ignore_ascii_case("text/html") {
                return 0;
            }
            let Some(filter) = request_filter() else { return 0 };
            let mut map = CefStringMultimap::new();
            response.header_map(Some(&mut map));
            let headers: Vec<(String, String)> =
                map.into_iter().flat_map(|(name, values)| values.into_iter().map(move |v| (name.clone(), v))).collect();
            let status = u16::try_from(response.status()).unwrap_or(0);
            let seen = FilterResponse { status, mime: &mime, headers: &headers };
            decided.csp = filter.response(&decided.request.borrow(), &seen).filter(|p| !p.trim().is_empty());
            0
        }

        fn resource_response_filter(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            _request: Option<&mut Request>,
            _response: Option<&mut Response>,
        ) -> Option<ResponseFilter> {
            let policy = self.decided.lock().csp.clone()?;
            Some(NdCsp::new(Csp(Arc::new(Mutex::new((CspInsert::new(&policy), Vec::new()))))))
        }
    }
}

struct Served {
    mime: String,
    body: Arc<[u8]>,
    offset: usize,
}

#[derive(Clone)]
struct Body(Arc<Mutex<Served>>);

wrap_resource_handler! {
    struct NdBody {
        body: Body,
    }

    impl ResourceHandler {
        fn open(&self, _request: Option<&mut Request>, handle_request: Option<&mut c_int>, _callback: Option<&mut Callback>) -> c_int {
            if let Some(handle) = handle_request {
                *handle = 1;
            }
            1
        }

        fn response_headers(&self, response: Option<&mut Response>, response_length: Option<&mut i64>, _redirect_url: Option<&mut CefString>) {
            let served = self.body.0.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(response) = response {
                response.set_status(200);
                response.set_mime_type(Some(&CefString::from(served.mime.as_str())));
                // A redirected script or XHR is usually cross-origin to the page.
                response.set_header_by_name(Some(&CefString::from("Access-Control-Allow-Origin")), Some(&CefString::from("*")), 1);
            }
            if let Some(len) = response_length {
                *len = i64::try_from(served.body.len()).unwrap_or(-1);
            }
        }

        fn read(&self, data_out: *mut u8, bytes_to_read: c_int, bytes_read: Option<&mut c_int>, _callback: Option<&mut ResourceReadCallback>) -> c_int {
            let mut served = self.body.0.lock().unwrap_or_else(PoisonError::into_inner);
            let rest = served.body.len().saturating_sub(served.offset);
            let n = rest.min(usize::try_from(bytes_to_read).unwrap_or(0));
            if n == 0 || data_out.is_null() {
                return 0;
            }
            // SAFETY: CEF hands a buffer of `bytes_to_read` bytes.
            unsafe { std::ptr::copy_nonoverlapping(served.body[served.offset..].as_ptr(), data_out, n) };
            served.offset += n;
            if let Some(out) = bytes_read {
                *out = n as c_int;
            }
            1
        }

        fn cancel(&self) {}
    }
}

/// The inserter and the output it has not handed to Chromium yet.
#[derive(Clone)]
struct Csp(Arc<Mutex<(CspInsert, Vec<u8>)>>);

wrap_response_filter! {
    struct NdCsp {
        csp: Csp,
    }

    impl ResponseFilter {
        fn init_filter(&self) -> c_int {
            1
        }

        fn filter(
            &self,
            data_in: Option<&mut Vec<u8>>,
            data_in_read: Option<&mut usize>,
            data_out: Option<&mut Vec<u8>>,
            data_out_written: Option<&mut usize>,
        ) -> ResponseFilterStatus {
            let mut guard = self.csp.0.lock().unwrap_or_else(PoisonError::into_inner);
            let (insert, pending) = &mut *guard;
            match data_in {
                Some(chunk) => {
                    insert.feed(chunk, pending);
                    if let Some(read) = data_in_read {
                        *read = chunk.len();
                    }
                }
                None => insert.finish(pending),
            }
            let written = match data_out {
                Some(out) => {
                    let n = pending.len().min(out.len());
                    out[..n].copy_from_slice(&pending[..n]);
                    pending.drain(..n);
                    n
                }
                None => 0,
            };
            if let Some(w) = data_out_written {
                *w = written;
            }
            // Held bytes still owe Chromium output: it calls again with no input at the end.
            if pending.is_empty() && !insert.holding() {
                ResponseFilterStatus::DONE
            } else {
                ResponseFilterStatus::NEED_MORE_DATA
            }
        }
    }
}
