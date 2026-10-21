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
//! The Chrome Web Store installs through Chromium's own flow, so a store install updates from the
//! store like Chrome's: the store page's "Add to Chrome" works in any Chrome-style page, and
//! [`install_from_store`] does the same by id. Chromium asks with its own prompt unless the app
//! takes the question over ([`on_store_install`]).
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

use crate::Once;
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
    fn install_from_store(&self, id: String, done: Reply<String>);
    /// The app answers store pages' installs from now on ([`on_store_install`]).
    fn watch_store(&self) {}
}

type Listener = Rc<dyn Fn(&Change, &mut App)>;
type StoreHandler = Rc<dyn Fn(StoreInstall, &mut App)>;

thread_local! {
    static ENGINE: RefCell<Option<Rc<dyn ExtensionEngine>>> = const { RefCell::new(None) };
    static LISTENERS: RefCell<Vec<Listener>> = const { RefCell::new(Vec::new()) };
    static STORE_HANDLER: RefCell<Option<StoreHandler>> = const { RefCell::new(None) };
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

/// Installs extension `id` from the Chrome Web Store, as the store page's "Add to Chrome" does:
/// Chromium downloads and verifies the package and keeps it updated from the store. The app has
/// asked the user already, so Chromium's own prompt is answered unseen (where the platform can;
/// elsewhere it shows). Answers the id once installed.
pub fn install_from_store(id: impl Into<String>, done: impl FnOnce(Result<String, String>, &mut App) + 'static) {
    let id = id.into();
    with_engine(done, move |e, reply| {
        if is_extension_id(&id) {
            e.install_from_store(id, reply);
        } else {
            reply(Err(format!("{id:?} is not an extension id")));
        }
    });
}

/// Whether `id` has the shape of an extension id: 32 letters from `a` to `p`.
pub fn is_extension_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| (b'a'..=b'p').contains(&b))
}

/// An install the Web Store page asked for (its "Add to Chrome"), held before Chromium prompts.
#[derive(Clone, Debug)]
pub struct StoreInstall {
    pub id: String,
    /// In the user's language, as the store names it.
    pub name: String,
    pub icon_url: Option<String>,
    /// Chrome's lines under "It can:" in its prompt, in its order ([`permission_lines`]).
    pub permissions: Vec<String>,
    pub responder: StoreResponder,
}

/// How a [`StoreInstall`] is answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreDecision {
    /// Install, with Chromium's prompt answered unseen.
    Install,
    /// The store hears that the user cancelled; Chromium never prompts.
    Cancel,
    /// Chromium asks with its own prompt. What a request the app drops unanswered gets.
    AskInChrome,
}

/// Answers a [`StoreInstall`]; dropped unanswered, Chromium asks with its own prompt.
#[derive(Clone)]
pub struct StoreResponder(pub Once<StoreDecision>);

impl std::fmt::Debug for StoreResponder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StoreResponder")
    }
}

impl StoreResponder {
    pub fn respond(&self, decision: StoreDecision) {
        self.0.answer(decision);
    }
}

/// Takes over the question Chromium asks before a Web Store install the store page starts: `f`
/// hears each one and answers it, usually from a dialog of the app's own. Replaces the handler
/// set before. Installs started with [`install_from_store`] are not asked about.
pub fn on_store_install(f: impl Fn(StoreInstall, &mut App) + 'static) {
    STORE_HANDLER.with(|h| *h.borrow_mut() = Some(Rc::new(f)));
    if let Some(engine) = ENGINE.with(|e| e.borrow().clone()) {
        engine.watch_store();
    }
}

/// Engines hand the app a store page's install here; `answer` runs once, on the main thread.
pub fn ask_store_install(
    id: String,
    name: String,
    icon_url: Option<String>,
    manifest: &str,
    answer: impl FnOnce(StoreDecision) + 'static,
) {
    let permissions = permission_lines(manifest);
    let responder = StoreResponder(Once::new(StoreDecision::AskInChrome, answer));
    Page::defer(move |cx| {
        let handler = STORE_HANDLER.with(|h| h.borrow().clone());
        let install = StoreInstall { id, name, icon_url, permissions, responder: responder.clone() };
        match handler {
            Some(handler) => handler(install, cx),
            None => responder.respond(StoreDecision::AskInChrome),
        }
    });
}

