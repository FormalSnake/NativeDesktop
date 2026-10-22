//! One page: its view, its browser, and the work CEF's handlers hand it.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::{Rc, Weak};

use cef::*;
use nd_webview::events::*;
use nd_webview::menu::filter;
use nd_webview::{
    CefStyle, Command, ContextMenuMode, Cookie, Download, DownloadControl, EngineView, FindOptions, MenuItem, Once,
    Page, PermissionDecision, PermissionResponder, SameSite, UserScript, WebViewProps,
};
use serde_json::json;

use crate::commands::{self, Route};
use crate::devtools::{self, Devtools};
use crate::keys;
use crate::menu::{self, Choice};
use crate::platform::dialogs;
use crate::platform::view::View;

thread_local! {
    static PAGES: RefCell<HashMap<u64, Weak<CefPage>>> = RefCell::default();
    static BROWSERS: RefCell<HashMap<i32, u64>> = RefCell::default();
    static NEXT: Cell<u64> = const { Cell::new(1) };
}

pub(crate) fn get(id: u64) -> Option<Rc<CefPage>> {
    PAGES.with(|p| p.borrow().get(&id).and_then(Weak::upgrade))
}

pub(crate) fn by_browser(browser: i32) -> Option<Rc<CefPage>> {
    BROWSERS.with(|b| b.borrow().get(&browser).copied()).and_then(get)
}

/// Pages with a live browser.
pub(crate) fn live_count() -> usize {
    browser_count()
}

pub(crate) fn browser_count() -> usize {
    BROWSERS.with(|b| b.borrow().len())
}

/// Closes every browser, for shutdown.
pub(crate) fn close_all() {
    let pages: Vec<Rc<CefPage>> = PAGES.with(|p| p.borrow().values().filter_map(Weak::upgrade).collect());
    for page in pages {
        page.close();
    }
}

/// A browser is gone (`on_before_close`).
pub(crate) fn closed(id: u64, browser: Option<i32>) {
    if let Some(browser) = browser {
        BROWSERS.with(|b| b.borrow_mut().remove(&browser));
    }
    if let Some(page) = get(id) {
        page.devtools.detach();
        page.browser.borrow_mut().take();
        page.view.browser_closed();
    }
    crate::extensions::page_closed(id);
    #[cfg(target_os = "linux")]
    crate::platform::chrome::released(id);
}

#[derive(Default)]
struct State {
    menu_items: Vec<MenuItem>,
    suppress_menu: bool,
    count_input: Option<String>,
    /// Counting was asked for before the browser existed (a Chrome-style page's comes later).
    count_input_queued: bool,
    scripts: Vec<(UserScript, Option<String>)>,
    handlers: HashMap<(String, String), Option<String>>,
    find: Option<(String, FindOptions)>,
    committed_url: String,
    last_secure: Option<(bool, bool)>,
    last_hover: String,
    closing: bool,
    closed: bool,
    default_ua: Option<String>,
    downloads: HashMap<u32, DownloadEntry>,
    prompts: HashMap<u64, (String, PermissionResponder)>,
    muted: bool,
    agent: bool,
    favicon: String,
    /// `request_close` is waiting for `beforeunload`.
    requesting_close: bool,
    /// Origins the app allowed popups from; `None` in the set means every origin.
    popups_allowed: Vec<Option<String>>,
    /// Origins a permission was asked for, for `reset_permissions` without an origin.
    asked: Vec<String>,
}

struct DownloadEntry {
    before: Option<BeforeDownloadCallback>,
    item: Option<DownloadItemCallback>,
    path: Option<PathBuf>,
}

pub(crate) struct CefPage {
    pub(crate) id: u64,
    pub(crate) page: Page,
    /// Fixed when the page is created: a style Chromium cannot host here (Chrome style in a
    /// Wayland window) falls back to off-screen.
    pub(crate) style: CefStyle,
    pub(crate) view: View,
    browser: RefCell<Option<Browser>>,
    pub(crate) devtools: Devtools,
    st: RefCell<State>,
    queue: RefCell<Vec<Command>>,
}

impl CefPage {
    fn new(page: Page) -> Rc<CefPage> {
        let id = NEXT.with(|n| n.replace(n.get() + 1));
        let view = crate::platform::view::new(id);
        let style = crate::platform::view::supported(page.options().resolved_cef_style());
        let this = Rc::new(CefPage {
            id,
            page,
            style,
            view,
            browser: RefCell::default(),
            devtools: Devtools::default(),
            st: RefCell::default(),
            queue: RefCell::default(),
        });
        PAGES.with(|p| p.borrow_mut().insert(id, Rc::downgrade(&this)));
        this
    }

    fn browser_settings() -> BrowserSettings {
        BrowserSettings {
            windowless_frame_rate: 60,
            background_color: 0xffff_ffff,
            // Chrome style anchors it to a toolbar nobody sees; zoom is reported to the app.
            chrome_zoom_bubble: cef::State::DISABLED,
            ..BrowserSettings::default()
        }
    }

    pub(crate) fn create(page: Page) -> Result<Rc<dyn EngineView>, String> {
        let this = Self::new(page);
        let mut context = crate::context(&this.page.options().profile);
        let mut client = crate::client::client(this.id, this.style);
        if this.style == CefStyle::Chrome {
            if let Some(context) = &context {
                crate::runtime::quiet_chrome(context);
            }
            this.view.create_chrome(client, &Self::browser_settings(), context)?;
            return Ok(this);
        }
        let browser = browser_host_create_browser_sync(
            Some(&this.view.window_info()),
            Some(&mut client),
            Some(&CefString::from("")),
            Some(&Self::browser_settings()),
            None,
            context.as_mut(),
        )
        .ok_or("Chromium could not create the browser")?;
        this.bind(browser);
        Ok(this)
    }

