//! [`WebView`]: the app's handle on one page. The handle owns the page; the `webview()` element
//! shows it. Unmounting the element parks the page (it keeps running, scripts and history
//! included) and mounting the handle again, in any window, shows it without a reload. The page
//! closes when the last handle is dropped and no element shows it.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use nd_ui::App;

use crate::engine::{CefStyle, Command, Engine, EngineView, Page, PageShared, PageState, default_engine, factory};
use crate::extensions::{ActionOutcome, ActionState, Anchor};
use crate::{Cookie, FindOptions, JsError, JsValue, UserScript};

/// Which cookie jar, cache and storage a page uses.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum Profile {
    /// The app's shared persistent store.
    #[default]
    Default,
    /// An in-memory store every page of the session shares (a private window's tabs), gone
    /// when the session and its last page are dropped.
    Private(PrivateSession),
    /// A persistent store of its own, the same one for the same name across launches.
    Named(String),
}

/// One private browsing session: hand a clone to every page of a private window. The store
/// lives while the app holds the session or any page made with it, and is thrown away after.
/// Clones are the same session; compare with `==`.
#[derive(Clone)]
pub struct PrivateSession(Rc<SessionInner>);

struct SessionInner {
    id: u64,
    ended: RefCell<Vec<Box<dyn FnOnce()>>>,
}

impl Drop for SessionInner {
    fn drop(&mut self) {
        for f in self.ended.take() {
            f();
        }
    }
}

impl PrivateSession {
    pub fn new() -> Self {
        thread_local! {
            static NEXT: std::cell::Cell<u64> = const { std::cell::Cell::new(1) };
        }
        let id = NEXT.with(|n| n.replace(n.get() + 1));
        Self(Rc::new(SessionInner { id, ended: RefCell::default() }))
    }

    /// Unique per session for the life of the process.
    pub fn id(&self) -> u64 {
        self.0.id
    }

    /// Runs `f` when the session ends, for engines to drop the store they made for it. Engines
    /// keep the session only by id, or it never ends.
    pub fn on_end(&self, f: impl FnOnce() + 'static) {
        self.0.ended.borrow_mut().push(Box::new(f));
    }
}

impl Default for PrivateSession {
    fn default() -> Self {
        Self::new()
    }
}

impl PartialEq for PrivateSession {
    fn eq(&self, other: &Self) -> bool {
        self.0.id == other.0.id
    }
}

impl Eq for PrivateSession {}

impl std::hash::Hash for PrivateSession {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.id.hash(state);
    }
}

impl std::fmt::Debug for PrivateSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PrivateSession({})", self.0.id)
    }
}

/// Fixed when the page is created.
#[derive(Clone, Debug, Default)]
pub struct WebViewOptions {
    pub profile: Profile,
    /// `None` takes [`crate::default_engine`].
    pub engine: Option<Engine>,
    /// How Chromium hosts the page; `None` takes [`crate::default_cef_style`]. Other engines
    /// ignore it.
    pub cef_style: Option<CefStyle>,
    /// `window.open` and `target=_blank` hand the app a live page ([`crate::NewWindow::popup`])
    /// that keeps `window.opener`, instead of only a URL.
    pub adopt_popups: bool,
    /// Lets the page be inspected (Web Inspector, DevTools). On by default in debug builds.
    pub inspectable: Option<bool>,
    pub user_agent: Option<String>,
}

impl WebViewOptions {
    pub fn engine(mut self, engine: Engine) -> Self {
        self.engine = Some(engine);
        self
    }

    pub fn cef_style(mut self, style: CefStyle) -> Self {
        self.cef_style = Some(style);
        self
    }

    /// The Chromium style this page uses.
    pub fn resolved_cef_style(&self) -> CefStyle {
        self.cef_style.unwrap_or_else(crate::default_cef_style)
    }

    pub fn profile(mut self, profile: Profile) -> Self {
        self.profile = profile;
        self
    }

    pub fn adopt_popups(mut self) -> Self {
        self.adopt_popups = true;
        self
    }

    pub fn inspectable(mut self, inspectable: bool) -> Self {
        self.inspectable = Some(inspectable);
        self
    }

    pub fn user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.user_agent = Some(user_agent.into());
        self
    }

    pub fn is_inspectable(&self) -> bool {
        self.inspectable.unwrap_or(cfg!(debug_assertions))
    }
}

/// A page. Clones share it; compare with `==`.
#[derive(Clone)]
pub struct WebView(pub(crate) Rc<Inner>);

