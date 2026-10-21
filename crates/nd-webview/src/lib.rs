//! Web pages inside a window (ARCHITECTURE.md 8): `webview(id, &page)` shows a [`WebView`] the
//! app owns, laid out, framed and focused like any other element, with overlays always above it.
//!
//! ```ignore
//! let page = WebView::with_url("https://example.com");
//! webview("tab-1", &page)
//!     .size_full()
//!     .on_navigate(|e, _, _| println!("{}", e.url))
//!     .on_new_window(|e, _, _| println!("open a tab at {}", e.url))
//! // later, from a toolbar button:
//! page.go_back();
//! page.execute_javascript("document.title", |title, _cx| println!("{title:?}"));
//! ```
//!
//! Engines: WKWebView on macOS (this crate), Chromium off-screen through `nd-cef`: on macOS when
//! the app opts in (`[webview] engine = "cef"`, nd's `cef` feature), on Linux always, since no OS
//! engine embeds in a GPUI window on Wayland. WebView2 (Windows) is not written yet: there a web
//! view shows a notice and every command reports that the engine is missing.

pub mod element;
pub mod engine;
pub mod events;
pub mod extensions;
pub mod filter;
mod handle;
pub mod menu;
mod platform;
pub mod script;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

pub use element::{WebViewElement, WebViewProps, WebViewWidget, webview};
pub use engine::{
    CefStyle, Command, Engine, EngineFactory, EngineView, Page, PageState, default_cef_style, default_engine,
    register_engine, set_default_cef_style, set_default_engine,
};
pub use events::*;
pub use filter::{
    FilterRequest, FilterResponse, RequestFilter, ResourceKind, Verdict, clear_request_filter, set_request_filter,
};
pub use handle::{Profile, WebView, WebViewOptions};
pub use menu::{ContextMenuMode, Contexts, Hit, MenuItem, MenuItemKind};
pub use script::{Injection, UserScript};

#[cfg(target_os = "macos")]
pub use platform::appkit;

/// A JavaScript value as JSON.
pub type JsValue = serde_json::Value;

/// A script threw, or the page could not run it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JsError(pub String);

impl std::fmt::Display for JsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for JsError {}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SameSite {
    #[default]
    None,
    Lax,
    Strict,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Cookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    pub secure: bool,
    pub http_only: bool,
    /// Seconds since the Unix epoch; `None` for a session cookie.
    pub expires: Option<f64>,
    pub same_site: SameSite,
}

impl Cookie {
    pub fn new(name: impl Into<String>, value: impl Into<String>, domain: impl Into<String>) -> Self {
        Self { name: name.into(), value: value.into(), domain: domain.into(), path: "/".into(), ..Self::default() }
    }

    /// Whether the cookie is sent to `host`.
    pub fn matches_host(&self, host: &str) -> bool {
        let domain = self.domain.strip_prefix('.').unwrap_or(&self.domain);
        host == domain || host.ends_with(&format!(".{domain}"))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FindOptions {
    pub case_sensitive: bool,
    pub wraps: bool,
    pub backwards: bool,
}

impl Default for FindOptions {
    fn default() -> Self {
        Self { case_sensitive: false, wraps: true, backwards: false }
    }
}

/// Actions on one download, implemented by its engine.
pub trait DownloadControl {
    /// Starts (or redirects) the transfer to `path`.
    fn save_to(&self, path: PathBuf);
    fn cancel(&self);
    fn pause(&self);
    fn resume(&self);
}

/// One download, from [`DownloadRequested`].
#[derive(Clone)]
pub struct Download {
    pub id: u64,
    pub url: String,
    pub suggested_filename: String,
    pub mime: Option<String>,
    /// Bytes, when the server said.
    pub total: Option<u64>,
    control: Rc<dyn DownloadControl>,
}

impl std::fmt::Debug for Download {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Download")
            .field("id", &self.id)
            .field("url", &self.url)
            .field("suggested_filename", &self.suggested_filename)
            .finish()
    }
}

impl Download {
    pub fn new(
        id: u64,
        url: String,
        suggested_filename: String,
        mime: Option<String>,
        total: Option<u64>,
        control: Rc<dyn DownloadControl>,
    ) -> Self {
        Self { id, url, suggested_filename, mime, total, control }
    }