    /// Chromium created the browser (`create`, or a popup this page adopted).
    pub(crate) fn bind(&self, browser: Browser) {
        if self.browser.borrow().is_some() {
            return;
        }
        BROWSERS.with(|b| b.borrow_mut().insert(browser.identifier(), self.id));
        let host = browser.host();
        *self.browser.borrow_mut() = Some(browser);
        let Some(host) = host else { return };
        self.devtools.attach(self.id, &host);
        let id = self.id;
        self.devtools.call("Browser.getVersion", json!({}), move |r| {
            if let (Ok(v), Some(p)) = (r, get(id)) {
                p.st.borrow_mut().default_ua = v["userAgent"].as_str().map(str::to_owned);
            }
        });
        if let Some(ua) = self.page.options().user_agent.clone() {
            self.set_user_agent(Some(ua));
        }
        self.install_agent();
        self.view.browser_created();
        host.was_resized();
        self.view.wake();
        if std::mem::take(&mut self.st.borrow_mut().count_input_queued) {
            self.count_input();
        }
        let queued = std::mem::take(&mut *self.queue.borrow_mut());
        for command in queued {
            self.run(command);
        }
    }

    /// Counts the pointer input of this document and every later one. A document that commits
    /// while the script is being registered gets it once registration is through: a Chrome-style
    /// page's first load races it. The script's title marks a document that already counts.
    fn count_input(&self) {
        self.st.borrow_mut().count_input = Some(String::new());
        let id = self.id;
        self.devtools.add_script(nd_webview::script::COUNTING_SCRIPT, None, move |identifier| {
            let Some(p) = get(id) else { return };
            p.st.borrow_mut().count_input = Some(identifier.unwrap_or_default());
            let code = format!("if(!document.title.startsWith('down=')){{{}}}", nd_webview::script::COUNTING_SCRIPT);
            p.devtools.evaluate(&code, None, |_| {});
        });
        self.devtools.evaluate(nd_webview::script::COUNTING_SCRIPT, None, |_| {});
    }

    pub(crate) fn host(&self) -> Option<BrowserHost> {
        self.browser.borrow().as_ref()?.host()
    }

    fn main_frame(&self) -> Option<Frame> {
        self.browser.borrow().as_ref()?.main_frame()
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn focused_frame(&self) -> Option<Frame> {
        let browser = self.browser.borrow();
        let browser = browser.as_ref()?;
        browser.focused_frame().or_else(|| browser.main_frame())
    }

    /// The framework's page-side agent, for what Chromium reports nowhere else off-screen:
    /// media playback (an audio handler would capture the audio instead of playing it).
    fn install_agent(&self) {
        if self.st.borrow().agent {
            return;
        }
        self.st.borrow_mut().agent = true;
        let world = nd_webview::script::INTERNAL_WORLD;
        let post = format!(
            "{}(JSON.stringify({{name:'{}',body:m}}))",
            devtools::BINDING,
            nd_webview::script::INTERNAL_HANDLER
        );
        let source = nd_webview::script::agent_source(&post, false);
        self.devtools.bind(world);
        self.devtools.add_script(&source, Some(world), |_| {});
        self.devtools.evaluate(&source, Some(world), |_| {});
    }

    /// A post through [`devtools::BINDING`].
    pub(crate) fn script_message(&self, world: String, payload: &str) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(payload) else { return };
        let name = v["name"].as_str().unwrap_or_default().to_owned();
        let body = v["body"].clone();
        if name == nd_webview::script::INTERNAL_HANDLER && world == nd_webview::script::INTERNAL_WORLD {
            if body["k"].as_str() == Some("audio") {
                let muted = self.st.borrow().muted;
                self.page.emit(AudioStateChanged { playing: body["playing"].as_bool().unwrap_or(false), muted });
            }
            return;
        }
        if world.is_empty() && crate::extensions::view_popup_message(&self.page, &name, &body) {
            return;
        }
        if self.st.borrow().handlers.contains_key(&(name.clone(), world.clone())) {
            self.page.emit(ScriptMessage { name, world, body });
        }
    }

    pub(crate) fn loading_state(&self, loading: bool, back: bool, forward: bool) {
        self.page.report_loading(loading);
        self.page.report_back(back);
        self.page.report_forward(forward);
        if loading {
            self.view.wake();
        }
    }

