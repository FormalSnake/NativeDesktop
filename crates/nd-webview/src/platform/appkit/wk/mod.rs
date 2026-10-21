//! The WebKit engine: `WKWebView` as an ordinary subview of the node's host view.
//!
//! Navigation state comes from key-value observation of the view's own properties (no polling;
//! an idle page costs nothing). What WebKit has no API for (the hovered link, the context
//! menu's hit test, media playback, element fullscreen) the framework's agent reports from the
//! page, in a private content world (`crate::script::agent_source`).

mod convert;
mod delegate;
mod downloads;
mod menu;
mod schemes;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::time::Instant;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject, Sel};
use objc2::{AnyThread, DefinedClass, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{NSApplication, NSEvent, NSEventType, NSMenu, NSPrintInfo, NSResponder, NSView};
use objc2_foundation::{
    MainThreadMarker, NSData, NSError, NSHTTPCookie, NSKeyValueObservingOptions, NSObject,
    NSObjectNSKeyValueObserverRegistration, NSRect, NSString, NSURLRequest, NSUUID,
};
use objc2_web_kit::{
    WKContentWorld, WKFindConfiguration, WKFindResult, WKUserScript, WKUserScriptInjectionTime, WKWebView,
    WKWebViewConfiguration, WKWebsiteDataStore,
};

use self::convert::{json, ns, url};
use self::delegate::Delegate;
use crate::element::WebViewProps;
use crate::engine::{Command, EngineFactory, EngineView, Page};
use crate::events::*;
use crate::handle::Profile;
use crate::menu::{ContextMenuMode, Hit, MenuItem};
use crate::script::{
    COUNTING_SCRIPT, FAVICON_SCRIPT, INTERNAL_HANDLER, INTERNAL_WORLD, Injection, UserScript, agent_source,
};
use crate::{JsError, JsValue};

pub(crate) struct WkFactory;

impl EngineFactory for WkFactory {
    fn create(&self, page: Page) -> Result<Rc<dyn EngineView>, String> {
        let mtm = MainThreadMarker::new().ok_or("web views are created on the main thread")?;
        Ok(WkPage::create(mtm, page, None))
    }
}

/// Keys of the view the engine observes.
const OBSERVED: [&str; 7] =
    ["URL", "title", "loading", "canGoBack", "canGoForward", "estimatedProgress", "hasOnlySecureContent"];

/// How long a hit the agent reported still describes the menu about to open. The report travels
/// ahead of WebKit's menu proposal on the same connection, so it is milliseconds old; this only
/// guards against one that never arrived.
const HIT_FRESHNESS: f64 = 2.;

pub(crate) struct WkPage {
    pub(crate) page: Page,
    pub(crate) web: Retained<NdWebView>,
    pub(crate) delegate: Retained<Delegate>,
    pub(crate) st: RefCell<State>,
    pub(crate) this: Weak<WkPage>,
}

#[derive(Default)]
pub(crate) struct State {
    scripts: Vec<UserScript>,
    handlers: HashMap<(String, String), Retained<WKContentWorld>>,
    pub menu_items: Vec<MenuItem>,
    pub suppress_menu: bool,
    count_input: bool,
    pub last_hit: Option<(Instant, Hit)>,
    pub last_hover: String,
    pub committed_url: String,
    pub provisional_url: String,
    find_text: String,
    find_options: crate::FindOptions,
    pub last_secure: Option<(bool, bool)>,
    pub menu_actions: Vec<menu::Action>,
    pub downloads: downloads::Downloads,
    pub closed: bool,
    attached: bool,
    /// Loading began and `didStartProvisionalNavigation` has not come yet.
    pub awaiting_start: bool,
    pub pending_stop: bool,
}

impl WkPage {
    /// A page; `config` is WebKit's for a popup it asked the app to create.
    pub(crate) fn create(
        mtm: MainThreadMarker,
        page: Page,
        config: Option<Retained<WKWebViewConfiguration>>,
    ) -> Rc<dyn EngineView> {
        let popup = config.is_some();
        let config = config.unwrap_or_else(|| new_configuration(mtm, &page));
        if popup {
            // WebKit hands a popup a copy of the opener's configuration, user content controller
            // included: sharing it would mix the two pages' scripts and handlers.
            unsafe { config.setUserContentController(&objc2_web_kit::WKUserContentController::new(mtm)) };
        }
        let created = Rc::<WkPage>::new_cyclic(|this| {
            let delegate = Delegate::new(mtm, this.clone());
            if !popup {
                for scheme in crate::schemes() {
                    unsafe {
                        config
                            .setURLSchemeHandler_forURLScheme(Some(ProtocolObject::from_ref(&*delegate)), &ns(&scheme))
                    };
                }
            }
            let controller = unsafe { config.userContentController() };
            let internal = unsafe { WKContentWorld::worldWithName(&ns(INTERNAL_WORLD), mtm) };
            unsafe {
                controller.addScriptMessageHandler_contentWorld_name(
                    ProtocolObject::from_ref(&*delegate),
                    &internal,
                    &ns(INTERNAL_HANDLER),
                )
            };
            let web = NdWebView::new(mtm, &config, this.clone());
            unsafe {
                web.setNavigationDelegate(Some(ProtocolObject::from_ref(&*delegate)));
                web.setUIDelegate(Some(ProtocolObject::from_ref(&*delegate)));
                web.setAllowsBackForwardNavigationGestures(true);
                web.setAllowsMagnification(true);
                if page.options().is_inspectable() {
                    web.setInspectable(true);
                }
                if let Some(ua) = &page.options().user_agent {
                    web.setCustomUserAgent(Some(&ns(ua)));
                }
                let store = config.websiteDataStore().httpCookieStore();
                store.addObserver(ProtocolObject::from_ref(&*delegate));
            }
            let observer: &NSObject = &delegate;
            for key in OBSERVED {
                unsafe {
                    web.addObserver_forKeyPath_options_context(
                        observer,
                        &ns(key),
                        NSKeyValueObservingOptions::New,
                        std::ptr::null_mut(),
                    )
                };
            }
            let st = State { last_secure: None, ..State::default() };
            WkPage { page, web, delegate, st: RefCell::new(st), this: this.clone() }
        });
        created.rebuild_scripts();
        created
    }

    fn mtm(&self) -> MainThreadMarker {
        self.web.mtm()
    }

    /// Replays every user script: WKUserContentController can only remove all of them, so the
    /// registry is the source of truth. The agent always goes first.
    fn rebuild_scripts(&self) {
        let mtm = self.mtm();
        let controller = unsafe { self.web.configuration().userContentController() };
        unsafe { controller.removeAllUserScripts() };
        let st = self.st.borrow();
        let post = format!("window.webkit.messageHandlers.{INTERNAL_HANDLER}.postMessage(m)");
        let agent = agent_source(&post, st.suppress_menu);
        add_script(mtm, &controller, &agent, Injection::Start, false, Some(INTERNAL_WORLD));
        if st.count_input {
            add_script(mtm, &controller, COUNTING_SCRIPT, Injection::Start, true, None);
        }
        for s in &st.scripts {
            add_script(mtm, &controller, &s.guarded_source(), s.injection, !s.all_frames, s.world.as_deref());
        }
    }

    pub(crate) fn evaluate(
        &self,
        code: &str,
        world: Option<&str>,
        done: impl FnOnce(Result<JsValue, JsError>) + 'static,
    ) {
        let mtm = self.mtm();
        let world = content_world(mtm, world);
        let done = Cell::new(Some(done));
        let block = RcBlock::new(move |value: *mut AnyObject, error: *mut NSError| {
            let Some(done) = done.take() else { return };
            if let Some(error) = unsafe { error.as_ref() } {
                done(Err(JsError(js_error_message(error))));
            } else {
                done(Ok(json(unsafe { value.as_ref() })));
            }
        });
        unsafe {
            self.web.evaluateJavaScript_inFrame_inContentWorld_completionHandler(&ns(code), None, &world, Some(&block))
        };
    }

    fn run_internal(&self, code: &str) {
        self.evaluate(code, Some(INTERNAL_WORLD), |_| {});
    }

    fn load(&self, address: &str) {
        let Some(u) = url(address) else {
            self.page.emit(LoadFailed { url: address.to_owned(), error: "not a valid address".into() });
            return;
        };
        unsafe { self.web.loadRequest(&NSURLRequest::requestWithURL(&u)) };
    }

    /// Re-reads every observed property, emitting what changed.
    pub(crate) fn sync_state(&self) {
        let web = &self.web;
        let loading = unsafe { web.isLoading() };
        {
            let mut st = self.st.borrow_mut();
            if loading && !self.page.state().loading {
                st.awaiting_start = true;
            } else if !loading {
                st.awaiting_start = false;
                st.pending_stop = false;
            }
        }
        unsafe {
            self.page.report_url(&convert::url_string(web.URL().as_deref()));
            self.page.report_title(&web.title().map(|t| t.to_string()).unwrap_or_default());
            self.page.report_loading(web.isLoading());
            self.page.report_back(web.canGoBack());
            self.page.report_forward(web.canGoForward());
            self.page.report_progress(web.estimatedProgress());
        }
    }

    pub(crate) fn report_security(&self) {
        let (https, only_secure) = unsafe {
            let https = self.web.URL().and_then(|u| u.scheme()).is_some_and(|s| s.to_string() == "https");
            (https, self.web.hasOnlySecureContent())
        };
        let secure = https && only_secure;
        let insecure_content = https && !only_secure;
        let changed = {
            let mut st = self.st.borrow_mut();
            let changed = st.last_secure != Some((secure, insecure_content));
            st.last_secure = Some((secure, insecure_content));
            changed
        };
        if changed {
            let url = convert::url_string(unsafe { self.web.URL() }.as_deref());
            self.page.emit(SecurityChanged { secure, insecure_content, url, error: None });
        }
    }

    pub(crate) fn report_favicon(&self) {
        let page = self.page.clone();
        self.evaluate(FAVICON_SCRIPT, Some(INTERNAL_WORLD), move |result| {
            let Ok(serde_json::Value::String(raw)) = result else { return };
            let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) else { return };
            let icon_url = v["iconUrl"].as_str().unwrap_or_default().to_owned();
            if !icon_url.is_empty() {
                page.emit(FaviconChanged { page_url: v["pageUrl"].as_str().unwrap_or_default().to_owned(), icon_url });
            }
        });
    }

    /// A message from the framework's agent.
    pub(crate) fn internal_message(&self, body: &serde_json::Value) {
        let text = |k: &str| body[k].as_str().unwrap_or_default().to_owned();
        let some = |k: &str| Some(text(k)).filter(|s| !s.is_empty());
        match body["k"].as_str() {
            Some("hover") => {
                let url = text("url");
                self.st.borrow_mut().last_hover = url.clone();
                self.page.emit(LinkHover { url });
            }
            Some("menu") => {
                let hit = Hit {
                    link: some("link"),
                    image: some("image"),
                    selection: some("selection"),
                    editable: body["editable"].as_bool().unwrap_or(false),
                };
                self.st.borrow_mut().last_hit = Some((Instant::now(), hit.clone()));
                self.page.emit(ContextMenu {
                    x: body["x"].as_f64().unwrap_or(0.),
                    y: body["y"].as_f64().unwrap_or(0.),
                    link: hit.link,
                    image: hit.image,
                    selection: hit.selection,
                    editable: hit.editable,
                });
            }
            Some("audio") => self.page.emit(AudioStateChanged {
                playing: body["playing"].as_bool().unwrap_or(false),
                muted: body["muted"].as_bool().unwrap_or(false),
            }),
            Some("fullscreen") => {
                self.page.emit(FullscreenChanged { fullscreen: body["on"].as_bool().unwrap_or(false) });
            }
            _ => {}
        }
    }

    fn find(&self, text: &str, options: crate::FindOptions) {
        let mtm = self.mtm();
        let config = unsafe { WKFindConfiguration::new(mtm) };
        unsafe {
            config.setCaseSensitive(options.case_sensitive);
            config.setWraps(options.wraps);
            config.setBackwards(options.backwards);
        }
        let page = self.page.clone();
        let block = RcBlock::new(move |result: std::ptr::NonNull<WKFindResult>| {
            let found = unsafe { result.as_ref().matchFound() };
            page.emit(FindResult { match_found: found, match_count: None, active_match: None, done: true });
        });
        unsafe { self.web.findString_withConfiguration_completionHandler(&ns(text), Some(&config), &block) };
    }

    fn cookie_store(&self) -> Retained<objc2_web_kit::WKHTTPCookieStore> {
        unsafe { self.web.configuration().websiteDataStore().httpCookieStore() }
    }

    fn open_inspector(&self) {
        unsafe { self.web.setInspectable(true) };
        // WebKit has no public call that opens the inspector; `_inspector` is the SPI Safari's
        // Develop menu and wry's `open_devtools` use. Without it the page is still inspectable
        // from Safari.
        let web: &AnyObject = &self.web;
        let has_inspector: bool = unsafe { msg_send![web, respondsToSelector: sel!(_inspector)] };
        if !has_inspector {
            tracing::warn!("web view: this WebKit has no _inspector; inspect the page from Safari's Develop menu");
            return;
        }
        let inspector: Option<Retained<AnyObject>> = unsafe { msg_send![web, _inspector] };
        if let Some(inspector) = inspector
            && responds(&inspector, sel!(show))
        {
            let _: () = unsafe { msg_send![&*inspector, show] };
        }
    }

    fn close_inspector(&self) {
        let web: &AnyObject = &self.web;
        let has_inspector: bool = unsafe { msg_send![web, respondsToSelector: sel!(_inspector)] };
        if !has_inspector {
            return;
        }
        let inspector: Option<Retained<AnyObject>> = unsafe { msg_send![web, _inspector] };
        if let Some(inspector) = inspector
            && responds(&inspector, sel!(close))
        {
            let _: () = unsafe { msg_send![&*inspector, close] };
        }
    }
}

