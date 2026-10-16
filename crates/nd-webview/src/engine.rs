//! The seam between the app-facing API and a web engine. WebKit (`WKWebView`) lives in this
//! crate; Chromium registers itself from `nd-cef` with [`register_engine`]. An engine builds one
//! [`EngineView`] per page and reports through the page's [`Sink`].

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::{Rc, Weak};

use nd_ui::events::Emit;

use crate::element::WebViewProps;
use crate::events::*;
use crate::handle::{Inner, WebViewOptions};
use crate::{Cookie, FindOptions, JsError, JsValue, UserScript};

/// Which engine renders a page.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Engine {
    /// The OS engine: WKWebView on macOS, WebView2 on Windows. Linux has no OS engine that
    /// embeds in a GPUI window on Wayland, so there it is Chromium ([`Engine::Cef`]).
    #[default]
    System,
    /// Chromium through CEF, off-screen and drawn in the window (`nd-cef`). On macOS an opt in
    /// (`[webview] engine = "cef"` and nd's `cef` feature); on Linux the only engine.
    Cef,
}

impl Engine {
    pub fn parse(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "system" | "webkit" | "wkwebview" => Some(Self::System),
            "cef" | "chromium" => Some(Self::Cef),
            _ => None,
        }
    }

    /// The engine that renders a page asked to use this one: [`Engine::System`] is Chromium on
    /// Linux.
    pub fn resolved(self) -> Self {
        if cfg!(target_os = "linux") { Self::Cef } else { self }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Cef => "cef",
        }
    }
}

thread_local! {
    static DEFAULT: Cell<Option<Engine>> = const { Cell::new(None) };
    static FACTORIES: RefCell<HashMap<Engine, Rc<dyn EngineFactory>>> = RefCell::default();
}

/// The engine pages use when their options name none: `ND_WEBVIEW_ENGINE` when set, else what
/// the app set with [`set_default_engine`] (nd sets it from `[webview] engine`), else
/// [`Engine::System`].
pub fn default_engine() -> Engine {
    if let Some(engine) = std::env::var("ND_WEBVIEW_ENGINE").ok().and_then(|v| Engine::parse(&v)) {
        return engine;
    }
    DEFAULT.with(Cell::get).unwrap_or_default()
}

pub fn set_default_engine(engine: Engine) {
    DEFAULT.with(|d| d.set(Some(engine)));
}

/// Makes `engine` available to pages. `nd-cef` registers [`Engine::Cef`].
pub fn register_engine(engine: Engine, factory: impl EngineFactory) {
    FACTORIES.with(|f| f.borrow_mut().insert(engine, Rc::new(factory)));
}

pub(crate) fn factory(engine: Engine) -> Option<Rc<dyn EngineFactory>> {
    crate::platform::register_builtin();
    FACTORIES.with(|f| f.borrow().get(&engine).cloned())
}

/// Builds pages for one engine.
pub trait EngineFactory: 'static {
    fn create(&self, page: Page) -> Result<Rc<dyn EngineView>, String>;
}

/// What a page's engine object does. All calls come on the main thread.
pub trait EngineView {
    /// The view the backend frames and places; macOS.
    #[cfg(target_os = "macos")]
    fn ns_view(&self) -> objc2::rc::Retained<objc2_app_kit::NSView>;

    /// The element that shows the page in a GPUI window, for engines that draw off-screen.
    /// `None`: this engine cannot draw there, and the node says so instead.
    #[cfg(any(not(target_os = "macos"), feature = "gpui"))]
    fn element(
        &self,
        _cx: &nd_backend_gpui::custom::CustomCx<'_>,
        _window: &mut nd_gpui::gpui::Window,
    ) -> Option<nd_gpui::gpui::AnyElement> {
        None
    }

    /// Props changed (`old` is `None` on mount).
    fn apply(&self, props: &WebViewProps, old: Option<&WebViewProps>);

    fn command(&self, command: Command);

    /// The page leaves the window (unmounted, parked) or comes back.
    fn set_attached(&self, _attached: bool) {}

    /// The handle and the element are both gone: free the page.
    fn close(&self);
}