pub(crate) struct Inner {
    pub engine: Engine,
    pub shared: Rc<PageShared>,
    /// The engine's object, built on first mount (or by the opener for an adopted popup).
    pub view: RefCell<Option<Rc<dyn EngineView>>>,
    /// Commands sent before the page had a view, replayed in order at creation.
    pub queue: RefCell<Vec<Command>>,
    /// Elements showing the page: at most one, the last mounted.
    pub mounts: RefCell<u32>,
    pub error: RefCell<Option<String>>,
}

impl Drop for Inner {
    fn drop(&mut self) {
        if let Some(view) = self.view.get_mut().take() {
            view.close();
        }
    }
}

impl Inner {
    pub(crate) fn adopt(&self, view: Rc<dyn EngineView>) {
        *self.view.borrow_mut() = Some(view);
    }
}

impl PartialEq for WebView {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl std::fmt::Debug for WebView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = self.0.shared.state.borrow();
        f.debug_struct("WebView").field("engine", &self.0.engine).field("url", &state.url).finish()
    }
}

impl Default for WebView {
    fn default() -> Self {
        Self::new()
    }
}

impl WebView {
    pub fn new() -> Self {
        Self::with_options(WebViewOptions::default())
    }

    pub fn with_options(options: WebViewOptions) -> Self {
        let engine = options.engine.unwrap_or_else(default_engine).resolved();
        let shared = Rc::new(PageShared {
            options,
            state: RefCell::new(PageState { zoom: 1., ..PageState::default() }),
            target: RefCell::default(),
            props: RefCell::default(),
        });
        WebView(Rc::new(Inner {
            engine,
            shared,
            view: RefCell::default(),
            queue: RefCell::default(),
            mounts: RefCell::new(0),
            error: RefCell::default(),
        }))
    }

    /// A page loading `url` once it is shown.
    pub fn with_url(url: impl Into<String>) -> Self {
        let view = Self::new();
        view.load(url);
        view
    }

    pub fn engine(&self) -> Engine {
        self.0.engine
    }

    pub fn options(&self) -> &WebViewOptions {
        &self.0.shared.options
    }

    pub(crate) fn page(&self) -> Page {
        Page { inner: Rc::downgrade(&self.0), shared: self.0.shared.clone() }
    }

    /// The engine's view, created on first use. `Err` when the engine is not available in this
    /// build, with the reason.
    pub(crate) fn view(&self) -> Result<Rc<dyn EngineView>, String> {
        if let Some(view) = self.0.view.borrow().as_ref() {
            return Ok(view.clone());
        }
        if let Some(error) = self.0.error.borrow().as_ref() {
            return Err(error.clone());
        }
        let created = match factory(self.0.engine) {
            Some(factory) => factory.create(self.page()),
            None => Err(missing_engine(self.0.engine)),
        };
        match created {
            Ok(view) => {
                self.0.adopt(view.clone());
                for command in self.0.queue.take() {
                    view.command(command);
                }
                Ok(view)
            }
            Err(error) => {
                tracing::error!(engine = self.0.engine.name(), "web view: {error}");
                *self.0.error.borrow_mut() = Some(error.clone());
                for command in self.0.queue.take() {
                    fail(command, &error);
                }
                Err(error)
            }
        }
    }

    fn send(&self, command: Command) {
        let view = self.0.view.borrow().clone();
        match view {
            Some(view) => view.command(command),
            None => match self.0.error.borrow().as_ref() {
                Some(error) => fail(command, error),
                None => self.0.queue.borrow_mut().push(command),
            },
        }
    }

    /// Navigation state as last reported.
    pub fn state(&self) -> PageState {
        self.0.shared.state.borrow().clone()
    }

    pub fn url(&self) -> String {
        self.0.shared.state.borrow().url.clone()
    }

    pub fn title(&self) -> String {
        self.0.shared.state.borrow().title.clone()
    }

    pub fn is_loading(&self) -> bool {
        self.0.shared.state.borrow().loading
    }

    pub fn can_go_back(&self) -> bool {
        self.0.shared.state.borrow().can_go_back
    }

    pub fn can_go_forward(&self) -> bool {
        self.0.shared.state.borrow().can_go_forward
    }

    pub fn is_mounted(&self) -> bool {
        *self.0.mounts.borrow() > 0
    }

    pub fn load(&self, url: impl Into<String>) {
        self.send(Command::Load(url.into()));
    }

