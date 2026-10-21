//! Chrome extensions, for pages on Chromium in Chrome style ([`crate::CefStyle::Chrome`]).
//! Chromium runs the extensions itself (service workers, content scripts, native messaging,
//! the Web Store's install flow); what Chrome would draw in its toolbar is the app's to draw:
//!
//! ```ignore
//! nd::webview::extensions::list(|list, cx| { /* one button per extension with an action */ });
//! nd::webview::extensions::on_changed(|change, cx| { /* installed, removed, enabled */ });
//! // a toolbar button's click, with the button's bounds in the window:
//! page.run_extension_action(id, bounds, |outcome, cx| {});
//! ```
//!
//! [`WebView::run_extension_action`](crate::WebView::run_extension_action) is a click on the
//! extension's toolbar button in Chrome: Chromium opens the extension's own popup (anchored
//! under the rectangle the app gives) or fires `chrome.action.onClicked` with `activeTab`
//! granted, whichever the extension's state for that page says at that moment.
//!
//! Extensions belong to the default profile; other profiles see none. Every call answers once,
//! on the main thread, between frames. Before Chromium has started (no Chrome-style page yet)
//! the calls wait for it, and the list is answered only once Chromium has loaded the installed
//! extensions, so an app never draws a button Chromium cannot run yet.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use nd_ui::App;

use crate::engine::Page;

/// One installed extension.
#[derive(Clone, Debug, PartialEq)]
pub struct Extension {
    pub id: String,
    /// In the user's language.
    pub name: String,
    pub version: String,
    pub enabled: bool,
    /// The extension's largest icon as PNG (or whatever image format it ships), when it has one.
    pub icon: Option<Rc<[u8]>>,
    /// Its options page, for a "Options" item next to it.
    pub options_url: Option<String>,
    /// Whether it has a toolbar action (`action`, `browser_action` or `page_action`).
    pub has_action: bool,
    pub source: Source,
}

/// Where an extension came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// The Chrome Web Store.
    Store,
    /// A directory or package the app loaded ([`load_unpacked`], [`install_package`]).
    Unpacked,
    /// Policy, the OS, or something else Chromium installed by itself.
    Other,
}

/// What [`on_changed`] hears.
#[derive(Clone, Debug, PartialEq)]
pub enum Change {
    Installed(String),
    Uninstalled(String),
    /// Enabled, disabled, updated or reloaded: read [`list`] again for the new state.
    Updated(String),
}

/// An extension's toolbar button for one page, as Chrome would draw it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ActionState {
    pub title: String,
    pub badge: String,
    /// RGBA, when the extension set one.
    pub badge_color: Option<[u8; 4]>,
    /// Whether a click opens a popup (`false`: it fires `onClicked`).
    pub has_popup: bool,
    /// Disabled for this page (`chrome.action.disable`, a page action that does not apply).
    pub enabled: bool,
}

/// What a click on an extension's action did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionOutcome {
    /// Its popup opened.
    PopupOpened,
    /// Its popup was open on this page and closed, as a second click on Chrome's button does.
    PopupClosed,
    /// It has no popup for this page: Chromium fired `chrome.action.onClicked`.
    Clicked,
}

/// A rectangle in the page's window, in logical pixels from the content's top left.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Anchor {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Anchor {
    pub fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self { x, y, width, height }
    }
}

type Reply<T> = Box<dyn FnOnce(Result<T, String>)>;

/// The profile-wide half of an extension runtime; `nd-cef` registers one. The per-page half
/// (actions) goes through the page's [`crate::Command`]s.
pub trait ExtensionEngine: 'static {
    fn list(&self, done: Reply<Vec<Extension>>);
    fn load_unpacked(&self, path: PathBuf, done: Reply<String>);
    fn install_package(&self, path: PathBuf, done: Reply<String>);
    fn uninstall(&self, id: String, done: Reply<()>);
    fn set_enabled(&self, id: String, enabled: bool, done: Reply<()>);
}

type Listener = Rc<dyn Fn(&Change, &mut App)>;

thread_local! {
    static ENGINE: RefCell<Option<Rc<dyn ExtensionEngine>>> = const { RefCell::new(None) };
    static LISTENERS: RefCell<Vec<Listener>> = const { RefCell::new(Vec::new()) };
}

pub fn register_extension_engine(engine: impl ExtensionEngine) {
    ENGINE.with(|e| *e.borrow_mut() = Some(Rc::new(engine)));
}

const NO_ENGINE: &str =
    "extensions need Chromium in Chrome style: nd's `cef` feature and `[webview.cef] style = \"chrome\"`";

fn with_engine<T: 'static>(
    done: impl FnOnce(Result<T, String>, &mut App) + 'static,
    call: impl FnOnce(&dyn ExtensionEngine, Reply<T>),
) {
    let reply: Reply<T> = Box::new(move |result| Page::defer(move |cx| done(result, cx)));
    match ENGINE.with(|e| e.borrow().clone()) {
        Some(engine) => call(&*engine, reply),
        None => reply(Err(NO_ENGINE.into())),
    }
}

/// Every installed extension, enabled or not, in install order.
pub fn list(done: impl FnOnce(Result<Vec<Extension>, String>, &mut App) + 'static) {
    with_engine(done, |e, reply| e.list(reply));
}

/// Loads the unpacked extension in `path` (a directory with a `manifest.json`), as
/// `chrome://extensions` does in developer mode. It stays installed across launches. Answers
/// its id.
pub fn load_unpacked(path: impl Into<PathBuf>, done: impl FnOnce(Result<String, String>, &mut App) + 'static) {
    let path = path.into();
    with_engine(done, move |e, reply| e.load_unpacked(path, reply));
}

/// Installs a `.crx` package: unpacked into the app's data and loaded like
/// [`load_unpacked`], under the id its key gives (the one the Web Store would). Answers its id.
pub fn install_package(path: impl Into<PathBuf>, done: impl FnOnce(Result<String, String>, &mut App) + 'static) {
    let path = path.into();
    with_engine(done, move |e, reply| e.install_package(path, reply));
}

/// Removes an extension without Chrome's confirmation; ask the user first.
pub fn uninstall(id: impl Into<String>, done: impl FnOnce(Result<(), String>, &mut App) + 'static) {
    let id = id.into();
    with_engine(done, move |e, reply| e.uninstall(id, reply));
}

pub fn set_enabled(id: impl Into<String>, enabled: bool, done: impl FnOnce(Result<(), String>, &mut App) + 'static) {
    let id = id.into();
    with_engine(done, move |e, reply| e.set_enabled(id, enabled, reply));
}

/// Calls `f` for every change to the installed extensions, from any source (the Web Store,
/// `chrome://extensions`, the calls above) for the rest of the run.
pub fn on_changed(f: impl Fn(&Change, &mut App) + 'static) {
    LISTENERS.with(|l| l.borrow_mut().push(Rc::new(f)));
}

/// Engines report changes here.
pub fn report_change(change: Change) {
    Page::defer(move |cx| {
        let listeners = LISTENERS.with(|l| l.borrow().clone());
        for listener in listeners {
            listener(&change, cx);
        }
    });
}
