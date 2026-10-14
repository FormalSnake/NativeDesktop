//! One object per page answering WebKit: navigation, UI, script messages, cookie changes,
//! scheme tasks, downloads and key-value observation. It holds the page weakly; the page holds
//! it (WebKit's delegate properties are weak).

use std::cell::Cell;
use std::ffi::c_void;
use std::rc::Weak;

use block2::{DynBlock, RcBlock};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool, NSObjectProtocol, ProtocolObject};
use objc2::{DefinedClass, MainThreadOnly, Message, define_class, msg_send};
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSEventModifierFlags, NSModalResponse, NSOpenPanel, NSTextField,
};
use objc2_foundation::{
    MainThreadMarker, NSArray, NSDictionary, NSError, NSHTTPURLResponse, NSObject, NSPoint, NSRect, NSSize, NSString,
    NSURL, NSURLErrorCancelled, NSURLErrorClientCertificateRejected, NSURLErrorClientCertificateRequired,
    NSURLErrorDomain, NSURLErrorSecureConnectionFailed, NSURLErrorServerCertificateHasBadDate,
    NSURLErrorServerCertificateHasUnknownRoot, NSURLErrorServerCertificateNotYetValid,
    NSURLErrorServerCertificateUntrusted, NSURLResponse,
};
use objc2_web_kit::{
    WKDownload, WKDownloadDelegate, WKFrameInfo, WKHTTPCookieStore, WKHTTPCookieStoreObserver, WKMediaCaptureType,
    WKNavigation, WKNavigationAction, WKNavigationActionPolicy, WKNavigationDelegate, WKNavigationResponse,
    WKNavigationResponsePolicy, WKNavigationType, WKOpenPanelParameters, WKPermissionDecision, WKScriptMessage,
    WKScriptMessageHandler, WKSecurityOrigin, WKUIDelegate, WKURLSchemeHandler, WKURLSchemeTask,
    WKUserContentController, WKWebView, WKWebViewConfiguration, WKWindowFeatures,
};

use super::convert::{json, url_string};
use super::{WkPage, downloads, schemes};
use crate::events::*;
use crate::script::{INTERNAL_HANDLER, INTERNAL_WORLD};
use crate::{Once, PermissionDecision, PermissionResponder};

pub struct DelegateIvars {
    owner: Weak<WkPage>,
}

impl Delegate {
    pub(super) fn new(mtm: MainThreadMarker, owner: Weak<WkPage>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(DelegateIvars { owner });
        unsafe { msg_send![super(this), init] }
    }

    fn owner(&self) -> Option<std::rc::Rc<WkPage>> {
        self.ivars().owner.upgrade().filter(|o| !o.st.borrow().closed)
    }

    /// See `webView:createWebViewWithConfiguration:...` below.
    fn popup(
        &self,
        config: &WKWebViewConfiguration,
        action: &WKNavigationAction,
        features: &WKWindowFeatures,
    ) -> Option<Retained<WKWebView>> {
        let owner = self.owner()?;
        let url = url_string(unsafe { action.request() }.URL().as_deref());
        let disposition = disposition(action, Some(features));
        let features = window_features(features);
        if !owner.page.options().adopt_popups {
            owner.page.emit(NewWindow { url, disposition, user_gesture: true, features, popup: None });
            return None;
        }
        let mtm = self.mtm();
        let config = config.retain();
        let mut created: Option<Retained<WKWebView>> = None;
        let popup = owner.page.popup(|page| {
            let view = WkPage::create(mtm, page, Some(config));
            created = view.ns_view().downcast::<WKWebView>().ok();
            view
        });
        owner.page.emit(NewWindow { url, disposition, user_gesture: true, features, popup: Some(popup) });
        created
    }
}

/// Cancellations and the tail of a navigation turned into a download are not failures.
fn is_noise(error: &NSError) -> bool {
    let domain = error.domain().to_string();
    (domain == unsafe { NSURLErrorDomain }.to_string() && error.code() == NSURLErrorCancelled)
        || (domain == "WebKitErrorDomain" && error.code() == 102)
}

const TLS_ERRORS: [isize; 7] = [
    NSURLErrorSecureConnectionFailed,
    NSURLErrorServerCertificateHasBadDate,
    NSURLErrorServerCertificateUntrusted,
    NSURLErrorServerCertificateHasUnknownRoot,
    NSURLErrorServerCertificateNotYetValid,
    NSURLErrorClientCertificateRejected,
    NSURLErrorClientCertificateRequired,
];