    pub fn load_html(&self, html: impl Into<String>, base_url: Option<&str>) {
        self.send(Command::LoadHtml { html: html.into(), base_url: base_url.map(Into::into) });
    }

    pub fn go_back(&self) {
        self.send(Command::GoBack);
    }

    pub fn go_forward(&self) {
        self.send(Command::GoForward);
    }

    pub fn reload(&self) {
        self.send(Command::Reload);
    }

    pub fn stop(&self) {
        self.send(Command::Stop);
    }

    /// Page zoom, 1 is 100 %.
    pub fn set_zoom(&self, factor: f64) {
        self.send(Command::SetZoom(factor));
    }

    /// `None` restores the engine's own.
    pub fn set_user_agent(&self, user_agent: Option<&str>) {
        self.send(Command::SetUserAgent(user_agent.map(Into::into)));
    }

    /// Opens the engine's inspector: Web Inspector in its own window on WebKit, DevTools in a
    /// window of its own on Chromium. Needs [`WebViewOptions::inspectable`].
    pub fn open_dev_tools(&self) {
        self.send(Command::OpenDevTools);
    }

    pub fn close_dev_tools(&self) {
        self.send(Command::CloseDevTools);
    }

    /// Runs `code` in the page's main frame and hands its completion value, as JSON, to `done`.
    /// A thrown exception is `Err` with its message; a returned promise is awaited.
    pub fn execute_javascript(
        &self,
        code: impl Into<String>,
        done: impl FnOnce(Result<JsValue, JsError>, &mut App) + 'static,
    ) {
        self.execute_javascript_in(code, None, done);
    }

    /// As [`Self::execute_javascript`], in the named isolated world (`None`: the page's own).
    pub fn execute_javascript_in(
        &self,
        code: impl Into<String>,
        world: Option<&str>,
        done: impl FnOnce(Result<JsValue, JsError>, &mut App) + 'static,
    ) {
        self.send(Command::ExecuteJavaScript {
            code: code.into(),
            world: world.map(Into::into),
            done: Box::new(move |result| Page::defer(move |cx| done(result, cx))),
        });
    }

    pub fn add_user_script(&self, script: UserScript) {
        self.send(Command::AddUserScript(script));
    }

    pub fn remove_user_script(&self, id: impl Into<String>) {
        self.send(Command::RemoveUserScript(id.into()));
    }

    /// Removes every script, or every script of one world.
    pub fn clear_user_scripts(&self, world: Option<&str>) {
        self.send(Command::ClearUserScripts(world.map(Into::into)));
    }

    /// Lets pages post to `name` (`window.webkit.messageHandlers.<name>.postMessage(v)`); the
    /// element hears [`crate::ScriptMessage`].
    pub fn register_script_message(&self, name: impl Into<String>, world: Option<&str>) {
        self.send(Command::RegisterScriptMessage { name: name.into(), world: world.map(Into::into) });
    }

    pub fn unregister_script_message(&self, name: impl Into<String>, world: Option<&str>) {
        self.send(Command::UnregisterScriptMessage { name: name.into(), world: world.map(Into::into) });
    }

    /// The cookies of the page's profile, those sent to `url` when given.
    pub fn get_cookies(&self, url: Option<&str>, done: impl FnOnce(Vec<Cookie>, &mut App) + 'static) {
        self.send(Command::GetCookies {
            url: url.map(Into::into),
            done: Box::new(move |cookies| Page::defer(move |cx| done(cookies, cx))),
        });
    }

    pub fn set_cookie(&self, cookie: Cookie) {
        self.send(Command::SetCookie(cookie));
    }

    /// Deletes the cookies named `name`, narrowed by domain and path when given.
    pub fn delete_cookie(&self, name: impl Into<String>, domain: Option<&str>, path: Option<&str>) {
        self.send(Command::DeleteCookie {
            name: name.into(),
            domain: domain.map(Into::into),
            path: path.map(Into::into),
        });
    }

    /// Finds `text` and selects the first match; [`crate::FindResult`] reports it.
    pub fn find(&self, text: impl Into<String>, options: FindOptions) {
        self.send(Command::Find { text: text.into(), options });
    }

    pub fn find_next(&self) {
        self.send(Command::FindNext { backwards: false });
    }

    pub fn find_previous(&self) {
        self.send(Command::FindNext { backwards: true });
    }

    pub fn stop_find(&self) {
        self.send(Command::StopFind);
    }