/// Commands from [`crate::WebView`]'s methods. Engines answer the ones with callbacks exactly
/// once, on the main thread.
pub enum Command {
    Load(String),
    LoadHtml {
        html: String,
        base_url: Option<String>,
    },
    GoBack,
    GoForward,
    Reload,
    Stop,
    SetZoom(f64),
    SetUserAgent(Option<String>),
    OpenDevTools,
    CloseDevTools,
    ExecuteJavaScript {
        code: String,
        world: Option<String>,
        done: Box<dyn FnOnce(Result<JsValue, JsError>)>,
    },
    AddUserScript(UserScript),
    RemoveUserScript(String),
    ClearUserScripts(Option<String>),
    RegisterScriptMessage {
        name: String,
        world: Option<String>,
    },
    UnregisterScriptMessage {
        name: String,
        world: Option<String>,
    },
    GetCookies {
        url: Option<String>,
        done: Box<dyn FnOnce(Vec<Cookie>)>,
    },
    SetCookie(Cookie),
    DeleteCookie {
        name: String,
        domain: Option<String>,
        path: Option<String>,
    },
    Find {
        text: String,
        options: FindOptions,
    },
    FindNext {
        backwards: bool,
    },
    StopFind,
    SaveSession(Box<dyn FnOnce(Option<Vec<u8>>)>),
    RestoreSession(Vec<u8>),
    SetMuted(bool),
    ExitFullscreen,
    StartDownload(String),
    Focus,
    /// Runs the page's `beforeunload`; [`CloseApproved`] answers whether it may close.
    RequestClose,
    /// Forgets the permission decisions the engine stored, for one origin or all.
    ResetPermissions {
        origin: Option<String>,
    },
    /// Lets `origin` (or every origin) open popups without a user gesture.
    AllowPopups {
        origin: Option<String>,
    },
    /// Delivers the key event nd is dispatching (Tab, Shift-Tab, which nd binds to focus
    /// traversal) to the page instead, so it moves focus inside the page.
    ForwardKey,
}

impl std::fmt::Debug for Command {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Command::Load(_) => "Load",
            Command::LoadHtml { .. } => "LoadHtml",
            Command::GoBack => "GoBack",
            Command::GoForward => "GoForward",
            Command::Reload => "Reload",
            Command::Stop => "Stop",
            Command::SetZoom(_) => "SetZoom",
            Command::SetUserAgent(_) => "SetUserAgent",
            Command::OpenDevTools => "OpenDevTools",
            Command::CloseDevTools => "CloseDevTools",
            Command::ExecuteJavaScript { .. } => "ExecuteJavaScript",
            Command::AddUserScript(_) => "AddUserScript",
            Command::RemoveUserScript(_) => "RemoveUserScript",
            Command::ClearUserScripts(_) => "ClearUserScripts",
            Command::RegisterScriptMessage { .. } => "RegisterScriptMessage",
            Command::UnregisterScriptMessage { .. } => "UnregisterScriptMessage",
            Command::GetCookies { .. } => "GetCookies",
            Command::SetCookie(_) => "SetCookie",
            Command::DeleteCookie { .. } => "DeleteCookie",
            Command::Find { .. } => "Find",
            Command::FindNext { .. } => "FindNext",
            Command::StopFind => "StopFind",
            Command::SaveSession(_) => "SaveSession",
            Command::RestoreSession(_) => "RestoreSession",
            Command::SetMuted(_) => "SetMuted",
            Command::ExitFullscreen => "ExitFullscreen",
            Command::StartDownload(_) => "StartDownload",
            Command::Focus => "Focus",
            Command::ForwardKey => "ForwardKey",
            Command::RequestClose => "RequestClose",
            Command::ResetPermissions { .. } => "ResetPermissions",
            Command::AllowPopups { .. } => "AllowPopups",
        };
        f.write_str(name)
    }
}

/// Navigation state as the engine last reported it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PageState {
    pub url: String,
    pub title: String,
    pub loading: bool,
    pub can_go_back: bool,
    pub can_go_forward: bool,
    pub progress: f64,
    pub focused: bool,
    pub zoom: f64,
}

/// Where a mounted page's events go.
pub(crate) struct Target {
    pub emit: Emit,
    pub wake: Rc<dyn Fn()>,
}