fn responds(object: &AnyObject, selector: Sel) -> bool {
    unsafe { msg_send![object, respondsToSelector: selector] }
}

fn content_world(mtm: MainThreadMarker, name: Option<&str>) -> Retained<WKContentWorld> {
    match name.filter(|n| !n.is_empty()) {
        Some(name) => unsafe { WKContentWorld::worldWithName(&ns(name), mtm) },
        None => unsafe { WKContentWorld::pageWorld(mtm) },
    }
}

fn add_script(
    mtm: MainThreadMarker,
    controller: &objc2_web_kit::WKUserContentController,
    source: &str,
    injection: Injection,
    main_frame_only: bool,
    world: Option<&str>,
) {
    let time = match injection {
        Injection::Start => WKUserScriptInjectionTime::AtDocumentStart,
        Injection::End => WKUserScriptInjectionTime::AtDocumentEnd,
    };
    let script = unsafe {
        WKUserScript::initWithSource_injectionTime_forMainFrameOnly_inContentWorld(
            WKUserScript::alloc(mtm),
            &ns(source),
            time,
            main_frame_only,
            &content_world(mtm, world),
        )
    };
    unsafe { controller.addUserScript(&script) };
}

/// The thrown message WebKit puts in `WKJavaScriptExceptionMessage`; its description is only "A
/// JavaScript exception occurred".
fn js_error_message(error: &NSError) -> String {
    let info = error.userInfo();
    info.objectForKey(&ns("WKJavaScriptExceptionMessage"))
        .and_then(|o| o.downcast::<NSString>().ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| error.localizedDescription().to_string())
}