    pub(crate) fn load_end(&self) {
        let url = self.main_frame().map(|f| crate::text(&f.url())).unwrap_or_default();
        self.st.borrow_mut().committed_url = url;
        self.report_security(None);
        // Off-screen (Alloy style) Chromium does not always report favicon URLs; the page's
        // own `<link rel=icon>` is read like on WebKit.
        let id = self.id;
        self.devtools.evaluate(
            nd_webview::script::FAVICON_SCRIPT,
            Some(nd_webview::script::INTERNAL_WORLD),
            move |r| {
                let (Ok(serde_json::Value::String(raw)), Some(p)) = (r, get(id)) else { return };
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
                    p.report_favicon(v["iconUrl"].as_str().unwrap_or_default().to_owned());
                }
            },
        );
    }

    pub(crate) fn load_error(&self, code: i32, error: String, url: String) {
        // ERR_ABORTED: a newer navigation or a download took over.
        if code == -3 {
            return;
        }
        // ERR_UNKNOWN_URL_SCHEME: `mailto:`, `tel:` and the like are the app's to open.
        if code == -302 {
            self.page.emit(ExternalProtocol { url });
            return;
        }
        self.page.emit(LoadFailed { url: url.clone(), error: error.clone() });
        // Certificate errors are -200 to -299; they are never accepted.
        if (-299..=-200).contains(&code) {
            self.st.borrow_mut().last_secure = Some((false, false));
            self.page.emit(SecurityChanged { secure: false, insecure_content: false, url, error: Some(error) });
        }
    }

    /// TLS state of the shown page, from the visible navigation entry.
    fn report_security(&self, error: Option<String>) {
        let Some(entry) = self.host().and_then(|h| h.visible_navigation_entry()) else { return };
        let url = crate::text(&entry.url());
        let https = url.starts_with("https:");
        let (secure, insecure) = match entry.sslstatus() {
            Some(ssl) if https => {
                let mixed = ssl.content_status() != SslContentStatus::NORMAL_CONTENT;
                (ssl.is_secure_connection() != 0 && !mixed, mixed)
            }
            _ => (false, false),
        };
        let changed = self.st.borrow_mut().last_secure.replace((secure, insecure)) != Some((secure, insecure));
        if changed {
            self.page.emit(SecurityChanged { secure, insecure_content: insecure, url, error });
        }
    }

    pub(crate) fn favicon(&self, urls: Vec<String>) {
        if let Some(icon_url) = urls.into_iter().next() {
            self.report_favicon(icon_url);
        }
    }

    fn report_favicon(&self, icon_url: String) {
        if icon_url.is_empty() || std::mem::replace(&mut self.st.borrow_mut().favicon, icon_url.clone()) == icon_url {
            return;
        }
        let page_url = self.page.state().url;
        self.page.emit(FaviconChanged { page_url, icon_url });
    }

    pub(crate) fn link_hover(&self, url: String) {
        let changed = {
            let mut st = self.st.borrow_mut();
            let changed = st.last_hover != url;
            st.last_hover = url.clone();
            changed
        };
        if changed {
            self.page.emit(LinkHover { url });
        }
    }

    /// Tab off the page's last field: nd's traversal continues past the view.
    pub(crate) fn take_focus(&self, next: bool) {
        self.view.take_focus(next);
    }

    fn disposition(d: WindowOpenDisposition) -> Disposition {
        if d == WindowOpenDisposition::NEW_BACKGROUND_TAB {
            Disposition::BackgroundTab
        } else if d == WindowOpenDisposition::NEW_WINDOW {
            Disposition::Window
        } else if d == WindowOpenDisposition::NEW_POPUP {
            Disposition::Popup
        } else {
            Disposition::ForegroundTab
        }
    }

    /// `window.open`, `target=_blank`, Cmd-click. Returns true to cancel Chromium's own popup.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn before_popup(
        &self,
        url: String,
        disposition: WindowOpenDisposition,
        user_gesture: bool,
        features: Option<PopupFeatures>,
        window_info: Option<&mut WindowInfo>,
        client: Option<&mut Option<Client>>,
        settings: Option<&mut BrowserSettings>,
    ) -> bool {
        let kind = Self::disposition(disposition);
        let features = features.and_then(|f| {
            let pick = |set: i32, v: i32| (set != 0).then_some(f64::from(v));
            let out = WindowFeatures {
                x: pick(f.x_set, f.x),
                y: pick(f.y_set, f.y),
                width: pick(f.width_set, f.width),
                height: pick(f.height_set, f.height),
            };
            (out != WindowFeatures::default()).then_some(out)
        });
        let opener = origin_of(&self.page.state().url);
        let allowed = self.st.borrow().popups_allowed.iter().any(|o| o.as_ref().is_none_or(|o| *o == opener));
        if !user_gesture && kind == Disposition::Popup && !allowed {
            self.page.emit(PopupBlocked { url, disposition: kind });
            return true;
        }
        let (Some(info), Some(client), Some(settings), true) =
            (window_info, client, settings, self.page.options().adopt_popups)
        else {
            self.page.emit(NewWindow { url, disposition: kind, user_gesture, features, popup: None });
            return true;
        };
        let popup = self.page.popup(|page| {
            let child = CefPage::new(page);
            if child.style == CefStyle::Chrome {
                child.view.expect_chrome_popup(info);
            } else {
                *info = child.view.window_info();
            }
            *client = Some(crate::client::client(child.id, child.style));
            *settings = Self::browser_settings();
            child as Rc<dyn EngineView>
        });
        self.page.emit(NewWindow { url, disposition: kind, user_gesture, features, popup: Some(popup) });
        false
    }

    /// A navigation the page asked to open elsewhere (middle click, Cmd-click on a link).
    pub(crate) fn open_from_tab(&self, url: String, disposition: WindowOpenDisposition, user_gesture: bool) -> bool {
        if disposition == WindowOpenDisposition::CURRENT_TAB {
            return false;
        }
        let kind = Self::disposition(disposition);
        self.page.emit(NewWindow { url, disposition: kind, user_gesture, features: None, popup: None });
        true
    }

    /// A Chrome command: the page's own run, browser UI goes to the app, the rest is refused.
    /// Returns whether Chromium should leave it alone.
    pub(crate) fn chrome_command(&self, id: i32) -> bool {
        match commands::route(id) {
            Some(Route::Allow) => false,
            Some(Route::App(command)) => {
                self.page.emit(ChromeCommand { command });
                true
            }
            None => {
                tracing::debug!("nd-cef: Chrome command {id} refused; it would open Chrome's own UI");
                true
            }
        }
    }

    /// `request_close` in Chrome style, where a close Chromium starts cannot be held for the app
    /// (no `DoClose`): the page's `beforeunload` handlers run on a synthetic event, and when one
    /// asks to stay, the same prompt as a real close's decides.
    fn request_close_chrome(&self) {
        const PROBE: &str = "(() => { const e = document.createEvent('BeforeUnloadEvent'); \
            e.initEvent('beforeunload', false, true); window.dispatchEvent(e); \
            return e.defaultPrevented || !!e.returnValue; })()";
        let id = self.id;
        self.devtools.evaluate(PROBE, None, move |result| {
            let Some(page) = get(id) else { return };
            let asks = result.ok().and_then(|v| v.as_bool()).unwrap_or(false);
            if !asks || !dialogs::available(&page.view) {
                page.page.emit(CloseApproved { approved: true });
                return;
            }
            let message = "Changes you made may not be saved.";
            dialogs::message(&page.view, "Leave this page?", message, &["Leave", "Cancel"], None, move |answer| {
                if let Some(page) = get(id) {
                    page.page.emit(CloseApproved { approved: answer.is_some() });
                }
            });
        });
    }

    /// The page closes itself (`window.close()`): the app decides, by unmounting it.
    pub(crate) fn do_close(&self) -> bool {
        if self.st.borrow().closing {
            return false;
        }
        if std::mem::take(&mut self.st.borrow_mut().requesting_close) {
            self.page.emit(CloseApproved { approved: true });
            return true;
        }
        self.page.emit(WindowClosed);
        true
    }

    pub(crate) fn before_context_menu(&self, params: &ContextMenuParams, model: &MenuModel) {
        let hit = menu::hit(params);
        self.page.emit(ContextMenu {
            x: f64::from(params.xcoord()),
            y: f64::from(params.ycoord()),
            link: hit.link,
            image: hit.image,
            selection: hit.selection,
            editable: hit.editable,
        });
        if self.st.borrow().suppress_menu {
            model.clear();
        }
    }

    /// Shows the menu on the next turn of the main loop, outside Chromium's message loop work.
    pub(crate) fn run_context_menu(
        &self,
        params: &ContextMenuParams,
        model: &MenuModel,
        callback: RunContextMenuCallback,
    ) -> bool {
        let hit = menu::hit(params);
        let at = (f64::from(params.xcoord()), f64::from(params.ycoord()));
        let (model, id) = (model.clone(), self.id);
        let page_url = crate::text(&params.page_url());
        crate::platform::run_later(move || {
            let Some(page) = get(id) else {
                callback.cancel();
                return;
            };
            let items = page.st.borrow().menu_items.clone();
            let entries = filter(&items, &hit);
            page.view.show_menu(at, &model, &entries, move |chosen| {
                let Some(page) = get(id) else {
                    callback.cancel();
                    return;
                };
                match chosen {
                    Some(Choice::Chromium(command)) => {
                        commands::running_menu_command(command, || callback.cont(command, EventFlags::default()))
                    }
                    Some(Choice::App(item)) => {
                        callback.cancel();
                        let (checked, was_checked) = nd_webview::menu::implied_check(&item);
                        page.page.emit(ContextMenuItemClicked {
                            id: item.id,
                            page_url,
                            link_url: hit.link,
                            image_url: hit.image,
                            selection_text: hit.selection,
                            editable: hit.editable,
                            checked,
                            was_checked,
                        });
                    }
                    None => callback.cancel(),
                }
                page.view.wake();
            });
        });
        true
    }

    pub(crate) fn before_download(
        &self,
        item: &mut DownloadItem,
        suggested: String,
        callback: BeforeDownloadCallback,
    ) -> bool {
        let id = item.id();
        self.st.borrow_mut().downloads.insert(id, DownloadEntry { before: Some(callback), item: None, path: None });
        let url = crate::text(&item.url());
        let mime = Some(crate::text(&item.mime_type())).filter(|m| !m.is_empty());
        let total = u64::try_from(item.total_bytes()).ok().filter(|t| *t > 0);
        let control = Rc::new(DownloadHandle { page: self.id, id, decided: Cell::new(false) });
        let download = Download::new(u64::from(id), url, suggested, mime, total, control);
        self.page.emit(DownloadRequested { download });
        true
    }

    pub(crate) fn download_updated(&self, item: &mut DownloadItem, callback: DownloadItemCallback) {
        let id = item.id();
        let path = {
            let mut st = self.st.borrow_mut();
            let Some(entry) = st.downloads.get_mut(&id) else { return };
            entry.item = Some(callback);
            let full = crate::text(&item.full_path());
            if !full.is_empty() {
                entry.path = Some(PathBuf::from(full));
            }
            entry.path.clone()
        };
        let state = if item.is_complete() != 0 {
            DownloadState::Done
        } else if item.is_canceled() != 0 {
            DownloadState::Cancelled
        } else if item.is_interrupted() != 0 {
            DownloadState::Failed
        } else {
            DownloadState::Running
        };
        if path.is_none() && state == DownloadState::Running {
            return;
        }
        self.page.emit(DownloadUpdated {
            id: u64::from(id),
            state,
            received: u64::try_from(item.received_bytes()).unwrap_or(0),
            total: u64::try_from(item.total_bytes()).ok().filter(|t| *t > 0),
            path,
            paused: item.is_paused() != 0,
            error: (state == DownloadState::Failed)
                .then(|| format!("interrupted ({})", item.interrupt_reason().get_raw())),
        });
        if matches!(state, DownloadState::Done | DownloadState::Cancelled) {
            self.st.borrow_mut().downloads.remove(&id);
        }
    }

    pub(crate) fn js_dialog(
        &self,
        kind: JsdialogType,
        message: String,
        default: String,
        callback: JsdialogCallback,
    ) -> bool {
        let (title, buttons, field): (&str, &[&str], Option<String>) = if kind == JsdialogType::CONFIRM {
            ("Confirm", &["OK", "Cancel"], None)
        } else if kind == JsdialogType::PROMPT {
            ("The page is asking for input", &["OK", "Cancel"], Some(default))
        } else {
            ("The page says", &["OK"], None)
        };
        if !dialogs::available(&self.view) {
            callback.cont(0, None);
            return true;
        }
        dialogs::message(&self.view, title, &message, buttons, field, move |answer| match answer {
            Some(text) => callback.cont(1, Some(&CefString::from(text.as_str()))),
            None => callback.cont(0, None),
        });
        true
    }

    pub(crate) fn before_unload(&self, reload: bool, callback: JsdialogCallback) -> bool {
        if !dialogs::available(&self.view) {
            callback.cont(1, None);
            return true;
        }
        let (title, button) = if reload { ("Reload this page?", "Reload") } else { ("Leave this page?", "Leave") };
        let id = self.id;
        let message = "Changes you made may not be saved.";
        dialogs::message(&self.view, title, message, &[button, "Cancel"], None, move |answer| {
            callback.cont(i32::from(answer.is_some()), None);
            if answer.is_none()
                && let Some(p) = get(id)
                && std::mem::take(&mut p.st.borrow_mut().requesting_close)
            {
                p.page.emit(CloseApproved { approved: false });
            }
        });
        true
    }

    pub(crate) fn file_dialog(
        &self,
        mode: FileDialogMode,
        default: String,
        extensions: Vec<String>,
        callback: FileDialogCallback,
    ) -> bool {
        if !dialogs::available(&self.view) {
            callback.cancel();
            return true;
        }
        dialogs::files(&self.view, mode, default, extensions, move |paths| match paths {
            Some(paths) => {
                let mut list = CefStringList::new();
                for path in paths {
                    list.append(&path.to_string_lossy());
                }
                callback.cont(Some(&mut list));
            }
            None => callback.cancel(),
        });
        true
    }

    pub(crate) fn media_permission(&self, origin: String, requested: u32, callback: MediaAccessCallback) -> bool {
        let mut kinds = Vec::new();
        if requested & (1 | 4) != 0 {
            kinds.push(PermissionKind::Microphone);
        }
        if requested & (2 | 8) != 0 {
            kinds.push(PermissionKind::Camera);
        }
        let responder = PermissionResponder(Once::new(PermissionDecision::Dismiss, move |d| match d {
            PermissionDecision::Allow => callback.cont(requested),
            PermissionDecision::Deny | PermissionDecision::Dismiss => callback.cancel(),
        }));
        self.page.emit(PermissionRequest { origin: clean_origin(&origin), kinds, responder });
        true
    }

    pub(crate) fn permission_prompt(
        &self,
        prompt: u64,
        origin: String,
        requested: u32,
        callback: PermissionPromptCallback,
    ) -> bool {
        let mut kinds = Vec::new();
        for (bit, kind) in [
            (4u32, PermissionKind::Camera),
            (4096, PermissionKind::Microphone),
            (256, PermissionKind::Geolocation),
            (32768, PermissionKind::Notifications),
            (16, PermissionKind::Clipboard),
        ] {
            if requested & bit != 0 {
                kinds.push(kind);
            }
        }
        if kinds.is_empty() {
            kinds.push(PermissionKind::Other);
        }
        let responder = PermissionResponder(Once::new(PermissionDecision::Dismiss, move |d| {
            callback.cont(match d {
                PermissionDecision::Allow => PermissionRequestResult::ACCEPT,
                PermissionDecision::Deny => PermissionRequestResult::DENY,
                PermissionDecision::Dismiss => PermissionRequestResult::DISMISS,
            })
        }));
        let origin = clean_origin(&origin);
        {
            let mut st = self.st.borrow_mut();
            st.prompts.insert(prompt, (origin.clone(), responder.clone()));
            if !st.asked.contains(&origin) {
                st.asked.push(origin.clone());
            }
        }
        self.page.emit(PermissionRequest { origin, kinds, responder });
        true
    }

    /// Chromium retired the prompt itself (a navigation, a closed page): later answers do nothing.
    pub(crate) fn permission_dismissed(&self, prompt: u64) {
        let removed = self.st.borrow_mut().prompts.remove(&prompt);
        if let Some((origin, responder)) = removed {
            responder.0.withdraw();
            self.page.emit(PermissionRequestDismissed { origin });
        }
    }

    fn set_user_agent(&self, ua: Option<String>) {
        let ua = ua.or_else(|| self.st.borrow().default_ua.clone()).unwrap_or_default();
        self.devtools.call("Emulation.setUserAgentOverride", json!({ "userAgent": ua }), |_| {});
    }

    fn sync_scripts(&self) {
        let pending: Vec<UserScript> =
            self.st.borrow().scripts.iter().filter(|(_, id)| id.is_none()).map(|(s, _)| s.clone()).collect();
        for script in pending {
            let source = devtools::script_source(&script);
            let (id, key) = (self.id, script.id.clone());
            self.devtools.add_script(&source, script.world.as_deref(), move |identifier| {
                if let Some(p) = get(id)
                    && let Some(entry) = p.st.borrow_mut().scripts.iter_mut().find(|(s, _)| s.id == key)
                {
                    entry.1 = identifier;
                }
            });
        }
    }

    fn cookie_manager(&self) -> Option<CookieManager> {
        let host = self.host()?;
        host.request_context()?.cookie_manager(None)
    }

    fn run(&self, command: Command) {
        let Some(browser) = self.browser.borrow().clone() else {
            self.queue.borrow_mut().push(command);
            return;
        };
        let host = browser.host();
        self.view.wake();
        match command {
            Command::Load(url) => {
                if let Some(frame) = browser.main_frame() {
                    frame.load_url(Some(&CefString::from(url.as_str())));
                }
            }
            Command::LoadHtml { html, base_url } => {
                if base_url.is_some() {
                    tracing::warn!("nd-cef: load_html ignores base_url; the page loads as a data: URL");
                }
                let url = format!("data:text/html;charset=utf-8;base64,{}", base64(html.as_bytes()));
                if let Some(frame) = browser.main_frame() {
                    frame.load_url(Some(&CefString::from(url.as_str())));
                }
            }
            Command::GoBack => browser.go_back(),
            Command::GoForward => browser.go_forward(),
            Command::Reload => browser.reload(),
            Command::Stop => browser.stop_load(),
            Command::SetZoom(factor) => {
                if let Some(host) = host {
                    host.set_zoom_level(keys::zoom_level(factor));
                }
                self.page.report_zoom(factor, ZoomSource::App);
            }
            Command::SetUserAgent(ua) => self.set_user_agent(ua),
            Command::OpenDevTools => {
                if let Some(host) = host {
                    // Its own window: an off-screen page has no window of Chromium's to dock
                    // into. Asking for Alloy logs that DevTools cannot use it and falls back, but
                    // the default style here leaves the page's own DevTools session hanging.
                    let info = WindowInfo {
                        window_name: CefString::from("Developer Tools"),
                        bounds: Rect { x: 120, y: 120, width: 1000, height: 700 },
                        runtime_style: RuntimeStyle::ALLOY,
                        ..WindowInfo::default()
                    };
                    let mut client = crate::client::devtools_client();
                    host.show_dev_tools(Some(&info), Some(&mut client), Some(&BrowserSettings::default()), None);
                }
            }
            Command::CloseDevTools => {
                if let Some(host) = host {
                    host.close_dev_tools();
                }
            }
            Command::ExecuteJavaScript { code, world, done } => {
                self.devtools.evaluate_as_user(&code, world.as_deref(), move |r| done(r.map_err(nd_webview::JsError)));
            }
            Command::AddUserScript(script) => {
                let old = {
                    let mut st = self.st.borrow_mut();
                    let old = st.scripts.iter().position(|(s, _)| s.id == script.id).map(|i| st.scripts.remove(i));
                    st.scripts.push((script, None));
                    old
                };
                if let Some((_, Some(identifier))) = old {
                    self.devtools.remove_script(&identifier);
                }
                self.sync_scripts();
            }
            Command::RemoveUserScript(id) => {
                let removed: Vec<_> = {
                    let mut st = self.st.borrow_mut();
                    let (gone, kept): (Vec<_>, Vec<_>) =
                        std::mem::take(&mut st.scripts).into_iter().partition(|(s, _)| s.id == id);
                    st.scripts = kept;
                    gone
                };
                for (_, identifier) in removed {
                    if let Some(identifier) = identifier {
                        self.devtools.remove_script(&identifier);
                    }
                }
            }
            Command::ClearUserScripts(world) => {
                let removed: Vec<_> = {
                    let mut st = self.st.borrow_mut();
                    let (gone, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut st.scripts)
                        .into_iter()
                        .partition(|(s, _)| world.is_none() || s.world == world);
                    st.scripts = kept;
                    gone
                };
                for (_, identifier) in removed.into_iter() {
                    if let Some(identifier) = identifier {
                        self.devtools.remove_script(&identifier);
                    }
                }
            }
            Command::RegisterScriptMessage { name, world } => {
                if name == nd_webview::script::INTERNAL_HANDLER {
                    tracing::warn!("nd-cef: {name} is reserved");
                    return;
                }
                let world = world.unwrap_or_default();
                let key = (name.clone(), world.clone());
                if self.st.borrow().handlers.contains_key(&key) {
                    return;
                }
                self.st.borrow_mut().handlers.insert(key.clone(), None);
                self.devtools.bind(&world);
                let shim = devtools::handler_shim(&name);
                let id = self.id;
                self.devtools.add_script(&shim, Some(&world), move |identifier| {
                    if let Some(p) = get(id)
                        && let Some(slot) = p.st.borrow_mut().handlers.get_mut(&key)
                    {
                        *slot = identifier;
                    }
                });
                self.devtools.evaluate(&shim, Some(&world), |_| {});
            }
            Command::UnregisterScriptMessage { name, world } => {
                let world = world.unwrap_or_default();
                if let Some(identifier) = self.st.borrow_mut().handlers.remove(&(name.clone(), world.clone())) {
                    if let Some(identifier) = identifier {
                        self.devtools.remove_script(&identifier);
                    }
                    self.devtools.evaluate(&devtools::remove_shim(&name), Some(&world), |_| {});
                }
            }
            Command::GetCookies { url, done } => match self.cookie_manager() {
                Some(manager) => {
                    let collector = Rc::new(Collector { cookies: RefCell::default(), done: Cell::new(Some(done)) });
                    let mut visitor = NdCookies::new(CollectorHandle(collector.clone()));
                    let started = match &url {
                        Some(url) => {
                            manager.visit_url_cookies(Some(&CefString::from(url.as_str())), 1, Some(&mut visitor))
                        }
                        None => manager.visit_all_cookies(Some(&mut visitor)),
                    };
                    drop(visitor);
                    if started == 0 {
                        collector.finish();
                    }
                }
                None => done(Vec::new()),
            },
            Command::SetCookie(cookie) => {
                if let Some(manager) = self.cookie_manager() {
                    let url = cookie_url(&cookie.domain, &cookie.path, cookie.secure);
                    manager.set_cookie(Some(&CefString::from(url.as_str())), Some(&cef_cookie(&cookie)), None);
                }
            }
            Command::DeleteCookie { name, domain, path } => {
                let Some(manager) = self.cookie_manager() else { return };
                match domain {
                    Some(domain) => {
                        let url = cookie_url(&domain, path.as_deref().unwrap_or("/"), false);
                        manager.delete_cookies(
                            Some(&CefString::from(url.as_str())),
                            Some(&CefString::from(name.as_str())),
                            None,
                        );
                    }
                    None => {
                        let collector = Rc::new(Collector {
                            cookies: RefCell::default(),
                            done: Cell::new(Some(Box::new(move |cookies: Vec<Cookie>| {
                                for c in cookies.into_iter().filter(|c| c.name == name) {
                                    if path.as_deref().is_some_and(|p| p != c.path) {
                                        continue;
                                    }
                                    let url = cookie_url(&c.domain, &c.path, c.secure);
                                    manager.delete_cookies(
                                        Some(&CefString::from(url.as_str())),
                                        Some(&CefString::from(c.name.as_str())),
                                        None,
                                    );
                                }
                            }))),
                        });
                        let mut visitor = NdCookies::new(CollectorHandle(collector.clone()));
                        if let Some(manager) = self.cookie_manager() {
                            manager.visit_all_cookies(Some(&mut visitor));
                        }
                    }
                }
            }
            Command::Find { text, options } => {
                self.st.borrow_mut().find = Some((text.clone(), options));
                if let Some(host) = host {
                    host.find(
                        Some(&CefString::from(text.as_str())),
                        i32::from(!options.backwards),
                        i32::from(options.case_sensitive),
                        0,
                    );
                }
            }
            Command::FindNext { backwards } => {
                let find = self.st.borrow().find.clone();
                if let (Some((text, options)), Some(host)) = (find, host) {
                    host.find(
                        Some(&CefString::from(text.as_str())),
                        i32::from(!backwards),
                        i32::from(options.case_sensitive),
                        1,
                    );
                }
            }
            Command::StopFind => {
                self.st.borrow_mut().find = None;
                if let Some(host) = host {
                    host.stop_finding(1);
                }
            }
            Command::SaveSession(done) => {
                tracing::warn!("nd-cef: CEF has no session state to save; save_session answers None");
                done(None);
            }
            Command::RestoreSession(_) => tracing::warn!("nd-cef: CEF has no session state to restore"),
            Command::SetMuted(muted) => {
                self.st.borrow_mut().muted = muted;
                if let Some(host) = host {
                    host.set_audio_muted(i32::from(muted));
                }
                // The agent mutes elements added later too, and reports the new state.
                let code = format!("window.__ndSetMuted&&window.__ndSetMuted({muted})");
                self.devtools.evaluate(&code, Some(nd_webview::script::INTERNAL_WORLD), |_| {});
            }
            Command::RequestClose if self.style == CefStyle::Chrome => self.request_close_chrome(),
            Command::RequestClose => {
                let Some(host) = host else { return };
                self.st.borrow_mut().requesting_close = true;
                // Runs `beforeunload`; an approved close comes back through `do_close`, which
                // keeps the browser for the app to unmount.
                if host.try_close_browser() != 0 && std::mem::take(&mut self.st.borrow_mut().requesting_close) {
                    self.page.emit(CloseApproved { approved: true });
                }
            }
            Command::ResetPermissions { origin } => {
                let Some(context) = host.and_then(|h| h.request_context()) else { return };
                let origins = match origin {
                    Some(o) => vec![o],
                    None => self.st.borrow().asked.clone(),
                };
                for origin in origins {
                    let url = CefString::from(format!("{origin}/").as_str());
                    for kind in [
                        ContentSettingTypes::MEDIASTREAM_CAMERA,
                        ContentSettingTypes::MEDIASTREAM_MIC,
                        ContentSettingTypes::GEOLOCATION,
                        ContentSettingTypes::NOTIFICATIONS,
                        ContentSettingTypes::CLIPBOARD_READ_WRITE,
                    ] {
                        context.set_content_setting(Some(&url), Some(&url), kind, ContentSettingValues::DEFAULT);
                    }
                }
            }
            Command::AllowPopups { origin } => {
                if let (Some(o), Some(context)) = (&origin, host.and_then(|h| h.request_context())) {
                    let url = CefString::from(format!("{o}/").as_str());
                    context.set_content_setting(
                        Some(&url),
                        Some(&url),
                        ContentSettingTypes::POPUPS,
                        ContentSettingValues::ALLOW,
                    );
                }
                self.st.borrow_mut().popups_allowed.push(origin);
            }
            Command::ExitFullscreen => {
                if let Some(host) = host {
                    host.exit_fullscreen(1);
                }
            }
            Command::StartDownload(url) => {
                if let Some(host) = host {
                    host.start_download(Some(&CefString::from(url.as_str())));
                }
            }
            Command::Focus => self.view.focus(),
            Command::ForwardKey => self.view.forward_current_key(),
            Command::ExtensionAction { id, done } => match self.style {
                CefStyle::Chrome => crate::extensions::action_state(self, id, done),
                CefStyle::Osr => done(Err(NOT_CHROME.into())),
            },
            Command::RunExtensionAction { id, anchor, done } => match self.style {
                CefStyle::Chrome => crate::extensions::run_action(self, id, anchor, done),
                CefStyle::Osr => done(Err(NOT_CHROME.into())),
            },
            Command::CloseExtensionPopup => crate::extensions::close_popup(self.id),
            // Chromium's print panel on macOS is a sheet on the page's own window, which an
            // off-screen page does not have.
            Command::Print if cfg!(target_os = "macos") && self.style == CefStyle::Osr => {
                tracing::warn!(
                    "nd-cef: an off-screen page has no print panel on macOS; use print_to_pdf or Chrome style"
                );
            }
            Command::Print => {
                if let Some(host) = host {
                    host.print();
                }
            }
            Command::PrintToPdf { path, done } => match host {
                Some(host) => crate::pdf::print_to_pdf(&host, &path, done),
                None => done(Err("the page has no browser yet".into())),
            },
        }
    }

    /// The page's DevTools target id.
    pub(crate) fn target_id(&self, then: impl FnOnce(Result<String, String>) + 'static) {
        self.devtools.call("Target.getTargetInfo", json!({}), move |r| {
            then(r.and_then(|v| {
                v["targetInfo"]["targetId"].as_str().map(str::to_owned).ok_or_else(|| "no target id".to_owned())
            }));
        });
    }
}

