//! `BROWSER_SCENE=extensions`: a password manager's fill, as early after launch as an app can
//! offer it. The test extension (`extension/`, loaded unpacked on the profile's first run) has a
//! popup that asks its service worker, which asks a native messaging host (`nd-test-host`),
//! which answers a secret its content script fills into the page: the path 1Password takes.
//!
//! The action runs the moment the extension list arrives, while the page is still loading. The
//! popup must open under the toolbar button, the fill must land, the content script must have
//! run in the page's first document, the popup must close itself, and the action's badge must
//! read back. Prints `ND_EXTENSIONS_OK` with the timings, or `ND_EXTENSIONS_FAILED` and why,
//! and exits. `ND_CEF_CACHE` names the profile, so a gate can run it on one profile many times.
//!
//! On Linux nd's Chrome-style pages have no toolbar for an action to run in, so the page asks
//! its content script for the fill and the popup checks are skipped.

use std::path::PathBuf;
use std::time::Instant;

use nd::prelude::*;
use nd::webview::extensions::{self, ActionOutcome, Anchor};
use nd::webview::{
    CefStyle, ExtensionPopupClosed, ExtensionPopupOpened, TitleChanged, WebView, WebViewOptions, webview,
};

/// The test extension's id, fixed by the key in its manifest.
pub const ID: &str = "bgbfijgfnneimkgfdaggpdpfmidadeen";
const HOST: &str = "dev.formalsnake.nd_test_host";
const SECRET: &str = "nd-secret-42";
/// Whether the scene clicks the action: see the module docs.
const CLICKS: bool = cfg!(target_os = "macos");
/// Where the toolbar button sits, in the window's content.
const BUTTON: Anchor = Anchor { x: 16., y: 16., width: 28., height: 28. };

struct Scene {
    page: WebView,
    start: Instant,
    clicked_ms: Option<u128>,
    popup: Option<(u128, Anchor)>,
    filled_ms: Option<u128>,
    closed: bool,
    checking: bool,
}

fn fail(why: impl std::fmt::Display) -> ! {
    eprintln!("ND_EXTENSIONS_FAILED {why}");
    std::process::exit(1);
}

fn extension_dir() -> PathBuf {
    std::env::var_os("ND_TEST_EXTENSION")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("extension"))
}

/// The host's manifest, in the profile's `NativeMessagingHosts`, pointing at `nd-test-host` next
/// to this executable.
fn install_host() {
    let Some(root) = std::env::var_os("ND_CEF_CACHE").map(PathBuf::from) else {
        fail("ND_CEF_CACHE must name the profile directory");
    };
    let host = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("nd-test-host")));
    let Some(host) = host.filter(|h| h.exists()) else { fail("nd-test-host is not built next to browser") };
    let dir = root.join("NativeMessagingHosts");
    let manifest = serde_json::json!({
        "name": HOST,
        "description": "nd test host",
        "path": host,
        "type": "stdio",
        "allowed_origins": [format!("chrome-extension://{ID}/")],
    });
    if let Err(e) = std::fs::create_dir_all(&dir)
        .and_then(|()| std::fs::write(dir.join(format!("{HOST}.json")), manifest.to_string()))
    {
        fail(format!("writing the host manifest: {e}"));
    }
}

impl Scene {
    fn ms(&self) -> u128 {
        self.start.elapsed().as_millis()
    }

    fn listed(&mut self, list: Result<Vec<extensions::Extension>, String>, loaded: bool, cx: &mut Context<Self>) {
        let list = list.unwrap_or_else(|e| fail(format!("list: {e}")));
        match list.iter().find(|e| e.id == ID) {
            Some(e) if !e.has_action => fail("the test extension reports no action"),
            Some(e) if e.icon.is_none() => fail("the test extension's icon did not come through"),
            // Chrome injects declared content scripts into documents that start after the
            // install, so the page that was open during it loads again.
            Some(_) if loaded => {
                self.page.reload();
                self.click();
            }
            Some(_) => self.click(),
            None if loaded => fail(format!("loaded, but not listed: {list:?}")),
            None => {
                eprintln!("ND_EXTENSIONS loading the test extension at {} ms", self.ms());
                let this = cx.weak_entity();
                extensions::load_unpacked(extension_dir(), move |r, _cx| {
                    if let Err(e) = r {
                        fail(format!("load_unpacked: {e}"));
                    }
                    extensions::list(move |list, cx| {
                        let _ = this.update(cx, |this, cx| this.listed(list, true, cx));
                    });
                });
            }
        }
    }