fn new_configuration(mtm: MainThreadMarker, page: &Page) -> Retained<WKWebViewConfiguration> {
    let config = unsafe { WKWebViewConfiguration::new(mtm) };
    unsafe {
        config.setWebsiteDataStore(&data_store(mtm, &page.options().profile));
        let prefs = config.preferences();
        prefs.setElementFullscreenEnabled(true);
        prefs.setJavaScriptCanOpenWindowsAutomatically(false);
    }
    config
}

thread_local! {
    static STORES: RefCell<HashMap<String, Retained<WKWebsiteDataStore>>> = RefCell::default();
    /// Private sessions' stores, by session id, until the session ends.
    static PRIVATE: RefCell<HashMap<u64, Retained<WKWebsiteDataStore>>> = RefCell::default();
}

/// `Default` is WebKit's default store, a private session one non-persistent store, and a named
/// profile a persistent store whose identifier is derived from the name, so the same name finds
/// the same cookies on every launch.
fn data_store(mtm: MainThreadMarker, profile: &Profile) -> Retained<WKWebsiteDataStore> {
    match profile {
        Profile::Default => unsafe { WKWebsiteDataStore::defaultDataStore(mtm) },
        Profile::Private(session) => PRIVATE.with(|p| {
            if let Some(store) = p.borrow().get(&session.id()) {
                return store.clone();
            }
            let store = unsafe { WKWebsiteDataStore::nonPersistentDataStore(mtm) };
            p.borrow_mut().insert(session.id(), store.clone());
            let id = session.id();
            session.on_end(move || {
                PRIVATE.with(|p| p.borrow_mut().remove(&id));
            });
            store
        }),
        Profile::Named(name) => STORES.with(|s| {
            s.borrow_mut()
                .entry(name.clone())
                .or_insert_with(|| {
                    let uuid = NSUUID::initWithUUIDString(NSUUID::alloc(), &ns(&profile_uuid(name)))
                        .expect("a well-formed UUID");
                    unsafe { WKWebsiteDataStore::dataStoreForIdentifier(&uuid, mtm) }
                })
                .clone()
        }),
    }
}

