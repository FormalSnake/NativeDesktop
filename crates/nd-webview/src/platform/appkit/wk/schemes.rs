//! Requests for schemes registered with [`crate::register_scheme`], answered by the app.

use std::cell::RefCell;
use std::collections::HashSet;

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{AnyThread, Message};
use objc2_foundation::{NSData, NSDictionary, NSError, NSHTTPURLResponse, NSString, NSURL};
use objc2_web_kit::WKURLSchemeTask;

use super::WkPage;
use super::convert::{ns, url_string};
use crate::events::SchemeRequest;
use crate::{Once, SchemeResponder, SchemeResponse};

thread_local! {
    /// Tasks WebKit stopped. Answering one raises an exception, so answers check here first.
    static STOPPED: RefCell<HashSet<usize>> = RefCell::default();
}

fn key(task: &ProtocolObject<dyn WKURLSchemeTask>) -> usize {
    std::ptr::from_ref(task) as usize
}

pub(super) fn start(owner: &WkPage, task: &ProtocolObject<dyn WKURLSchemeTask>) {
    let request = unsafe { task.request() };
    let url = request.URL();
    let scheme = url.as_ref().and_then(|u| u.scheme()).map(|s| s.to_string()).unwrap_or_default();
    let method = request.HTTPMethod().map(|m| m.to_string()).unwrap_or_else(|| "GET".into());
    let url = url_string(url.as_deref());
    let task: Retained<ProtocolObject<dyn WKURLSchemeTask>> = task.retain();
    STOPPED.with(|s| s.borrow_mut().remove(&key(&task)));
    let request_url = url.clone();
    let responder = SchemeResponder(Once::new(Err("no handler answered the request".into()), move |answer| {
        if STOPPED.with(|s| s.borrow_mut().remove(&key(&task))) {
            return;
        }
        match answer {
            Ok(response) => respond(&task, &request_url, response),
            Err(error) => refuse(&task, &error),
        }
    }));
    owner.page.emit(SchemeRequest { url, scheme, method, responder });
}

pub(super) fn stop(task: &ProtocolObject<dyn WKURLSchemeTask>) {
    STOPPED.with(|s| s.borrow_mut().insert(key(task)));
}

pub(super) fn refuse(task: &ProtocolObject<dyn WKURLSchemeTask>, message: &str) {
    let info = NSDictionary::from_slices(
        &[unsafe { objc2_foundation::NSLocalizedDescriptionKey }],
        &[&*ns(message) as &objc2::runtime::AnyObject],
    );
    let error = unsafe { NSError::errorWithDomain_code_userInfo(&ns("NDWebView"), 1, Some(&info)) };
    unsafe { task.didFailWithError(&error) };
}

fn respond(task: &ProtocolObject<dyn WKURLSchemeTask>, url: &str, response: SchemeResponse) {
    // App headers go last so they win over the defaults (a charset on the type, a ranged
    // length).
    let mut fields: Vec<(String, String)> = vec![
        ("Content-Type".into(), response.mime.clone()),
        ("Content-Length".into(), response.body.len().to_string()),
    ];
    for (name, value) in response.headers {
        fields.retain(|(n, _)| !n.eq_ignore_ascii_case(&name));
        fields.push((name, value));
    }
    let keys: Vec<Retained<NSString>> = fields.iter().map(|(k, _)| ns(k)).collect();
    let values: Vec<Retained<NSString>> = fields.iter().map(|(_, v)| ns(v)).collect();
    let keys: Vec<&NSString> = keys.iter().map(|k| &**k).collect();
    let values: Vec<&NSString> = values.iter().map(|v| &**v).collect();
    let headers = NSDictionary::from_slices(&keys, &values);
    let target = NSURL::URLWithString(&ns(url))
        .unwrap_or_else(|| NSURL::URLWithString(&ns("about:blank")).expect("about:blank"));
    let http = NSHTTPURLResponse::initWithURL_statusCode_HTTPVersion_headerFields(
        NSHTTPURLResponse::alloc(),
        &target,
        response.status as isize,
        Some(&ns("HTTP/1.1")),
        Some(&headers),
    );
    let Some(http) = http else {
        refuse(task, "could not build the response");
        return;
    };
    unsafe {
        task.didReceiveResponse(&http);
        task.didReceiveData(&NSData::with_bytes(&response.body));
        task.didFinish();
    }
}