const NOT_CHROME: &str = "extension actions run in Chrome-style pages (`[webview.cef] style = \"chrome\"`)";

impl EngineView for CefPage {
    #[cfg(target_os = "macos")]
    fn ns_view(&self) -> objc2::rc::Retained<objc2_app_kit::NSView> {
        objc2::rc::Retained::into_super(self.view.clone())
    }

    #[cfg(target_os = "linux")]
    fn element(
        &self,
        cx: &nd_backend_gpui::custom::CustomCx<'_>,
        window: &mut nd_gpui::gpui::Window,
    ) -> Option<nd_gpui::gpui::AnyElement> {
        Some(self.view.element(cx, window))
    }

    fn apply(&self, props: &WebViewProps, old: Option<&WebViewProps>) {
        {
            let mut st = self.st.borrow_mut();
            st.suppress_menu = props.context_menu_mode == ContextMenuMode::Suppress;
            st.menu_items = props.context_menu_items.clone();
        }
        self.st.borrow_mut().count_input_queued = props.count_input && self.host().is_none();
        if props.count_input && self.host().is_some() && self.st.borrow().count_input.is_none() {
            self.count_input();
        } else if !props.count_input
            && let Some(identifier) = self.st.borrow_mut().count_input.take()
            && !identifier.is_empty()
        {
            self.devtools.remove_script(&identifier);
        }
        if let Some(url) = props.url.as_deref().filter(|u| !u.is_empty()) {
            let differs = {
                let st = self.st.borrow();
                url != self.page.state().url && url != st.committed_url
            };
            if differs && old.is_none_or(|o| o.url.as_deref() != Some(url)) {
                self.run(Command::Load(url.to_owned()));
            }
        }
    }

