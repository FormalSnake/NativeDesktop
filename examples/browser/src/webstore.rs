//! `BROWSER_SCENE=webstore`: Chrome Web Store installs on a fresh profile, over the network. One
//! extension installs by id, one through the store page's own "Add to Chrome" with the app
//! answering the install (its permission lines in hand), and one the app cancels, which must
//! never install. Both installs must list as store extensions. Prints `ND_WEBSTORE_OK` with the
//! timings, or `ND_WEBSTORE_FAILED` and why, and exits.

use std::time::Instant;

use nd::prelude::*;
use nd::webview::extensions::{self, Change, Source, StoreDecision, StoreInstall};
use nd::webview::{CefStyle, LoadingChanged, WebView, WebViewOptions, webview};

/// Dark Reader.
const BY_ID: &str = "eimadpbcbfnmbkopoojfekhnkhdbieeh";
/// JSON Formatter.
const BY_BUTTON: &str = "bcjindcccaagfpapjjmafapmmgkkhgoa";
/// uBlock Origin Lite.
const CANCELLED: &str = "ddkjiahejlhfcafbddmgiahcphecmpfh";

#[derive(Clone, Copy, Debug, PartialEq)]
enum Stage {
    ById,
    ByButton,
    Cancel,
    Done,
}

struct Scene {
    page: WebView,
    start: Instant,
    stage: Stage,
    by_id_ms: u128,
    by_button_ms: u128,
}

fn fail(why: impl std::fmt::Display) -> ! {
    eprintln!("ND_WEBSTORE_FAILED {why}");
    std::process::exit(1);
}

fn detail(id: &str) -> String {
    format!("https://chromewebstore.google.com/detail/{id}?hl=en")
}

/// Clicks the store page's "Add to Chrome" once it renders, or Google's consent page's "Reject
/// all" first. Runs at each load's end; the page does the waiting.
const CLICK: &str = r#"(() => {
  if (window.__ndClicking) return 'waiting';
  window.__ndClicking = true;
  const find = (re) => [...document.querySelectorAll('button, [role=button]')].find((b) => re.test(b.innerText || ''));
  const timer = setInterval(() => {
    const hit = location.host === 'consent.google.com' ? find(/reject all/i) : find(/add to chrome/i);
    if (hit && !hit.disabled) { clearInterval(timer); hit.scrollIntoView({ block: 'center' }); hit.click(); }
  }, 250);
  setTimeout(() => clearInterval(timer), 60000);
  return 'armed';
})()"#;

impl Scene {
    fn ms(&self) -> u128 {
        self.start.elapsed().as_millis()
    }

    fn installed(&mut self, id: &str, cx: &mut Context<Self>) {
        let this = cx.weak_entity();
        let id = id.to_owned();
        extensions::list(move |list, cx| {
            let list = list.unwrap_or_else(|e| fail(format!("list: {e}")));
            match list.iter().find(|e| e.id == id) {
                Some(e) if e.source == Source::Store && e.enabled => {}
                Some(e) => fail(format!("{id} installed as {:?}, enabled {}", e.source, e.enabled)),
                None => fail(format!("{id} installed but not listed")),
            }
            let _ = this.update(cx, |this, cx| this.next(cx));
        });
    }

    fn next(&mut self, cx: &mut Context<Self>) {
        match self.stage {
            Stage::ById => {
                self.by_id_ms = self.ms();
                eprintln!("ND_WEBSTORE installed by id at {} ms", self.by_id_ms);
                self.stage = Stage::ByButton;
                self.page.load(detail(BY_BUTTON));
            }
            Stage::ByButton => {
                self.by_button_ms = self.ms();
                eprintln!("ND_WEBSTORE installed from the store's button at {} ms", self.by_button_ms);
                self.stage = Stage::Cancel;
                self.page.load(detail(CANCELLED));
            }
            Stage::Cancel | Stage::Done => {}
        }
        cx.notify();
    }