/// A version 5 style UUID from two FNV-1a hashes of the name. WebKit rejects the nil UUID, and
/// only stability across launches matters here.
fn profile_uuid(name: &str) -> String {
    let fnv = |seed: u64| name.bytes().fold(seed, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3));
    let (a, b) = (fnv(0xcbf2_9ce4_8422_2325), fnv(0x6c62_272e_07bb_0142));
    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(&a.to_be_bytes());
    bytes[8..].copy_from_slice(&b.to_be_bytes());
    bytes[6] = (bytes[6] & 0x0f) | 0x50;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let h: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &h[..8], &h[8..12], &h[12..16], &h[16..20], &h[20..])
}

impl EngineView for WkPage {
    fn ns_view(&self) -> Retained<NSView> {
        Retained::into_super(Retained::into_super(self.web.clone()))
    }

    fn apply(&self, props: &WebViewProps, old: Option<&WebViewProps>) {
        let mut rebuild = false;
        {
            let mut st = self.st.borrow_mut();
            let suppress = props.context_menu_mode == ContextMenuMode::Suppress;
            if st.suppress_menu != suppress {
                st.suppress_menu = suppress;
                rebuild = true;
            }
            st.menu_items = props.context_menu_items.clone();
            if st.count_input != props.count_input {
                st.count_input = props.count_input;
                rebuild = true;
            }
        }
        if rebuild {
            self.rebuild_scripts();
            let suppress = self.st.borrow().suppress_menu;
            self.run_internal(&format!("window.__ndSetSuppressMenu&&window.__ndSetSuppressMenu({suppress})"));
            if props.count_input && !old.is_some_and(|o| o.count_input) {
                self.evaluate(COUNTING_SCRIPT, None, |_| {});
            }
        }
        if let Some(address) = props.url.as_deref().filter(|u| !u.is_empty()) {
            let differs = {
                let st = self.st.borrow();
                let current = convert::url_string(unsafe { self.web.URL() }.as_deref());
                address != current && address != st.committed_url && address != st.provisional_url
            };
            if differs && old.is_none_or(|o| o.url.as_deref() != Some(address)) {
                self.load(address);
            }
        }
    }