    pub fn save_to(&self, path: impl Into<PathBuf>) {
        self.control.save_to(path.into());
    }

    /// Saves under the suggested name in `~/Downloads`, numbered when the name is taken.
    pub fn save_to_downloads(&self) -> PathBuf {
        let dir = Page::downloads_dir();
        let name = if self.suggested_filename.is_empty() { "download" } else { &self.suggested_filename };
        let mut path = dir.join(name);
        let (stem, ext) = match name.rsplit_once('.') {
            Some((s, e)) if !s.is_empty() => (s.to_owned(), format!(".{e}")),
            _ => (name.to_owned(), String::new()),
        };
        let mut n = 1;
        while path.exists() {
            path = dir.join(format!("{stem} ({n}){ext}"));
            n += 1;
        }
        self.save_to(path.clone());
        path
    }

    pub fn cancel(&self) {
        self.control.cancel();
    }

    pub fn pause(&self) {
        self.control.pause();
    }

    pub fn resume(&self) {
        self.control.resume();
    }
}

/// A one-shot answer an engine waits on. Answering twice, or after the engine gave up, is a
/// no-op. When every copy is dropped unanswered (the app had no handler, or the page was
/// parked) the engine gets the fallback given at creation, so nothing waits forever.
pub struct Once<T>(Rc<OnceInner<T>>);

type Answer<T> = Box<dyn FnOnce(T)>;

struct OnceInner<T> {
    answer: RefCell<Option<Answer<T>>>,
    fallback: RefCell<Option<T>>,
}

impl<T> Drop for OnceInner<T> {
    fn drop(&mut self) {
        if let (Some(answer), Some(fallback)) = (self.answer.get_mut().take(), self.fallback.get_mut().take()) {
            answer(fallback);
        }
    }
}

impl<T> Clone for Once<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T> Once<T> {
    pub fn new(fallback: T, answer: impl FnOnce(T) + 'static) -> Self {
        Self(Rc::new(OnceInner {
            answer: RefCell::new(Some(Box::new(answer))),
            fallback: RefCell::new(Some(fallback)),
        }))
    }

    pub fn answer(&self, value: T) {
        let answer = self.0.answer.borrow_mut().take();
        if let Some(answer) = answer {
            answer(value);
        }
    }

    pub fn is_answered(&self) -> bool {
        self.0.answer.borrow().is_none()
    }

    /// Drops the pending answer without calling it: the engine withdrew the request.
    pub fn withdraw(&self) {
        self.0.answer.borrow_mut().take();
    }
}

/// The body of a scheme response.
#[derive(Clone, Debug, PartialEq)]
pub struct SchemeResponse {
    pub status: u16,
    pub mime: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl SchemeResponse {
    pub fn ok(mime: impl Into<String>, body: impl Into<Vec<u8>>) -> Self {
        Self { status: 200, mime: mime.into(), headers: Vec::new(), body: body.into() }
    }

    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    pub fn status(mut self, status: u16) -> Self {
        self.status = status;
        self
    }
}

/// Answers a [`SchemeRequest`].
#[derive(Clone)]
pub struct SchemeResponder(pub Once<Result<SchemeResponse, String>>);

impl std::fmt::Debug for SchemeResponder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SchemeResponder")
    }
}

impl SchemeResponder {
    pub fn respond(&self, response: SchemeResponse) {
        self.0.answer(Ok(response));
    }

    pub fn fail(&self, error: impl Into<String>) {
        self.0.answer(Err(error.into()));
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PermissionDecision {
    Allow,
    Deny,
    /// Waved away (Esc, a click elsewhere, a navigation): nothing is recorded.
    Dismiss,
}

/// Answers a [`PermissionRequest`].
#[derive(Clone)]
pub struct PermissionResponder(pub Once<PermissionDecision>);

impl std::fmt::Debug for PermissionResponder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PermissionResponder")
    }
}

