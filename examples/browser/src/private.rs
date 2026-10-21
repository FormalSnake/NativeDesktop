//! `BROWSER_SCENE=private`: a private window is one in-memory profile. Two tabs of one private
//! window (both shown, side by side) share a cookie one of them set; a normal window's page does
//! not see it; after the private window closes and its session is dropped, a new private window
//! starts empty. Prints `ND_PRIVATE_OK`, or `ND_PRIVATE_FAILED` and why, and exits.

use nd::core::WindowId;
use nd::prelude::*;
use nd::webview::{LoadingChanged, Page, PrivateSession, Profile, WebView, WebViewOptions, webview};

use crate::server;

const COOKIE: &str = "nd_private";

type Then = Box<dyn FnOnce(Vec<WebView>, &mut App)>;

fn fail(why: impl std::fmt::Display) -> ! {
    eprintln!("ND_PRIVATE_FAILED {why}");
    std::process::exit(1);
}

fn private_page(session: &PrivateSession, url: &str) -> WebView {
    let page = WebView::with_options(WebViewOptions::default().profile(Profile::Private(session.clone())));
    page.load(url);
    page
}

/// What `document.cookie` holds in `page`.
fn cookies(page: &WebView, done: impl FnOnce(String, &mut App) + 'static) {
    page.execute_javascript("document.cookie", move |result, cx| match result {
        Ok(serde_json::Value::String(c)) => done(c, cx),
        other => fail(format!("document.cookie: {other:?}")),
    });
}

/// One window's tabs, shown side by side; `then` runs once every one has loaded.
struct Tabs {
    pages: Vec<WebView>,
    loaded: Vec<bool>,
    then: Option<Then>,
}

impl Render for Tabs {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut row = h_flex().size_full();
        for (i, page) in self.pages.iter().enumerate() {
            let tab = webview(SharedString::from(format!("tab-{i}")), page).flex_1().h_full();
            row = row.child(tab.on_loading_changed(cx.listener(move |this, e: &LoadingChanged, _, _| {
                if e.loading || this.loaded[i] {
                    return;
                }
                this.loaded[i] = true;
                if this.loaded.iter().all(|l| *l)
                    && let Some(then) = this.then.take()
                {
                    let pages = this.pages.clone();
                    Page::defer(move |cx| then(pages, cx));
                }
            })));
        }
        row
    }
}

/// Opens a window showing `pages`; `then` runs once every one has loaded.
fn open_tabs(
    cx: &mut App,
    title: &'static str,
    pages: Vec<WebView>,
    then: impl FnOnce(Vec<WebView>, &mut App) + 'static,
) -> WindowId {
    let options = WindowOptions::main(title).title(title).size(size(px(900.), px(480.)));
    let loaded = vec![false; pages.len()];
    let then: Then = Box::new(then);
    cx.open_window(options, move |_window, cx| cx.new(move |_cx| Tabs { pages, loaded, then: Some(then) })).id()
}

pub fn open(cx: &mut App) {
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_secs(90));
        fail("no verdict within 90 s");
    });
    let url = format!("{}/second", server::start());
    let session = PrivateSession::new();
    let tabs = vec![private_page(&session, &url), private_page(&session, &url)];
    let window = std::rc::Rc::new(std::cell::Cell::new(None));
    let shown = window.clone();
    let id = open_tabs(cx, "Private", tabs, move |pages, _| {
        let set = format!("document.cookie = '{COOKIE}=1; path=/'");
        let (first, second) = (pages[0].clone(), pages[1].clone());
        first.execute_javascript(set, move |_, _| {
            cookies(&second, move |seen, cx| {
                if !seen.contains(COOKIE) {
                    fail(format!("the private window's second tab does not see the first's cookie: {seen:?}"));
                }
                eprintln!("ND_PRIVATE shared ok");
                normal(cx, url, pages, session, shown.get().expect("the private window"));
            });
        });
    });
    window.set(Some(id));
}

/// A normal window's page must not see the private cookie. Then the private window closes.
fn normal(cx: &mut App, url: String, private: Vec<WebView>, session: PrivateSession, window: WindowId) {
    let page = WebView::new();
    page.load(&url);
    open_tabs(cx, "Normal", vec![page], move |pages, _| {
        cookies(&pages[0], move |seen, cx| {
            if seen.contains(COOKIE) {
                fail(format!("a normal window sees the private cookie: {seen:?}"));
            }
            eprintln!("ND_PRIVATE isolated ok");
            cx.close_window(window);
            drop(private);
            drop(session);
            Page::defer(move |cx| reopened(cx, url));
        });
    });
}

/// A private window opened after the first closed starts with no cookies.
fn reopened(cx: &mut App, url: String) {
    let session = PrivateSession::new();
    let page = private_page(&session, &url);
    open_tabs(cx, "Private again", vec![page], move |pages, _| {
        cookies(&pages[0], move |seen, _| {
            drop(session);
            if seen.contains(COOKIE) {
                fail(format!("a new private window sees the closed one's cookie: {seen:?}"));
            }
            eprintln!("ND_PRIVATE_OK");
            std::process::exit(0);
        });
    });
}