    fn command(&self, command: Command) {
        let web = &self.web;
        match command {
            Command::Load(address) => self.load(&address),
            Command::LoadHtml { html, base_url } => {
                let base = base_url.as_deref().and_then(url);
                unsafe { web.loadHTMLString_baseURL(&ns(&html), base.as_deref()) };
            }
            Command::GoBack => unsafe {
                if web.canGoBack() {
                    web.goBack();
                }
            },
            Command::GoForward => unsafe {
                if web.canGoForward() {
                    web.goForward();
                }
            },
            Command::Reload => unsafe {
                web.reload();
            },
            Command::Stop => {
                // WebKit ignores a stop that lands before the navigation reaches the web process
                // and then reports the page as loading forever; such a stop waits for the start.
                let mut st = self.st.borrow_mut();
                if st.awaiting_start {
                    st.pending_stop = true;
                } else {
                    drop(st);
                    unsafe { web.stopLoading() };
                }
            }
            Command::SetZoom(factor) => {
                unsafe { web.setPageZoom(factor) };
                self.page.report_zoom(factor, ZoomSource::App);
            }
            Command::SetUserAgent(ua) => unsafe { web.setCustomUserAgent(ua.as_deref().map(ns).as_deref()) },
            Command::OpenDevTools => self.open_inspector(),
            Command::CloseDevTools => self.close_inspector(),
            Command::ExecuteJavaScript { code, world, done } => self.evaluate(&code, world.as_deref(), done),
            Command::AddUserScript(script) => {
                {
                    let mut st = self.st.borrow_mut();
                    st.scripts.retain(|s| s.id != script.id);
                    st.scripts.push(script);
                }
                self.rebuild_scripts();
            }
            Command::RemoveUserScript(id) => {
                self.st.borrow_mut().scripts.retain(|s| s.id != id);
                self.rebuild_scripts();
            }
            Command::ClearUserScripts(world) => {
                self.st.borrow_mut().scripts.retain(|s| world.is_some() && s.world != world);
                self.rebuild_scripts();
            }
            Command::RegisterScriptMessage { name, world } => {
                if name == INTERNAL_HANDLER {
                    tracing::warn!("web view: {INTERNAL_HANDLER} is reserved");
                    return;
                }
                let key = (name.clone(), world.clone().unwrap_or_default());
                let controller = unsafe { web.configuration().userContentController() };
                let content_world = content_world(self.mtm(), world.as_deref());
                if self.st.borrow().handlers.contains_key(&key) {
                    return;
                }
                unsafe {
                    controller.addScriptMessageHandler_contentWorld_name(
                        ProtocolObject::from_ref(&*self.delegate),
                        &content_world,
                        &ns(&name),
                    )
                };
                self.st.borrow_mut().handlers.insert(key, content_world);
            }
            Command::UnregisterScriptMessage { name, world } => {
                let key = (name.clone(), world.unwrap_or_default());
                if let Some(content_world) = self.st.borrow_mut().handlers.remove(&key) {
                    let controller = unsafe { web.configuration().userContentController() };
                    unsafe { controller.removeScriptMessageHandlerForName_contentWorld(&ns(&name), &content_world) };
                }
            }
            Command::GetCookies { url: filter, done } => {
                let host = filter.as_deref().and_then(url).and_then(|u| u.host()).map(|h| h.to_string());
                let done = Cell::new(Some(done));
                let block = RcBlock::new(move |cookies: std::ptr::NonNull<objc2_foundation::NSArray<NSHTTPCookie>>| {
                    let Some(done) = done.take() else { return };
                    let cookies = unsafe { cookies.as_ref() };
                    let list = cookies
                        .iter()
                        .map(|c| convert::cookie(&c))
                        .filter(|c| host.as_deref().is_none_or(|h| c.matches_host(h)))
                        .collect();
                    done(list);
                });
                unsafe { self.cookie_store().getAllCookies(&block) };
            }
            Command::SetCookie(cookie) => match convert::ns_cookie(&cookie) {
                Some(c) => unsafe { self.cookie_store().setCookie_completionHandler(&c, None) },
                None => tracing::warn!("web view: set_cookie needs a name and a domain"),
            },
            Command::DeleteCookie { name, domain, path } => {
                // The store deletes by identity, so the live cookies are looked up first.
                let store = self.cookie_store();
                let store2 = store.clone();
                let block = RcBlock::new(move |cookies: std::ptr::NonNull<objc2_foundation::NSArray<NSHTTPCookie>>| {
                    for c in unsafe { cookies.as_ref() }.iter() {
                        let matches = c.name().to_string() == name
                            && domain.as_deref().is_none_or(|d| c.domain().to_string() == d)
                            && path.as_deref().is_none_or(|p| c.path().to_string() == p);
                        if matches {
                            unsafe { store2.deleteCookie_completionHandler(&c, None) };
                        }
                    }
                });
                unsafe { store.getAllCookies(&block) };
            }
            Command::Find { text, options } => {
                {
                    let mut st = self.st.borrow_mut();
                    st.find_text = text.clone();
                    st.find_options = options;
                }
                self.find(&text, options);
            }
            Command::FindNext { backwards } => {
                let (text, options) = {
                    let st = self.st.borrow();
                    (st.find_text.clone(), st.find_options)
                };
                if !text.is_empty() {
                    self.find(&text, crate::FindOptions { backwards, ..options });
                }
            }
            Command::StopFind => {
                self.st.borrow_mut().find_text.clear();
                self.evaluate("window.getSelection().removeAllRanges()", None, |_| {});
            }
            Command::SaveSession(done) => {
                let state = unsafe { web.interactionState() };
                let bytes = state.and_then(|s| s.downcast::<NSData>().ok()).map(|d| d.to_vec());
                done(bytes);
            }
            Command::RestoreSession(bytes) => {
                let data = NSData::with_bytes(&bytes);
                unsafe { web.setInteractionState(Some(&data)) };
            }
            Command::SetMuted(muted) => {
                self.run_internal(&format!("window.__ndSetMuted&&window.__ndSetMuted({muted})"))
            }
            Command::ExitFullscreen => {
                self.evaluate("document.fullscreenElement&&document.exitFullscreen()", None, |_| {});
            }
            Command::StartDownload(address) => {
                if let Some(u) = url(&address) {
                    let this = self.this.clone();
                    let block = RcBlock::new(move |download: std::ptr::NonNull<objc2_web_kit::WKDownload>| {
                        if let Some(this) = this.upgrade() {
                            downloads::adopt(&this, unsafe { download.as_ref() });
                        }
                    });
                    unsafe {
                        web.startDownloadUsingRequest_completionHandler(&NSURLRequest::requestWithURL(&u), &block)
                    };
                }
            }
            Command::Focus => {
                if let Some(window) = web.window() {
                    window.makeFirstResponder(Some(web));
                }
            }
            Command::RequestClose => self.page.emit(CloseApproved { approved: true }),
            Command::ResetPermissions { .. } => {
                tracing::debug!("web view: WebKit stores no permission decisions to reset");
            }
            Command::AllowPopups { .. } => unsafe {
                web.configuration().preferences().setJavaScriptCanOpenWindowsAutomatically(true);
            },
            Command::ForwardKey => {
                let app = NSApplication::sharedApplication(self.mtm());
                if let Some(event) = app.currentEvent()
                    && matches!(event.r#type(), NSEventType::KeyDown | NSEventType::KeyUp)
                {
                    forward_key(web, &event);
                }
            }
            command @ (Command::ExtensionAction { .. } | Command::RunExtensionAction { .. }) => {
                crate::handle::fail(
                    command,
                    "WebKit has no extension runtime; extensions run on Chromium in Chrome style",
                );
            }
            Command::CloseExtensionPopup => {}
            Command::Print => {
                let operation = unsafe { web.printOperationWithPrintInfo(&NSPrintInfo::sharedPrintInfo()) };
                // WebKit lays out nothing for an operation whose view has an empty frame.
                if let Some(view) = operation.view() {
                    view.setFrame(web.bounds());
                }
                match web.window() {
                    Some(window) => unsafe {
                        operation.runOperationModalForWindow_delegate_didRunSelector_contextInfo(
                            &window,
                            None,
                            None,
                            std::ptr::null_mut(),
                        )
                    },
                    None => tracing::warn!("web view: print needs the page in a window"),
                }
            }
            Command::PrintToPdf { path, done } => {
                let done = Cell::new(Some(done));
                let block = RcBlock::new(move |data: *mut NSData, error: *mut objc2_foundation::NSError| {
                    let Some(done) = done.take() else { return };
                    let result = match unsafe { data.as_ref() } {
                        Some(data) => {
                            std::fs::write(&path, data.to_vec()).map_err(|e| format!("{}: {e}", path.display()))
                        }
                        None => Err(unsafe { error.as_ref() }
                            .map(|e| e.localizedDescription().to_string())
                            .unwrap_or_else(|| "WebKit made no PDF".into())),
                    };
                    done(result);
                });
                unsafe { web.createPDFWithConfiguration_completionHandler(None, &block) };
            }
        }
    }

    fn set_attached(&self, attached: bool) {
        self.st.borrow_mut().attached = attached;
        if attached {
            self.sync_state();
        }
    }

    fn close(&self) {
        if std::mem::replace(&mut self.st.borrow_mut().closed, true) {
            return;
        }
        let observer: &NSObject = &self.delegate;
        for key in OBSERVED {
            unsafe { self.web.removeObserver_forKeyPath(observer, &ns(key)) };
        }
        unsafe {
            self.web.stopLoading();
            self.web.setNavigationDelegate(None);
            self.web.setUIDelegate(None);
            self.cookie_store().removeObserver(ProtocolObject::from_ref(&*self.delegate));
            let controller = self.web.configuration().userContentController();
            controller.removeAllScriptMessageHandlers();
            controller.removeAllUserScripts();
        }
        self.web.removeFromSuperview();
    }
}

fn forward_key(web: &NdWebView, event: &NSEvent) {
    if event.r#type() == NSEventType::KeyDown {
        let _: () = unsafe { msg_send![web, keyDown: event] };
    } else {
        let _: () = unsafe { msg_send![web, keyUp: event] };
    }
}