/// Chrome's wording for what an extension can do, for the permissions that carry a warning in
/// its install prompt; the rest (storage, alarms, ...) carry none there either.
const WARNINGS: &[(&str, &str)] = &[
    ("tabs", "Read your browsing history"),
    ("webNavigation", "Read your browsing history"),
    ("history", "Read and change your browsing history"),
    ("topSites", "Read a list of your most frequently visited websites"),
    ("bookmarks", "Read and change your bookmarks"),
    ("downloads", "Manage your downloads"),
    ("nativeMessaging", "Communicate with cooperating native applications"),
    ("notifications", "Display notifications"),
    ("clipboardRead", "Read data you copy and paste"),
    ("clipboardWrite", "Modify data you copy and paste"),
    ("privacy", "Change your privacy-related settings"),
    ("management", "Manage your apps, extensions, and themes"),
    ("declarativeNetRequest", "Block content on any page"),
    ("geolocation", "Detect your physical location"),
    ("contentSettings", "Change your settings that control websites' access to features"),
    ("proxy", "Read and change all your data on all websites"),
    ("debugger", "Access the page debugger backend"),
    ("desktopCapture", "Capture content of your screen"),
    ("tabCapture", "Read and change all your data on all websites"),
];

/// The lines under "It can:" in Chrome's install prompt for `manifest` (the manifest's JSON):
/// site access first, then the other warnings, each said once.
pub fn permission_lines(manifest: &str) -> Vec<String> {
    let Ok(manifest) = serde_json::from_str::<serde_json::Value>(manifest) else { return Vec::new() };
    let strings = |v: &serde_json::Value| -> Vec<String> {
        v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect()).unwrap_or_default()
    };
    let permissions = strings(&manifest["permissions"]);
    let mut patterns = strings(&manifest["host_permissions"]);
    patterns.extend(permissions.iter().filter(|p| p.contains("://") || *p == "<all_urls>").cloned());
    if let Some(scripts) = manifest["content_scripts"].as_array() {
        for script in scripts {
            patterns.extend(strings(&script["matches"]));
        }
    }
    let all_hosts = |p: &str| matches!(p, "<all_urls>" | "*://*/*" | "http://*/*" | "https://*/*" | "*://*/");
    let host = |p: &str| -> Option<String> {
        let (scheme, rest) = p.split_once("://")?;
        if !matches!(scheme, "*" | "http" | "https" | "ws" | "wss" | "ftp") {
            return None;
        }
        let (host, _) = rest.split_once('/')?;
        (!host.is_empty()).then(|| host.strip_prefix("*.").unwrap_or(host).to_owned())
    };
    let mut lines = Vec::new();
    if patterns.iter().any(|p| all_hosts(p)) {
        lines.push("Read and change all your data on all websites".to_owned());
    } else {
        let mut hosts: Vec<String> = Vec::new();
        for h in patterns.iter().filter_map(|p| host(p)) {
            if !hosts.contains(&h) {
                hosts.push(h);
            }
        }
        match hosts.len() {
            0 => {}
            1 => lines.push(format!("Read and change your data on {}", hosts[0])),
            2 | 3 => {
                let (last, rest) = hosts.split_last().expect("two or three hosts");
                lines.push(format!("Read and change your data on {} and {last}", rest.join(", ")));
            }
            _ => lines.push("Read and change your data on a number of websites".to_owned()),
        }
    }
    for p in &permissions {
        if let Some((_, line)) = WARNINGS.iter().find(|(name, _)| name == p)
            && !lines.iter().any(|l| l == line)
        {
            lines.push((*line).to_owned());
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_lines_follow_chrome_s_prompt() {
        let all =
            r#"{"permissions":["tabs","storage","nativeMessaging","webNavigation"],"host_permissions":["<all_urls>"]}"#;
        assert_eq!(
            permission_lines(all),
            [
                "Read and change all your data on all websites",
                "Read your browsing history",
                "Communicate with cooperating native applications",
            ]
        );
        let two = r#"{"content_scripts":[{"matches":["https://*.example.com/*","https://a.org/x"]}]}"#;
        assert_eq!(permission_lines(two), ["Read and change your data on example.com and a.org"]);
        let many = r#"{"host_permissions":["https://a/*","https://b/*","https://c/*","https://d/*"]}"#;
        assert_eq!(permission_lines(many), ["Read and change your data on a number of websites"]);
        assert!(permission_lines("{}").is_empty());
        assert!(permission_lines("not json").is_empty());
    }

    #[test]
    fn extension_ids() {
        assert!(is_extension_id("aeblfdkhhhdcdjpifhhbdiojplfjncoa"));
        assert!(!is_extension_id("aeblfdkhhhdcdjpifhhbdiojplfjncoz"));
        assert!(!is_extension_id("short"));
    }
}