    fn command(&self, command: Command) {
        self.run(command);
    }

    fn set_attached(&self, attached: bool) {
        if let Some(host) = self.host() {
            host.was_hidden(i32::from(!attached));
        }
        if !attached {
            self.view.stop_link();
        }
    }

    fn close(&self) {
        {
            let mut st = self.st.borrow_mut();
            if std::mem::replace(&mut st.closed, true) {
                return;
            }
            st.closing = true;
        }
        self.view.detach();
        if let Some(host) = self.host() {
            host.close_browser(1);
        }
        PAGES.with(|p| p.borrow_mut().remove(&self.id));
    }
}

impl Drop for CefPage {
    fn drop(&mut self) {
        self.close();
    }
}

/// `scheme://host[:port]` of a URL.
fn origin_of(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else { return String::new() };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    format!("{scheme}://{host}")
}

/// `https://example.com/` style origin, without the trailing slash Chromium's GURL adds.
fn clean_origin(origin: &str) -> String {
    origin.strip_suffix('/').unwrap_or(origin).to_owned()
}

fn cookie_url(domain: &str, path: &str, secure: bool) -> String {
    let host = domain.trim_start_matches('.');
    let path = if path.starts_with('/') { path.to_owned() } else { format!("/{path}") };
    format!("{}://{host}{path}", if secure { "https" } else { "http" })
}