    fn click(&mut self) {
        self.clicked_ms = Some(self.ms());
        eprintln!("ND_EXTENSIONS click at {} ms", self.ms());
        if !CLICKS {
            self.page.execute_javascript("postMessage('nd-fill', '*'), true", |r, _cx| {
                if let Err(e) = r {
                    fail(format!("asking the page for a fill: {e}"));
                }
            });
            return;
        }
        self.page.run_extension_action(ID, BUTTON, |outcome, _cx| match outcome {
            Ok(ActionOutcome::PopupOpened) => {}
            other => fail(format!("the click did {other:?}, not open the popup")),
        });
    }

    fn popup_opened(&mut self, e: &ExtensionPopupOpened, cx: &mut Context<Self>) {
        let b = e.bounds;
        eprintln!("ND_EXTENSIONS popup at {b:?}, {} ms", self.ms());
        let under = (b.x - BUTTON.x).abs() <= 2. && (b.y - (BUTTON.y + BUTTON.height)).abs() <= 2.;
        if e.id != ID || !under {
            fail(format!("popup {} at {b:?}, not under the button {BUTTON:?}", e.id));
        }
        self.popup = Some((self.ms(), b));
        self.check(cx);
    }

    fn title(&mut self, title: &str, cx: &mut Context<Self>) {
        if title == format!("filled:{SECRET}") && self.filled_ms.is_none() {
            self.filled_ms = Some(self.ms());
            eprintln!("ND_EXTENSIONS filled at {} ms", self.ms());
            self.check(cx);
        }
    }

    fn check(&mut self, cx: &mut Context<Self>) {
        let popup_done = !CLICKS || (self.popup.is_some() && self.closed);
        if !popup_done || self.filled_ms.is_none() || self.checking {
            return;
        }
        self.checking = true;
        let this = cx.weak_entity();
        self.page.execute_javascript("window.ndFirstDocument === true", move |first, cx| {
            if first != Ok(serde_json::Value::Bool(true)) {
                fail(format!("the content script missed the page's first document: {first:?}"));
            }
            let _ = this.update(cx, |this, _cx| {
                let report = (this.clicked_ms, this.popup.map(|p| p.0), this.filled_ms);
                eprintln!("ND_EXTENSIONS reading the action state");
                this.page.extension_action(ID, move |state, _cx| {
                    let state = state.unwrap_or_else(|e| fail(format!("action state: {e}")));
                    if state.badge != "nd" || !state.has_popup || !state.enabled {
                        fail(format!("action state {state:?}"));
                    }
                    let (click, popup, filled) = report;
                    eprintln!(
                        "ND_EXTENSIONS_OK click_ms={} popup_ms={} fill_ms={}",
                        click.unwrap_or_default(),
                        popup.unwrap_or_default(),
                        filled.unwrap_or_default()
                    );
                    std::process::exit(0);
                });
            });
        });
    }
}

impl Render for Scene {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .child(h_flex().p(px(BUTTON.x as f32)).child(
                div().id("extension-button").w(px(BUTTON.width as f32)).h(px(BUTTON.height as f32)).bg(rgb(0x2f8f5b)),
            ))
            .child(
                webview("web", &self.page)
                    .flex_1()
                    .on_title_changed(cx.listener(|this, e: &TitleChanged, _, cx| this.title(&e.title, cx)))
                    .on_extension_popup_opened(
                        cx.listener(|this, e: &ExtensionPopupOpened, _, cx| this.popup_opened(e, cx)),
                    )
                    .on_extension_popup_closed(cx.listener(|this, e: &ExtensionPopupClosed, _, cx| {
                        if e.id == ID {
                            eprintln!("ND_EXTENSIONS popup closed at {} ms", this.ms());
                            this.closed = true;
                            this.check(cx);
                        }
                    })),
            )
    }
}

pub fn open(cx: &mut App) {
    let start = Instant::now();
    std::thread::spawn(|| {
        // A fresh profile's first launch is slow: Chromium sets the profile up first.
        std::thread::sleep(std::time::Duration::from_secs(120));
        fail("timed out");
    });
    install_host();
    let base = crate::server::start();
    let page = WebView::with_options(WebViewOptions::default().cef_style(CefStyle::Chrome));
    page.load(format!("{base}/login"));
    let options = WindowOptions::main("main").title("Extensions").size(size(px(900.), px(640.)));
    cx.open_window(options, move |_window, cx| {
        cx.new(move |cx: &mut Context<Scene>| {
            let this = cx.weak_entity();
            extensions::list(move |list, cx| {
                let _ = this.update(cx, |this, cx| this.listed(list, false, cx));
            });
            Scene { page, start, clicked_ms: None, popup: None, filled_ms: None, closed: false, checking: false }
        })
    });
}