    fn asked(&mut self, install: StoreInstall, cx: &mut Context<Self>) {
        eprintln!(
            "ND_WEBSTORE asked for {} ({:?}) at {} ms, it can: {:?}",
            install.id,
            install.name,
            self.ms(),
            install.permissions
        );
        match (self.stage, install.id.as_str()) {
            (Stage::ByButton, BY_BUTTON) => {
                if install.name.is_empty() {
                    fail("the store's request names no extension");
                }
                install.responder.respond(StoreDecision::Install);
            }
            (Stage::Cancel, CANCELLED) => {
                if install.permissions.is_empty() {
                    fail("no permission lines for an extension that reads every page");
                }
                install.responder.respond(StoreDecision::Cancel);
                self.stage = Stage::Done;
                let this = cx.weak_entity();
                // The held call ends in the page: the store hears the cancel, and Chromium never saw it.
                self.page.execute_javascript("window.__ndStoreAnswer('x', false)", move |r, _cx| {
                    if r != Ok(serde_json::Value::String("none".into())) {
                        fail(format!("the store page's hook: {r:?}"));
                    }
                    extensions::list(move |list, cx| {
                        let list = list.unwrap_or_else(|e| fail(format!("list: {e}")));
                        if list.iter().any(|e| e.id == CANCELLED) {
                            fail("the cancelled extension installed");
                        }
                        let _ = this.update(cx, |this, _cx| {
                            eprintln!(
                                "ND_WEBSTORE_OK by_id_ms={} by_button_ms={} cancel_ms={}",
                                this.by_id_ms,
                                this.by_button_ms,
                                this.ms()
                            );
                            std::process::exit(0);
                        });
                    });
                });
            }
            (stage, id) => fail(format!("asked for {id} during {stage:?}")),
        }
    }

    fn loaded(&mut self) {
        if matches!(self.stage, Stage::ByButton | Stage::Cancel) {
            self.page.execute_javascript(CLICK, |r, _cx| {
                if let Err(e) = r {
                    fail(format!("clicking the store's button: {e}"));
                }
            });
        }
    }
}

impl Render for Scene {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex().size_full().child(webview("web", &self.page).flex_1().on_loading_changed(cx.listener(
            |this, e: &LoadingChanged, _, _cx| {
                if !e.loading {
                    this.loaded();
                }
            },
        )))
    }
}

pub fn open(cx: &mut App) {
    let start = Instant::now();
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_secs(180));
        fail("timed out");
    });
    let page = WebView::with_options(WebViewOptions::default().cef_style(CefStyle::Chrome));
    page.load("https://chromewebstore.google.com/?hl=en");
    let options = WindowOptions::main("main").title("Web Store").size(size(px(1000.), px(760.)));
    cx.open_window(options, move |_window, cx| {
        cx.new(move |cx: &mut Context<Scene>| {
            let this = cx.weak_entity();
            extensions::on_store_install(move |install, cx| {
                let _ = this.update(cx, |this, cx| this.asked(install, cx));
            });
            let this = cx.weak_entity();
            extensions::on_changed(move |change, cx| {
                if let Change::Installed(id) = change {
                    let id = id.clone();
                    let _ = this.update(cx, |this, cx| {
                        if this.stage == Stage::ByButton && id == BY_BUTTON {
                            this.installed(&id, cx);
                        } else if id == CANCELLED {
                            fail("the cancelled extension installed");
                        }
                    });
                }
            });
            let this = cx.weak_entity();
            // The list starts the watch that hears the store button's install.
            extensions::list(move |list, _cx| {
                if let Err(e) = list {
                    fail(format!("list: {e}"));
                }
                eprintln!("ND_WEBSTORE installing {BY_ID} by id");
                extensions::install_from_store(BY_ID, move |r, cx| {
                    let id = r.unwrap_or_else(|e| fail(format!("install_from_store: {e}")));
                    let _ = this.update(cx, |this, cx| this.installed(&id, cx));
                });
            });
            Scene { page, start, stage: Stage::ById, by_id_ms: 0, by_button_ms: 0 }
        })
    });
}