fn load_failed(owner: &WkPage, error: &NSError) {
    owner.st.borrow_mut().provisional_url.clear();
    if is_noise(error) {
        return;
    }
    let failing =
        super::convert::failing_url(error).unwrap_or_else(|| url_string(unsafe { owner.web.URL() }.as_deref()));
    let message = error.localizedDescription().to_string();
    owner.page.emit(LoadFailed { url: failing.clone(), error: message.clone() });
    let tls =
        error.domain().to_string() == unsafe { NSURLErrorDomain }.to_string() && TLS_ERRORS.contains(&error.code());
    if tls {
        owner.st.borrow_mut().last_secure = Some((false, false));
        owner.page.emit(SecurityChanged { secure: false, insecure_content: false, url: failing, error: Some(message) });
    }
}

fn disposition(action: &WKNavigationAction, features: Option<&WKWindowFeatures>) -> Disposition {
    let flags = unsafe { action.modifierFlags() };
    let middle = unsafe { action.buttonNumber() } == 2;
    if flags.contains(NSEventModifierFlags::Shift) {
        Disposition::Window
    } else if flags.contains(NSEventModifierFlags::Command) || middle {
        Disposition::BackgroundTab
    } else if features.is_some_and(|f| unsafe { f.width().is_some() || f.height().is_some() }) {
        Disposition::Popup
    } else {
        Disposition::ForegroundTab
    }
}

fn window_features(f: &WKWindowFeatures) -> Option<WindowFeatures> {
    let n = |v: Option<Retained<objc2_foundation::NSNumber>>| v.map(|n| n.as_f64());
    let out = unsafe { WindowFeatures { x: n(f.x()), y: n(f.y()), width: n(f.width()), height: n(f.height()) } };
    (out != WindowFeatures::default()).then_some(out)
}