fn cef_cookie(c: &Cookie) -> cef::Cookie {
    let mut out = cef::Cookie {
        name: CefString::from(c.name.as_str()),
        value: CefString::from(c.value.as_str()),
        domain: CefString::from(c.domain.as_str()),
        path: CefString::from(if c.path.is_empty() { "/" } else { c.path.as_str() }),
        secure: i32::from(c.secure),
        httponly: i32::from(c.http_only),
        same_site: match c.same_site {
            SameSite::Lax => CookieSameSite::from(sys::cef_cookie_same_site_t::CEF_COOKIE_SAME_SITE_LAX_MODE),
            SameSite::Strict => CookieSameSite::from(sys::cef_cookie_same_site_t::CEF_COOKIE_SAME_SITE_STRICT_MODE),
            // Unspecified, not `None`: Chromium refuses `SameSite=None` without `Secure`.
            SameSite::None => CookieSameSite::UNSPECIFIED,
        },
        ..cef::Cookie::default()
    };
    if let Some(expires) = c.expires {
        out.has_expires = 1;
        out.expires = Basetime { val: unix_to_basetime(expires) };
    }
    out
}

/// `cef_basetime_t` counts microseconds since 1601-01-01 (Windows epoch).
const EPOCH_DELTA_US: i64 = 11_644_473_600_000_000;

