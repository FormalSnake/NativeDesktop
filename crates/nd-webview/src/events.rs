//! What a web view reports. Each event is its own type, heard with the matching `on_*` method of
//! [`crate::WebViewElement`]. Engines emit them on the node's queue; a page that is parked (its
//! element unmounted) keeps its [`crate::WebView`] state current, and the element that mounts it
//! next hears the navigation state again.

use crate::handle::WebView;
use crate::{Download, JsValue, PermissionResponder, SchemeResponder};

/// The address the view shows changed: a load committed, a fragment or `pushState` moved it, or
/// a load started (the provisional address).
#[derive(Clone, Debug, PartialEq)]
pub struct Navigate {
    pub url: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TitleChanged {
    pub title: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LoadingChanged {
    pub loading: bool,
}

/// The page took or lost the keyboard.
#[derive(Clone, Debug, PartialEq)]
pub struct FocusChanged {
    pub focused: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BackAvailable {
    pub available: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ForwardAvailable {
    pub available: bool,
}

/// Estimated load progress, 0 to 1, emitted on change.
#[derive(Clone, Debug, PartialEq)]
pub struct LoadProgress {
    pub progress: f64,
}

/// A navigation failed. `url` is the address that failed, not the page the view still shows.
/// Cancellations (a newer load superseding one in flight, a response turned into a download)
/// are not failures and are not reported.
#[derive(Clone, Debug, PartialEq)]
pub struct LoadFailed {
    pub url: String,
    pub error: String,
}

/// The page's content process ended (crash, out of memory, killed). The view shows nothing
/// until the app reloads it.
#[derive(Clone, Debug, PartialEq)]
pub struct RenderProcessGone {
    pub reason: String,
}

/// Where a browser would put a new page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Disposition {
    /// `target=_blank`, plain `window.open`.
    ForegroundTab,
    /// Cmd or middle click, "Open Link in New Tab".
    BackgroundTab,
    /// Shift click, "Open Link in New Window".
    Window,
    /// `window.open` with size or position features.
    Popup,
}

/// Size and position a `window.open` asked for, in CSS pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WindowFeatures {
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub width: Option<f64>,
    pub height: Option<f64>,
}

/// The page asked for a new window (`target=_blank`, `window.open`). Without
/// [`crate::WebViewOptions::adopt_popups`] the engine opens nothing and the app decides what a
/// new window means, usually a tab at `url`. With it, `popup` is a live view the page holds a
/// `window.opener` handle to: mount it (`webview(id, popup)`) in a tab or window within ten
/// seconds, or it is closed.
#[derive(Clone, Debug)]
pub struct NewWindow {
    pub url: String,
    pub disposition: Disposition,
    pub user_gesture: bool,
    pub features: Option<WindowFeatures>,
    pub popup: Option<WebView>,
}

/// A popup opened without a user gesture was refused. Chromium engine only; WebKit refuses
/// them without telling anyone.
#[derive(Clone, Debug, PartialEq)]
pub struct PopupBlocked {
    pub url: String,
    pub disposition: Disposition,
}

/// A browser command Chromium would serve with its own browser UI, which a Chrome-style page
/// (`[webview.cef] style = "chrome"`) never shows: its shortcuts and menus reach the app instead,
/// which opens its own tab, history or downloads. Chromium does nothing with it.
#[derive(Clone, Debug, PartialEq)]
pub struct ChromeCommand {
    pub command: BrowserCommand,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BrowserCommand {
    NewTab,
    NewWindow,
    NewPrivateWindow,
    ReopenClosedTab,
    CloseTab,
    CloseWindow,
    NextTab,
    PreviousTab,
    History,
    Downloads,
    Bookmarks,
    BookmarkPage,
    TabSearch,
    Settings,
    Extensions,
    ClearBrowsingData,
    SavePage,
    ViewSource,
    OpenFile,
    Find,
    FindNext,
    FindPrevious,
    FocusAddress,
    Fullscreen,
    Home,
    TaskManager,
    Print,
    Quit,
}

/// The page closed its own window (`window.close()` in a window a script opened). The app
/// closes the tab or window that shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct WindowClosed;

/// A response the engine does not render, or an attachment. The download waits for
/// [`Download::save_to`] or [`Download::cancel`]; dropping the event without either cancels it.
#[derive(Clone, Debug)]
pub struct DownloadRequested {
    pub download: Download,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DownloadState {
    Running,
    Done,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DownloadUpdated {
    pub id: u64,
    pub state: DownloadState,
    pub received: u64,
    /// `None` while the size is unknown.
    pub total: Option<u64>,
    pub path: Option<std::path::PathBuf>,
    pub paused: bool,
    pub error: Option<String>,
}

/// A page posted to a handler registered with [`WebView::register_script_message`]:
/// `window.webkit.messageHandlers.<name>.postMessage(body)` on both engines.
#[derive(Clone, Debug, PartialEq)]
pub struct ScriptMessage {
    pub name: String,
    /// The content world the handler was registered in; empty for the page's own.
    pub world: String,
    pub body: JsValue,
}

/// A request for a scheme registered with [`crate::register_scheme`]. Answer it through
/// `responder`; until then the page waits.
#[derive(Clone, Debug)]
pub struct SchemeRequest {
    pub url: String,
    pub scheme: String,
    pub method: String,
    pub responder: SchemeResponder,
}

/// The view's cookie store changed.
#[derive(Clone, Debug, PartialEq)]
pub struct CookiesChanged;

/// The page's icon. WebKit has no favicon API, so there `icon_url` is the page's best
/// `<link rel=icon>` (or `/favicon.ico`) and the app fetches the bytes; Chromium reports the
/// same URL.
#[derive(Clone, Debug, PartialEq)]
pub struct FaviconChanged {
    pub page_url: String,
    pub icon_url: String,
}

/// The outcome of [`WebView::find`]. `match_count` and `active_match` are Chromium's; WebKit
/// reports only whether something matched.
#[derive(Clone, Debug, PartialEq)]
pub struct FindResult {
    pub match_found: bool,
    pub match_count: Option<u32>,
    pub active_match: Option<u32>,
    /// False on an interim count update.
    pub done: bool,
}

/// TLS state of the shown page: on commit, on mixed content, on a certificate failure (which is
/// never accepted: the load fails and `error` says why).
#[derive(Clone, Debug, PartialEq)]
pub struct SecurityChanged {
    pub secure: bool,
    pub insecure_content: bool,
    pub url: String,
    pub error: Option<String>,
}

/// The link under the pointer, empty when it left every link.
#[derive(Clone, Debug, PartialEq)]
pub struct LinkHover {
    pub url: String,
}

/// What a right click hit, reported whether or not a menu opens.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ContextMenu {
    /// In the view's coordinates, points, top-left origin.
    pub x: f64,
    pub y: f64,
    pub link: Option<String>,
    pub image: Option<String>,
    pub selection: Option<String>,
    pub editable: bool,
}

/// One of the app's items in the context menu was chosen. For a checkbox or radio item
/// `checked` is the state the click implies; the menu itself never changes, the app answers
/// with new [`crate::WebViewElement::context_menu_items`].
#[derive(Clone, Debug, PartialEq)]
pub struct ContextMenuItemClicked {
    pub id: String,
    pub page_url: String,
    pub link_url: Option<String>,
    pub image_url: Option<String>,
    pub selection_text: Option<String>,
    pub editable: bool,
    pub checked: Option<bool>,
    pub was_checked: Option<bool>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AudioStateChanged {
    pub playing: bool,
    pub muted: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZoomSource {
    /// [`WebView::set_zoom`].
    App,
    /// A zoom chord or ctrl-wheel the engine served itself.
    Page,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ZoomChanged {
    pub factor: f64,
    pub source: ZoomSource,
}

/// The page entered or left element fullscreen. The page fills the view and nothing more;
/// taking the window fullscreen is the app's call.
#[derive(Clone, Debug, PartialEq)]
pub struct FullscreenChanged {
    pub fullscreen: bool,
}

/// The answer to [`WebView::request_close`]: false when the page's `beforeunload` kept it open.
#[derive(Clone, Debug, PartialEq)]
pub struct CloseApproved {
    pub approved: bool,
}

/// The page navigated to a scheme no engine renders (`mailto:`, `tel:`, an app's own). Nothing
/// opens; the app decides, usually with the OS's handler.
#[derive(Clone, Debug, PartialEq)]
pub struct ExternalProtocol {
    pub url: String,
}

/// The engine withdrew a [`PermissionRequest`] itself (navigation, close); answering it now is a
/// no-op.
#[derive(Clone, Debug, PartialEq)]
pub struct PermissionRequestDismissed {
    pub origin: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PermissionKind {
    Camera,
    Microphone,
    Geolocation,
    Notifications,
    Clipboard,
    Other,
}

/// A page asks for a permission. Answer through `responder`; an unanswered request leaves the
/// page waiting.
#[derive(Clone, Debug)]
pub struct PermissionRequest {
    pub origin: String,
    pub kinds: Vec<PermissionKind>,
    pub responder: PermissionResponder,
}
