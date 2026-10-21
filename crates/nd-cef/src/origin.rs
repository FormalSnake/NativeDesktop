//! An extension's requests leave with the origin Chrome gives them. CEF's URL loader proxy adds
//! the `Origin` of a CORS request the way a navigation's is built (`proxy_url_loader_factory.cc`,
//! `InterceptedRequest::Restart`), and that sanitizer keeps only http(s) initiators, so an
//! extension's fetch leaves with `Origin: null`. The network service sets the extension's own
//! origin wherever Chrome sends one, so the header CEF added is dropped and it decides. 1Password's
//! servers answer `Origin: null` with 500 and 503, and its extension then cannot add the account
//! its desktop app hands it.
//!
//! Only a request some handler is asked about can be changed, and the default profile's pages run
//! in the global context, which takes no handler. A context sharing its storage carries one, and
//! CEF asks it about every request of a worker with no frame (an extension's service worker).

use std::cell::RefCell;

use cef::*;

thread_local! {
    static CONTEXT: RefCell<Option<RequestContext>> = const { RefCell::new(None) };
}

/// Runs once, after Chromium has initialized.
pub(crate) fn install() {
    let Some(mut global) = request_context_get_global_context() else { return };
    let mut handler = OriginContext::new();
    let context = request_context_cef_create_context_shared(Some(&mut global), Some(&mut handler));
    CONTEXT.with(|c| *c.borrow_mut() = context);
}

fn needs_fix(initiator: &str, origin: &str) -> bool {
    initiator.starts_with("chrome-extension://") && origin == "null"
}

wrap_request_context_handler! {
    struct OriginContext;

    impl RequestContextHandler {
        fn resource_request_handler(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            request: Option<&mut Request>,
            _is_navigation: ::std::os::raw::c_int,
            _is_download: ::std::os::raw::c_int,
            request_initiator: Option<&CefString>,
            _disable_default_handling: Option<&mut ::std::os::raw::c_int>,
        ) -> Option<ResourceRequestHandler> {
            let initiator = request_initiator.map(|s| s.to_string()).unwrap_or_default();
            let origin = crate::text(&request?.header_by_name(Some(&CefString::from("Origin"))));
            needs_fix(&initiator, &origin).then(OriginFix::new)
        }
    }
}

wrap_resource_request_handler! {
    struct OriginFix;

    impl ResourceRequestHandler {
        fn on_before_resource_load(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            request: Option<&mut Request>,
            _callback: Option<&mut Callback>,
        ) -> ReturnValue {
            if let Some(request) = request {
                drop_header(request, "Origin");
            }
            ReturnValue::CONTINUE
        }
    }
}

/// `CefRequest` has no remove, and an empty value would still be sent.
fn drop_header(request: &mut Request, name: &str) {
    let mut all = CefStringMultimap::new();
    request.header_map(Some(&mut all));
    let mut kept = CefStringMultimap::new();
    for (key, values) in all {
        if key.eq_ignore_ascii_case(name) {
            continue;
        }
        for value in values {
            kept.append(&key, &value);
        }
    }
    request.set_header_map(Some(&mut kept));
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_an_extension_s_null_origin_is_dropped() {
        assert!(super::needs_fix("chrome-extension://abc", "null"));
        assert!(!super::needs_fix("chrome-extension://abc", "chrome-extension://abc"));
        assert!(!super::needs_fix("https://example.com", "null"));
        assert!(!super::needs_fix("chrome-extension://abc", ""));
    }
}