fn unix_to_basetime(seconds: f64) -> i64 {
    (seconds * 1e6) as i64 + EPOCH_DELTA_US
}

fn basetime_to_unix(val: i64) -> f64 {
    (val - EPOCH_DELTA_US) as f64 / 1e6
}

fn nd_cookie(c: &cef::Cookie) -> Cookie {
    let same_site = if c.same_site == CookieSameSite::from(sys::cef_cookie_same_site_t::CEF_COOKIE_SAME_SITE_LAX_MODE) {
        SameSite::Lax
    } else if c.same_site == CookieSameSite::from(sys::cef_cookie_same_site_t::CEF_COOKIE_SAME_SITE_STRICT_MODE) {
        SameSite::Strict
    } else {
        SameSite::None
    };
    Cookie {
        name: c.name.to_string(),
        value: c.value.to_string(),
        domain: c.domain.to_string(),
        path: c.path.to_string(),
        secure: c.secure != 0,
        http_only: c.httponly != 0,
        expires: (c.has_expires != 0).then(|| basetime_to_unix(c.expires.val)),
        same_site,
    }
}

/// Gathers cookies until CEF lets the visitor go (after the last cookie, or at once when there
/// are none), then answers.
type CookiesDone = Box<dyn FnOnce(Vec<Cookie>)>;