pub struct WebIvars {
    owner: Weak<WkPage>,
}

define_class!(
    // SAFETY: WKWebView may be subclassed; the overrides call super. No Drop impl.
    #[unsafe(super(WKWebView, NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = WebIvars]
    pub struct NdWebView;

    impl NdWebView {
        /// The engine's menu with the app's items appended, or nothing in suppress mode (the
        /// page-side `preventDefault` covers the rest).
        #[unsafe(method(willOpenMenu:withEvent:))]
        fn will_open_menu(&self, menu: &NSMenu, event: &NSEvent) {
            let owner = self.ivars().owner.upgrade();
            if owner.as_ref().is_some_and(|o| o.st.borrow().suppress_menu) {
                menu.removeAllItems();
                return;
            }
            let _: () = unsafe { msg_send![super(self), willOpenMenu: menu, withEvent: event] };
            if let Some(owner) = owner {
                menu::append(&owner, menu);
            }
        }

        #[unsafe(method(becomeFirstResponder))]
        fn become_first_responder(&self) -> bool {
            let took: bool = unsafe { msg_send![super(self), becomeFirstResponder] };
            if took && let Some(owner) = self.ivars().owner.upgrade() {
                owner.page.report_focus(true);
            }
            took
        }

        #[unsafe(method(resignFirstResponder))]
        fn resign_first_responder(&self) -> bool {
            let gave: bool = unsafe { msg_send![super(self), resignFirstResponder] };
            if gave && let Some(owner) = self.ivars().owner.upgrade() {
                owner.page.report_focus(false);
            }
            gave
        }
    }
);

impl NdWebView {
    fn new(mtm: MainThreadMarker, config: &WKWebViewConfiguration, owner: Weak<WkPage>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(WebIvars { owner });
        unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO, configuration: config] }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn profile_uuids_are_stable_and_versioned() {
        let a = super::profile_uuid("work");
        assert_eq!(a, super::profile_uuid("work"));
        assert_ne!(a, super::profile_uuid("home"));
        assert_eq!(a.len(), 36);
        assert_eq!(&a[14..15], "5");
    }
}