/// One page as its engine sees it: its options, and where it reports. Cheap to clone; engines
/// keep one and call it from their callbacks.
#[derive(Clone)]
pub struct Page {
    pub(crate) inner: Weak<Inner>,
    pub(crate) shared: Rc<PageShared>,
}

pub(crate) struct PageShared {
    pub options: WebViewOptions,
    pub state: RefCell<PageState>,
    pub target: RefCell<Option<Target>>,
    pub props: RefCell<Option<WebViewProps>>,
}

impl Page {
    pub fn options(&self) -> &WebViewOptions {
        &self.shared.options
    }

    pub fn state(&self) -> PageState {
        self.shared.state.borrow().clone()
    }

    /// The props of the element showing the page, `None` while it is parked.
    pub fn props(&self) -> Option<WebViewProps> {
        self.shared.props.borrow().clone()
    }

    pub fn is_attached(&self) -> bool {
        self.shared.target.borrow().is_some()
    }

    /// Queues `event` for the element showing the page and wakes the app. Dropped while the page
    /// is parked.
    pub fn emit<E: 'static>(&self, event: E) {
        let target = self.shared.target.borrow();
        if let Some(t) = target.as_ref() {
            t.emit.emit(event);
            (t.wake)();
        }
    }

    fn update<T: PartialEq + Clone>(&self, get: impl Fn(&mut PageState) -> &mut T, value: T) -> bool {
        let mut state = self.shared.state.borrow_mut();
        let slot = get(&mut state);
        if *slot == value {
            return false;
        }
        *slot = value;
        true
    }

    /// Each of these records the value and emits its event only when it changed, so an engine
    /// can report as often as it likes.
    pub fn report_url(&self, url: &str) {
        if !url.is_empty() && self.update(|s| &mut s.url, url.to_owned()) {
            self.emit(Navigate { url: url.to_owned() });
        }
    }

    pub fn report_title(&self, title: &str) {
        if self.update(|s| &mut s.title, title.to_owned()) {
            self.emit(TitleChanged { title: title.to_owned() });
        }
    }

    pub fn report_loading(&self, loading: bool) {
        if self.update(|s| &mut s.loading, loading) {
            self.emit(LoadingChanged { loading });
        }
    }

    pub fn report_back(&self, available: bool) {
        if self.update(|s| &mut s.can_go_back, available) {
            self.emit(BackAvailable { available });
        }
    }

    pub fn report_forward(&self, available: bool) {
        if self.update(|s| &mut s.can_go_forward, available) {
            self.emit(ForwardAvailable { available });
        }
    }

    pub fn report_progress(&self, progress: f64) {
        let progress = (progress.clamp(0., 1.) * 1000.).round() / 1000.;
        if self.update(|s| &mut s.progress, progress) {
            self.emit(LoadProgress { progress });
        }
    }

    pub fn report_focus(&self, focused: bool) {
        if self.update(|s| &mut s.focused, focused) {
            self.emit(FocusChanged { focused });
        }
    }

    pub fn report_zoom(&self, factor: f64, source: ZoomSource) {
        if self.update(|s| &mut s.zoom, factor) {
            self.emit(ZoomChanged { factor, source });
        }
    }

    /// The live handle of this page, when the app still holds one.
    pub fn handle(&self) -> Option<crate::WebView> {
        self.inner.upgrade().map(crate::WebView)
    }

    /// Runs `f` with the app, between frames: how answers to [`Command`] callbacks reach app
    /// code.
    pub fn defer(f: impl FnOnce(&mut nd_ui::App) + 'static) {
        crate::platform::defer(Box::new(f));
    }

    /// A popup page for [`NewWindow::popup`], sharing this page's options. The engine builds its
    /// view now (the opener needs a live window object) and the element that mounts the handle
    /// takes it over.
    pub fn popup(&self, view: impl FnOnce(Page) -> Rc<dyn EngineView>) -> crate::WebView {
        let options = self.shared.options.clone();
        let handle = crate::WebView::with_options(options);
        let page = handle.page();
        let engine = view(page);
        handle.0.adopt(engine);
        handle.close_unless_mounted(std::time::Duration::from_secs(10));
        handle
    }

    /// The download directory a browser would offer: `~/Downloads`.
    pub fn downloads_dir() -> PathBuf {
        std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join("Downloads")
    }
}