/// An alert, confirm or prompt as a sheet on the view's window. `None` is dismissed.
fn script_panel(
    owner: &WkPage,
    title: &str,
    message: &NSString,
    buttons: &[&str],
    field: Option<&str>,
    done: impl FnOnce(Option<String>) + 'static,
) {
    let mtm = owner.web.mtm();
    let Some(window) = owner.web.window() else {
        done(None);
        return;
    };
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str(title));
    alert.setInformativeText(message);
    for b in buttons {
        alert.addButtonWithTitle(&NSString::from_str(b));
    }
    let input = field.map(|initial| {
        let input = NSTextField::initWithFrame(
            NSTextField::alloc(mtm),
            NSRect::new(NSPoint::new(0., 0.), NSSize::new(260., 24.)),
        );
        input.setStringValue(&NSString::from_str(initial));
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

define_class!(
    // SAFETY: NSObject has no subclassing requirements. No Drop impl.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = DelegateIvars]
    pub struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    impl Delegate {
        #[unsafe(method(observeValueForKeyPath:ofObject:change:context:))]
        fn observe(
            &self,
            key: Option<&NSString>,
            _object: Option<&AnyObject>,
            _change: Option<&NSDictionary<NSString, AnyObject>>,
            context: *mut c_void,
        ) {
            let Some(owner) = self.owner() else { return };
            if !context.is_null() {
                downloads::progress(&owner, context as u64);
                return;
            }
            if key.is_some_and(|k| k.to_string() == "hasOnlySecureContent") {
                owner.report_security();
            } else {
                owner.sync_state();
            }
        }
    }

    unsafe impl WKNavigationDelegate for Delegate {
        #[unsafe(method(webView:decidePolicyForNavigationAction:decisionHandler:))]
        fn decide_action(
            &self,
            _web: &WKWebView,
            action: &WKNavigationAction,
            decide: &DynBlock<dyn Fn(WKNavigationActionPolicy)>,
        ) {
            let Some(owner) = self.owner() else {
                decide.call((WKNavigationActionPolicy::Allow,));
                return;
            };
            let request_url = unsafe { action.request() }.URL();
            let scheme = request_url.as_ref().and_then(|u| u.scheme()).map(|s| s.to_string().to_ascii_lowercase());
            if let Some(scheme) = scheme
                && !matches!(scheme.as_str(), "http" | "https" | "about" | "data" | "blob" | "file" | "javascript")
                && !crate::schemes().contains(&scheme)
                && !unsafe { WKWebView::handlesURLScheme(&NSString::from_str(&scheme), self.mtm()) }
            {
                owner.page.emit(ExternalProtocol { url: url_string(request_url.as_deref()) });
                decide.call((WKNavigationActionPolicy::Cancel,));
                return;
            }
            if unsafe { action.shouldPerformDownload() } {
                decide.call((WKNavigationActionPolicy::Download,));
                return;
            }
            // Cmd, Shift or middle click on a link is a new tab or window, as in a browser.
            let link = unsafe { action.navigationType() } == WKNavigationType::LinkActivated;
            let kind = disposition(action, None);
            if link && kind != Disposition::ForegroundTab {
                let url = url_string(unsafe { action.request() }.URL().as_deref());
                owner.page.emit(NewWindow { url, disposition: kind, user_gesture: true, features: None, popup: None });
                decide.call((WKNavigationActionPolicy::Cancel,));
                return;
            }
            decide.call((WKNavigationActionPolicy::Allow,));
        }

        #[unsafe(method(webView:decidePolicyForNavigationResponse:decisionHandler:))]
        fn decide_response(
            &self,
            _web: &WKWebView,
            response: &WKNavigationResponse,
            decide: &DynBlock<dyn Fn(WKNavigationResponsePolicy)>,
        ) {
            let attachment = unsafe { response.response() }
                .downcast::<NSHTTPURLResponse>()
                .ok()
                .and_then(|r| {
                    r.allHeaderFields().to_vecs().0.iter().zip(r.allHeaderFields().to_vecs().1.iter()).find_map(|(k, v)| {
                        let k = k.downcast_ref::<NSString>()?.to_string();
                        let v = v.downcast_ref::<NSString>()?.to_string();
                        k.eq_ignore_ascii_case("content-disposition").then_some(v)
                    })
                })
                .is_some_and(|v| v.trim_start().to_ascii_lowercase().starts_with("attachment"));
            let main = unsafe { response.isForMainFrame() };
            if main && (attachment || !unsafe { response.canShowMIMEType() }) {
                decide.call((WKNavigationResponsePolicy::Download,));
            } else {
                decide.call((WKNavigationResponsePolicy::Allow,));
            }
        }

        #[unsafe(method(webView:didStartProvisionalNavigation:))]
        fn did_start(&self, web: &WKWebView, _nav: Option<&WKNavigation>) {
            let Some(owner) = self.owner() else { return };
            let stop = {
                let mut st = owner.st.borrow_mut();
                st.provisional_url = url_string(unsafe { web.URL() }.as_deref());
                st.awaiting_start = false;
                std::mem::take(&mut st.pending_stop)
            };
            if stop {
                unsafe { web.stopLoading() };
            }
            owner.sync_state();
        }

        /// WebKit commits `about:blank` in place of a navigation it refuses (blocked ports, some
        /// malformed addresses) without a failure callback; that is reported as a failure.
        #[unsafe(method(webView:didCommitNavigation:))]
        fn did_commit(&self, web: &WKWebView, _nav: Option<&WKNavigation>) {
            let Some(owner) = self.owner() else { return };
            let committed = url_string(unsafe { web.URL() }.as_deref());
            let requested = {
                let mut st = owner.st.borrow_mut();
                st.committed_url = committed.clone();
                std::mem::take(&mut st.provisional_url)
            };
            owner.report_security();
            owner.sync_state();
            if committed == "about:blank" && !requested.is_empty() && requested != "about:blank" {
                owner.page.emit(LoadFailed { url: requested, error: "The address could not be loaded.".into() });
            }
        }

        #[unsafe(method(webView:didFinishNavigation:))]
        fn did_finish(&self, _web: &WKWebView, _nav: Option<&WKNavigation>) {
            let Some(owner) = self.owner() else { return };
            owner.sync_state();
            owner.report_security();
            owner.report_favicon();
        }

        #[unsafe(method(webView:didFailNavigation:withError:))]
        fn did_fail(&self, _web: &WKWebView, _nav: Option<&WKNavigation>, error: &NSError) {
            if let Some(owner) = self.owner() {
                load_failed(&owner, error);
                owner.sync_state();
            }
        }

        #[unsafe(method(webView:didFailProvisionalNavigation:withError:))]
        fn did_fail_provisional(&self, _web: &WKWebView, _nav: Option<&WKNavigation>, error: &NSError) {
            if let Some(owner) = self.owner() {
                load_failed(&owner, error);
                owner.sync_state();
            }
        }

        #[unsafe(method(webViewWebContentProcessDidTerminate:))]
        fn process_gone(&self, _web: &WKWebView) {
            if let Some(owner) = self.owner() {
                owner.page.emit(RenderProcessGone { reason: "the web content process ended".into() });
                owner.sync_state();
            }
        }

        #[unsafe(method(webView:navigationAction:didBecomeDownload:))]
        fn action_download(&self, _web: &WKWebView, _action: &WKNavigationAction, download: &WKDownload) {
            if let Some(owner) = self.owner() {
                downloads::adopt(&owner, download);
            }
        }

        #[unsafe(method(webView:navigationResponse:didBecomeDownload:))]
        fn response_download(&self, _web: &WKWebView, _response: &WKNavigationResponse, download: &WKDownload) {
            if let Some(owner) = self.owner() {
                downloads::adopt(&owner, download);
            }
        }
    }

    unsafe impl WKUIDelegate for Delegate {
        /// `window.open` and `target=_blank`. Without adopted popups nothing opens and the app
        /// hears the URL; with them the opener gets a live window the app mounts.
        #[unsafe(method_id(webView:createWebViewWithConfiguration:forNavigationAction:windowFeatures:))]
        fn create_web_view(
            &self,
            _web: &WKWebView,
            config: &WKWebViewConfiguration,
            action: &WKNavigationAction,
            features: &WKWindowFeatures,
        ) -> Option<Retained<WKWebView>> {
            self.popup(config, action, features)
        }

        #[unsafe(method(webViewDidClose:))]
        fn did_close(&self, _web: &WKWebView) {
            if let Some(owner) = self.owner() {
                owner.page.emit(WindowClosed);
            }
        }

        #[unsafe(method(webView:runJavaScriptAlertPanelWithMessage:initiatedByFrame:completionHandler:))]
        fn alert(&self, _web: &WKWebView, message: &NSString, _frame: &WKFrameInfo, done: &DynBlock<dyn Fn()>) {
            let done = done.copy();
            match self.owner() {
                Some(owner) => script_panel(&owner, "The page says", message, &["OK"], None, move |_| done.call(())),
                None => done.call(()),
            }
        }

        #[unsafe(method(webView:runJavaScriptConfirmPanelWithMessage:initiatedByFrame:completionHandler:))]
        fn confirm(&self, _web: &WKWebView, message: &NSString, _frame: &WKFrameInfo, done: &DynBlock<dyn Fn(Bool)>) {
            let done = done.copy();
            match self.owner() {
                Some(owner) => script_panel(&owner, "Confirm", message, &["OK", "Cancel"], None, move |answer| {
                    done.call((Bool::new(answer.is_some()),))
                }),
                None => done.call((Bool::NO,)),
            }
        }

        #[unsafe(method(webView:runJavaScriptTextInputPanelWithPrompt:defaultText:initiatedByFrame:completionHandler:))]
        fn prompt(
            &self,
            _web: &WKWebView,
            prompt: &NSString,
            default_text: Option<&NSString>,
            _frame: &WKFrameInfo,
            done: &DynBlock<dyn Fn(*mut NSString)>,
        ) {
            let done = done.copy();
            let initial = default_text.map(|t| t.to_string()).unwrap_or_default();
            match self.owner() {
                Some(owner) => script_panel(
                    &owner,
                    "The page is asking for input",
                    prompt,
                    &["OK", "Cancel"],
                    Some(&initial),
                    move |answer| match answer {
                        Some(text) => {
                            let s = NSString::from_str(&text);
                            done.call((Retained::as_ptr(&s).cast_mut(),));
                        }
                        None => done.call((std::ptr::null_mut(),)),
                    },
                ),
                None => done.call((std::ptr::null_mut(),)),
            }
        }

        #[unsafe(method(webView:runOpenPanelWithParameters:initiatedByFrame:completionHandler:))]
        fn open_panel(
            &self,
            web: &WKWebView,
            parameters: &WKOpenPanelParameters,
            _frame: &WKFrameInfo,
            done: &DynBlock<dyn Fn(*mut NSArray<NSURL>)>,
        ) {
            let done = done.copy();
            let Some(window) = web.window() else {
                done.call((std::ptr::null_mut(),));
                return;
            };
            let panel = NSOpenPanel::openPanel(self.mtm());
            unsafe {
                panel.setAllowsMultipleSelection(parameters.allowsMultipleSelection());
                panel.setCanChooseDirectories(parameters.allowsDirectories());
            }
            panel.setCanChooseFiles(true);
            let chosen = panel.clone();
            let handler = RcBlock::new(move |response: NSModalResponse| {
                if response == objc2_app_kit::NSModalResponseOK {
                    let urls = chosen.URLs();
                    done.call((Retained::as_ptr(&urls).cast_mut(),));
                } else {
                    done.call((std::ptr::null_mut(),));
                }
            });
            panel.beginSheetModalForWindow_completionHandler(&window, &handler);
        }

        #[unsafe(method(webView:requestMediaCapturePermissionForOrigin:initiatedByFrame:type:decisionHandler:))]
        fn media_permission(
            &self,
            _web: &WKWebView,
            origin: &WKSecurityOrigin,
            _frame: &WKFrameInfo,
            kind: WKMediaCaptureType,
            decide: &DynBlock<dyn Fn(WKPermissionDecision)>,
        ) {
            let decide = decide.copy();
            let Some(owner) = self.owner() else {
                decide.call((WKPermissionDecision::Prompt,));
                return;
            };
            let kinds = match kind {
                WKMediaCaptureType::Camera => vec![PermissionKind::Camera],
                WKMediaCaptureType::Microphone => vec![PermissionKind::Microphone],
                _ => vec![PermissionKind::Camera, PermissionKind::Microphone],
            };
            let origin = unsafe {
                let (scheme, host, port) = (origin.protocol().to_string(), origin.host().to_string(), origin.port());
                let default = matches!((scheme.as_str(), port), ("https", 443) | ("http", 80) | (_, 0));
                if default { format!("{scheme}://{host}") } else { format!("{scheme}://{host}:{port}") }
            };
            // Unanswered, WebKit asks the user itself.
            let responder = PermissionResponder(Once::new(PermissionDecision::Dismiss, move |d| {
                let decision = match d {
                    PermissionDecision::Allow => WKPermissionDecision::Grant,
                    PermissionDecision::Deny => WKPermissionDecision::Deny,
                    PermissionDecision::Dismiss => WKPermissionDecision::Prompt,
                };
                decide.call((decision,));
            }));
            owner.page.emit(PermissionRequest { origin, kinds, responder });
        }
    }

    unsafe impl WKScriptMessageHandler for Delegate {
        #[unsafe(method(userContentController:didReceiveScriptMessage:))]
        fn message(&self, _controller: &WKUserContentController, message: &WKScriptMessage) {
            let Some(owner) = self.owner() else { return };
            let (name, world) = unsafe {
                (message.name().to_string(), message.world().name().map(|n| n.to_string()).unwrap_or_default())
            };
            let body = json(Some(&*unsafe { message.body() }));
            if name == INTERNAL_HANDLER && world == INTERNAL_WORLD {
                owner.internal_message(&body);
            } else {
                owner.page.emit(ScriptMessage { name, world, body });
            }
        }
    }

    unsafe impl WKHTTPCookieStoreObserver for Delegate {
        #[unsafe(method(cookiesDidChangeInCookieStore:))]
        fn cookies_changed(&self, _store: &WKHTTPCookieStore) {
            if let Some(owner) = self.owner() {
                owner.page.emit(CookiesChanged);
            }
        }
    }

    unsafe impl WKURLSchemeHandler for Delegate {
        #[unsafe(method(webView:startURLSchemeTask:))]
        fn start_task(&self, _web: &WKWebView, task: &ProtocolObject<dyn WKURLSchemeTask>) {
            match self.owner() {
                Some(owner) => schemes::start(&owner, task),
                None => schemes::refuse(task, "the page is closed"),
            }
        }

        #[unsafe(method(webView:stopURLSchemeTask:))]
        fn stop_task(&self, _web: &WKWebView, task: &ProtocolObject<dyn WKURLSchemeTask>) {
            schemes::stop(task);
        }
    }

    unsafe impl WKDownloadDelegate for Delegate {
        #[unsafe(method(download:decideDestinationUsingResponse:suggestedFilename:completionHandler:))]
        fn decide_destination(
            &self,
            download: &WKDownload,
            response: &NSURLResponse,
            suggested: &NSString,
            done: &DynBlock<dyn Fn(*mut NSURL)>,
        ) {
            match self.owner() {
                Some(owner) => downloads::decide(&owner, download, response, &suggested.to_string(), done.copy()),
                None => done.call((std::ptr::null_mut(),)),
            }
        }

        #[unsafe(method(downloadDidFinish:))]
        fn finished(&self, download: &WKDownload) {
            if let Some(owner) = self.owner() {
                downloads::finished(&owner, download);
            }
        }

        #[unsafe(method(download:didFailWithError:resumeData:))]
        fn failed(&self, download: &WKDownload, error: &NSError, resume: Option<&objc2_foundation::NSData>) {
            if let Some(owner) = self.owner() {
                downloads::failed(&owner, download, error, resume);
            }
        }
    }
);