impl PermissionResponder {
    pub fn respond(&self, decision: PermissionDecision) {
        self.0.answer(decision);
    }
}

thread_local! {
    static SCHEMES: RefCell<(Vec<String>, bool)> = const { RefCell::new((Vec::new(), false)) };
}

/// Registers a URL scheme pages may load, answered through [`SchemeRequest`]. WebKit binds
/// schemes to a configuration that is frozen once a view exists, so this has to run before the
/// first page is created, on every engine.
pub fn register_scheme(scheme: &str) -> Result<(), String> {
    let valid = !scheme.is_empty()
        && scheme.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '+' | '-' | '.'));
    if !valid {
        return Err(format!("invalid scheme {scheme:?}: lowercase letters, digits, '+', '-' and '.'"));
    }
    SCHEMES.with(|s| {
        let mut s = s.borrow_mut();
        if s.1 {
            return Err("register_scheme must run before the first web view is created".into());
        }
        if s.0.iter().any(|x| x == scheme) {
            return Err(format!("{scheme} is already registered"));
        }
        s.0.push(scheme.to_owned());
        Ok(())
    })
}

/// The registered schemes; freezes the list. Engines call it when building their first page.
pub fn schemes() -> Vec<String> {
    SCHEMES.with(|s| {
        let mut s = s.borrow_mut();
        s.1 = true;
        s.0.clone()
    })
}

/// Turns a typed address into a URL: a scheme is kept, a host-like word becomes `https://`, and
/// anything else is searched with `search` (`{}` replaced by the encoded text).
pub fn resolve_address(text: &str, search: &str) -> String {
    let text = text.trim();
    if text.is_empty() {
        return "about:blank".into();
    }
    let has_scheme = text.split_once(':').is_some_and(|(s, rest)| {
        !s.is_empty()
            && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
            && (rest.starts_with("//") || matches!(s, "about" | "data" | "file" | "javascript" | "mailto" | "blob"))
    });
    if has_scheme {
        return text.to_owned();
    }
    let host = text.split(['/', '?', '#']).next().unwrap_or("");
    let looks_like_host = !text.contains(' ')
        && (host.contains('.') || host.starts_with("localhost") || host.parse::<std::net::IpAddr>().is_ok());
    if looks_like_host {
        let scheme = if host.starts_with("localhost") || host.starts_with("127.") { "http" } else { "https" };
        return format!("{scheme}://{text}");
    }
    let encoded: String = text
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            b' ' => "+".into(),
            _ => format!("%{b:02X}"),
        })
        .collect();
    search.replace("{}", &encoded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_resolve_like_a_browser() {
        let s = "https://duckduckgo.com/?q={}";
        assert_eq!(resolve_address("example.com", s), "https://example.com");
        assert_eq!(resolve_address("localhost:8080/x", s), "http://localhost:8080/x");
        assert_eq!(resolve_address("http://a.b", s), "http://a.b");
        assert_eq!(resolve_address("about:blank", s), "about:blank");
        assert_eq!(resolve_address("rust lang", s), "https://duckduckgo.com/?q=rust+lang");
        assert_eq!(resolve_address("a&b", s), "https://duckduckgo.com/?q=a%26b");
    }

    #[test]
    fn cookie_domains_cover_subdomains() {
        let c = Cookie::new("a", "1", ".example.com");
        assert!(c.matches_host("example.com"));
        assert!(c.matches_host("www.example.com"));
        assert!(!c.matches_host("badexample.com"));
    }

    #[test]
    fn schemes_validate() {
        assert!(register_scheme("Bad").is_err());
        assert!(register_scheme("app").is_ok());
        assert!(register_scheme("app").is_err());
        assert_eq!(schemes(), vec!["app".to_string()]);
        assert!(register_scheme("late").is_err());
    }
}