struct Collector {
    cookies: RefCell<Vec<Cookie>>,
    done: Cell<Option<CookiesDone>>,
}

impl Collector {
    fn finish(&self) {
        if let Some(done) = self.done.take() {
            done(self.cookies.take());
        }
    }
}

impl Drop for Collector {
    fn drop(&mut self) {
        self.finish();
    }
}

/// The collector, as a field CEF's visitor can clone. Visitors run and are released on the UI
/// thread, which is the main thread here.
#[derive(Clone)]
struct CollectorHandle(Rc<Collector>);

wrap_cookie_visitor! {
    struct NdCookies {
        collector: CollectorHandle,
    }

    impl CookieVisitor {
        fn visit(&self, cookie: Option<&cef::Cookie>, _count: i32, _total: i32, _delete_cookie: Option<&mut i32>) -> i32 {
            if let Some(cookie) = cookie {
                self.collector.0.cookies.borrow_mut().push(nd_cookie(cookie));
            }
            1
        }
    }
}

struct DownloadHandle {
    page: u64,
    id: u32,
    decided: Cell<bool>,
}

impl DownloadHandle {
    fn with(&self, f: impl FnOnce(&mut DownloadEntry)) {
        if let Some(p) = get(self.page)
            && let Some(entry) = p.st.borrow_mut().downloads.get_mut(&self.id)
        {
            f(entry);
        }
    }
}

impl Drop for DownloadHandle {
    /// Nobody chose a destination: dropping CEF's callback unexecuted cancels the download.
    fn drop(&mut self) {
        if !self.decided.get() {
            self.cancel();
        }
    }
}

impl DownloadControl for DownloadHandle {
    fn save_to(&self, path: PathBuf) {
        self.decided.set(true);
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        self.with(|e| {
            e.path = Some(path.clone());
            if let Some(before) = e.before.take() {
                before.cont(Some(&CefString::from(path.to_string_lossy().as_ref())), 0);
            }
        });
    }

    fn cancel(&self) {
        self.decided.set(true);
        self.with(|e| {
            e.before.take();
            if let Some(item) = &e.item {
                item.cancel();
            }
        });
    }

    fn pause(&self) {
        self.with(|e| {
            if let Some(item) = &e.item {
                item.pause();
            }
        });
    }

    fn resume(&self) {
        self.with(|e| {
            if let Some(item) = &e.item {
                item.resume();
            }
        });
    }
}

fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(T[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_pads() {
        assert_eq!(base64(b"hi"), "aGk=");
        assert_eq!(base64(b"<p>"), "PHA+");
        assert_eq!(base64(b"a"), "YQ==");
    }

    #[test]
    fn cookie_urls_and_times() {
        assert_eq!(cookie_url(".example.com", "/a", true), "https://example.com/a");
        assert_eq!(cookie_url("x.org", "", false), "http://x.org/");
        assert!((basetime_to_unix(unix_to_basetime(1_700_000_000.5)) - 1_700_000_000.5).abs() < 1e-3);
        assert_eq!(clean_origin("https://a.b/"), "https://a.b");
        assert_eq!(origin_of("http://127.0.0.1:8080/x?y"), "http://127.0.0.1:8080");
    }
}