    /// The page's history and scroll position as opaque bytes for [`Self::restore_session`];
    /// `None` where the engine cannot.
    pub fn save_session(&self, done: impl FnOnce(Option<Vec<u8>>, &mut App) + 'static) {
        self.send(Command::SaveSession(Box::new(move |state| Page::defer(move |cx| done(state, cx)))));
    }

    pub fn restore_session(&self, state: Vec<u8>) {
        self.send(Command::RestoreSession(state));
    }

    pub fn set_muted(&self, muted: bool) {
        self.send(Command::SetMuted(muted));
    }

    pub fn exit_fullscreen(&self) {
        self.send(Command::ExitFullscreen);
    }

    /// Downloads `url` with the page's profile; it comes back as
    /// [`crate::DownloadRequested`].
    pub fn start_download(&self, url: impl Into<String>) {
        self.send(Command::StartDownload(url.into()));
    }

    /// Runs the page's `beforeunload` handlers; the element hears [`crate::CloseApproved`].
    /// WebKit has no way to run them without leaving the page, so there it approves at once.
    pub fn request_close(&self) {
        self.send(Command::RequestClose);
    }

    /// Forgets stored permission decisions (Chromium), for `origin` or every origin.
    pub fn reset_permissions(&self, origin: Option<&str>) {
        self.send(Command::ResetPermissions { origin: origin.map(Into::into) });
    }

    /// Lets `origin` (or every origin) open popups without a user gesture (Chromium; WebKit
    /// takes `javaScriptCanOpenWindowsAutomatically` for the whole page instead).
    pub fn allow_popups(&self, origin: Option<&str>) {
        self.send(Command::AllowPopups { origin: origin.map(Into::into) });
    }

    /// Gives the page the keyboard.
    pub fn focus(&self) {
        self.send(Command::Focus);
    }

    /// An extension's toolbar state for this page: badge, title, whether a click opens a popup
    /// ([`crate::extensions`]).
    pub fn extension_action(
        &self,
        id: impl Into<String>,
        done: impl FnOnce(Result<ActionState, String>, &mut App) + 'static,
    ) {
        self.send(Command::ExtensionAction {
            id: id.into(),
            done: Box::new(move |result| Page::defer(move |cx| done(result, cx))),
        });
    }

    /// A click on extension `id`'s toolbar button for this page: its popup opens under
    /// `anchor` (the button's rectangle in the page's window), or `onClicked` fires; a click
    /// while its popup is open closes it. The element hears [`crate::ExtensionPopupClosed`] when
    /// the popup goes, however it went.
    pub fn run_extension_action(
        &self,
        id: impl Into<String>,
        anchor: Anchor,
        done: impl FnOnce(Result<ActionOutcome, String>, &mut App) + 'static,
    ) {
        self.send(Command::RunExtensionAction {
            id: id.into(),
            anchor,
            done: Box::new(move |result| Page::defer(move |cx| done(result, cx))),
        });
    }

    pub fn close_extension_popup(&self) {
        self.send(Command::CloseExtensionPopup);
    }

    pub(crate) fn forward_key(&self) {
        self.send(Command::ForwardKey);
    }

    /// Closes the page unless an element shows it within `after`: an adopted popup the app
    /// never mounts.
    pub(crate) fn close_unless_mounted(&self, after: Duration) {
        let weak = Rc::downgrade(&self.0);
        crate::platform::after(after, move || {
            if let Some(inner) = weak.upgrade()
                && *inner.mounts.borrow() == 0
                && let Some(view) = inner.view.borrow_mut().take()
            {
                view.close();
            }
        });
    }
}

fn missing_engine(engine: Engine) -> String {
    match engine {
        Engine::Cef if cfg!(target_os = "linux") => {
            "the Chromium engine is not in this build: nd's `webview` feature links it (nd-cef)".into()
        }
        Engine::Cef => "the Chromium engine is not in this build: enable nd's `cef` feature and set \
                        `[webview] engine = \"cef\"` in nativedesktop.toml"
            .into(),
        Engine::System => crate::platform::SYSTEM_ENGINE_MISSING.into(),
    }
}

/// Answers a command that can no longer run, so no callback is left waiting.
pub(crate) fn fail(command: Command, error: &str) {
    match command {
        Command::ExecuteJavaScript { done, .. } => done(Err(JsError(error.to_owned()))),
        Command::GetCookies { done, .. } => done(Vec::new()),
        Command::SaveSession(done) => done(None),
        Command::ExtensionAction { done, .. } => done(Err(error.to_owned())),
        Command::RunExtensionAction { done, .. } => done(Err(error.to_owned())),
        _ => {}
    }
}
