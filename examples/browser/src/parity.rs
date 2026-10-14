//! Every event and command of `nd::webview` against a local test page, on whichever engine the
//! run picked. Each step runs a command and waits for what the page must report; the run prints
//! `ND_PARITY <step> ok` per step and `ND_WEBVIEW_PARITY_OK engine=<name>` at the end, or
//! `ND_WEBVIEW_PARITY_FAILED` with the step and what was seen, and exits.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use nd::prelude::*;
use nd::webview::*;

use crate::server;

const SCHEME: &str = "ndtest";
const STEP_TIMEOUT: Duration = Duration::from_secs(15);

type Run = fn(&mut Driver, &mut Context<Driver>);
type Done = fn(&Driver) -> bool;

struct Step {
    name: &'static str,
    /// Engines the step applies to; empty is every engine.
    only: &'static [Engine],
    run: Run,
    done: Done,
}

struct Driver {
    page: WebView,
    base: String,
    log: Vec<String>,
    /// Where the current step's log starts.
    mark: usize,
    step: usize,
    steps: Vec<Step>,
    watchdog: Arc<Mutex<(String, Instant)>>,
    download: PathBuf,
}

impl Driver {
    fn since(&self) -> &[String] {
        &self.log[self.mark..]
    }

    fn saw(&self, entry: &str) -> bool {
        self.since().iter().any(|l| l == entry)
    }

    fn saw_prefix(&self, prefix: &str) -> bool {
        self.since().iter().any(|l| l.starts_with(prefix))
    }

    fn ever(&self, entry: &str) -> bool {
        self.log.iter().any(|l| l == entry)
    }

    fn push(&mut self, entry: String, cx: &mut Context<Self>) {
        eprintln!("ND_PARITY_LOG {entry}");
        self.log.push(entry);
        self.advance(cx);
    }

    fn advance(&mut self, cx: &mut Context<Self>) {
        while let Some(step) = self.steps.get(self.step) {
            if !(step.done)(self) {
                return;
            }
            eprintln!("ND_PARITY {} ok", step.name);
            self.step += 1;
            self.start(cx);
        }
    }

    /// Runs the next step that applies to this engine.
    fn start(&mut self, cx: &mut Context<Self>) {
        let engine = self.page.engine();
        while let Some(step) = self.steps.get(self.step) {
            if step.only.is_empty() || step.only.contains(&engine) {
                break;
            }
            eprintln!("ND_PARITY {} skipped on {}", step.name, engine.name());
            self.step += 1;
        }
        let Some(step) = self.steps.get(self.step) else {
            eprintln!("ND_WEBVIEW_PARITY_OK engine={}", engine.name());
            std::process::exit(0);
        };
        *self.watchdog.lock().expect("watchdog") = (step.name.to_owned(), Instant::now());
        self.mark = self.log.len();
        let run = step.run;
        run(self, cx);
    }

    fn js(&self, tag: &'static str, code: &str, world: Option<&str>, cx: &mut Context<Self>) {
        let this = cx.weak_entity();
        self.page.execute_javascript_in(code, world, move |result, cx| {
            let entry = match result {
                Ok(v) => format!("js {tag} ok {v}"),
                Err(e) => format!("js {tag} err {e}"),
            };
            let _ = this.update(cx, |d, cx| d.push(entry, cx));
        });
    }

    /// Reads the cookies for the page's origin until `name` is there (or gone, with `present`
    /// false); the cookie store applies writes asynchronously.
    fn cookies(&self, tag: &'static str, name: &'static str, present: bool, cx: &mut Context<Self>) {
        poll_cookies(self.page.clone(), self.base.clone(), cx.weak_entity(), tag, name, present, 50);
    }
}

fn poll_cookies(
    page: WebView,
    base: String,
    this: WeakEntity<Driver>,
    tag: &'static str,
    name: &'static str,
    present: bool,
    attempts: u32,
) {
    let again = page.clone();
    page.get_cookies(Some(&base.clone()), move |cookies, cx| {
        let names: Vec<String> = cookies.iter().map(|c| c.name.clone()).collect();
        let settled = names.iter().any(|n| n == name) == present;
        if settled || attempts == 0 {
            let _ = this.update(cx, |d, cx| d.push(format!("cookies {tag} [{}]", names.join(",")), cx));
        } else {
            poll_cookies(again, base, this, tag, name, present, attempts - 1);
        }
    });
}

fn steps() -> Vec<Step> {
    vec![
        Step {
            name: "load",
            only: &[],
            run: |d, _| d.page.load(format!("{}/", d.base)),
            done: |d| {
                d.saw("title Parity Index")
                    && d.saw(&format!("navigate {}/", d.base))
                    && d.saw("loading false")
                    && d.saw("progress 1")
            },
        },
        Step {
            name: "slow-load",
            only: &[],
            run: |d, _| d.page.load(format!("{}/slow", d.base)),
            done: |d| d.saw("loading true"),
        },
        Step { name: "stop", only: &[], run: |d, _| d.page.stop(), done: |d| d.saw("loading false") },
        Step { name: "favicon", only: &[], run: |_, _| {}, done: |d| d.ever(&format!("favicon {}/icon.png", d.base)) },
        Step { name: "security", only: &[], run: |_, _| {}, done: |d| d.ever("security secure=false") },
        Step {
            name: "js-value",
            only: &[],
            run: |d, cx| d.js("add", "1 + 2", None, cx),
            done: |d| d.saw("js add ok 3"),
        },
        Step {
            name: "js-object",
            only: &[],
            run: |d, cx| d.js("obj", "({a: [1, 'b', true], n: null})", None, cx),
            done: |d| {
                d.since().iter().filter_map(|l| l.strip_prefix("js obj ok ")).any(|v| {
                    serde_json::from_str::<JsValue>(v).ok() == Some(serde_json::json!({"a": [1, "b", true], "n": null}))
                })
            },
        },
        Step {
            name: "js-throw",
            only: &[],
            run: |d, cx| d.js("throw", "throw new Error('boom')", None, cx),
            done: |d| d.since().iter().any(|l| l.starts_with("js throw err") && l.contains("boom")),
        },
        Step {
            name: "js-world",
            only: &[],
            run: |d, cx| {
                d.js("iso-set", "window.__w = 7; window.__w", Some("parity"), cx);
                d.js("page-get", "typeof window.__w", None, cx);
            },
            done: |d| d.saw("js iso-set ok 7") && d.saw(r#"js page-get ok "undefined""#),
        },
        Step {
            name: "script-message",
            only: &[],
            run: |d, cx| {
                d.page.register_script_message("bridge", None);
                d.js("post", "window.webkit.messageHandlers.bridge.postMessage({a: 1}); 0", None, cx);
            },
            done: |d| d.saw(r#"script bridge {"a":1}"#),
        },
        Step {
            name: "find",
            only: &[],
            run: |d, _| d.page.find("needle", FindOptions::default()),
            done: |d| d.saw("find true"),
        },
        Step {
            name: "user-script",
            only: &[],
            run: |d, _| {
                d.page.add_user_script(UserScript::new("u", "window.__ndUser = 42;").at_start());
                d.page.reload();
            },
            done: |d| d.saw("loading true") && d.saw("loading false"),
        },
        Step {
            name: "user-script-ran",
            only: &[],
            run: |d, cx| d.js("user", "window.__ndUser", None, cx),
            done: |d| d.saw("js user ok 42"),
        },
        Step {
            name: "navigate",
            only: &[],
            run: |d, _| d.page.load(format!("{}/second", d.base)),
            done: |d| d.saw("title Second") && d.saw("back true") && d.saw("loading false"),
        },
        Step {
            name: "go-back",
            only: &[],
            run: |d, _| d.page.go_back(),
            done: |d| d.saw(&format!("navigate {}/", d.base)) && d.saw("forward true"),
        },
        Step {
            name: "go-forward",
            only: &[],
            run: |d, _| d.page.go_forward(),
            done: |d| d.saw(&format!("navigate {}/second", d.base)) && d.saw("forward false"),
        },
        Step { name: "zoom", only: &[], run: |d, _| d.page.set_zoom(1.5), done: |d| d.saw("zoom 1.5 app") },
        Step {
            name: "user-agent",
            only: &[],
            run: |d, _| {
                d.page.set_user_agent(Some("NdParity/1.0"));
                d.page.reload();
            },
            done: |d| d.saw("loading true") && d.saw("loading false"),
        },
        Step {
            name: "user-agent-seen",
            only: &[],
            run: |d, cx| d.js("ua", "navigator.userAgent", None, cx),
            done: |d| d.saw(r#"js ua ok "NdParity/1.0""#),
        },
        Step {
            name: "cookie-set",
            only: &[],
            run: |d, cx| {
                d.page.set_cookie(Cookie::new("ndc", "1", "127.0.0.1"));
                d.cookies("set", "ndc", true, cx);
            },
            done: |d| {
                d.saw_prefix("cookies set [")
                    && d.since().iter().any(|l| l.starts_with("cookies set") && l.contains("ndc"))
            },
        },
        Step {
            name: "cookie-seen-by-page",
            only: &[],
            run: |d, cx| d.js("cookie", "document.cookie", None, cx),
            done: |d| d.since().iter().any(|l| l.starts_with("js cookie ok") && l.contains("ndc=1")),
        },
        Step {
            name: "cookie-delete",
            only: &[],
            run: |d, cx| {
                d.page.delete_cookie("ndc", None, None);
                d.js("after-delete", "0", None, cx);
            },
            done: |d| d.saw("js after-delete ok 0"),
        },
        Step {
            name: "cookie-gone",
            only: &[],
            run: |d, cx| d.cookies("gone", "ndc", false, cx),
            done: |d| d.since().iter().any(|l| l.starts_with("cookies gone") && !l.contains("ndc")),
        },
        Step {
            name: "download",
            only: &[],
            run: |d, _| d.page.load(format!("{}/download", d.base)),
            done: |d| {
                d.saw("download-requested parity.txt")
                    && d.saw("download done")
                    && std::fs::read_to_string(&d.download).is_ok_and(|s| s == server::DOWNLOAD_BODY)
            },
        },
        Step {
            name: "scheme",
            only: &[],
            run: |d, _| d.page.load(format!("{SCHEME}://page")),
            // Chromium canonicalizes a standard scheme's URL with a trailing slash.
            done: |d| {
                (d.saw(&format!("scheme {SCHEME}://page GET")) || d.saw(&format!("scheme {SCHEME}://page/ GET")))
                    && d.saw("title Scheme Page")
            },
        },
        Step {
            name: "load-failed",
            only: &[],
            run: |d, _| d.page.load("http://127.0.0.1:1/"),
            done: |d| d.saw_prefix("failed http://127.0.0.1:1/"),
        },
        Step {
            name: "external-protocol",
            only: &[],
            run: |d, cx| d.js("mailto", "location.href = 'mailto:parity@example.com'; 0", None, cx),
            done: |d| d.saw("external mailto:parity@example.com"),
        },
        Step {
            name: "request-close",
            only: &[],
            run: |d, _| d.page.request_close(),
            done: |d| d.saw("close-approved true"),
        },
        Step {
            name: "reload-index",
            only: &[],
            run: |d, _| d.page.load(format!("{}/", d.base)),
            done: |d| d.saw("title Parity Index") && d.saw("loading false"),
        },
        Step {
            name: "new-window",
            only: &[],
            run: |d, cx| d.js("open", &format!("window.open('{}/second'); 0", d.base), None, cx),
            done: |d| d.saw(&format!("newwindow {}/second popup=true", d.base)),
        },
        Step { name: "focus", only: &[], run: |d, _| d.page.focus(), done: |d| d.ever("focus true") },
        Step {
            name: "muted",
            only: &[],
            run: |d, _| d.page.set_muted(true),
            done: |d| d.saw_prefix("audio") && d.since().iter().any(|l| l.contains("muted=true")),
        },
        Step {
            name: "session",
            only: &[Engine::System],
            run: |d, cx| {
                let this = cx.weak_entity();
                d.page.save_session(move |state, cx| {
                    let n = state.map_or(0, |s| s.len());
                    let _ = this.update(cx, |d, cx| d.push(format!("session {}", n > 0), cx));
                });
            },
            done: |d| d.saw("session true"),
        },
        Step {
            name: "devtools",
            only: &[],
            run: |d, cx| {
                d.page.open_dev_tools();
                d.page.close_dev_tools();
                d.js("alive", "1", None, cx);
            },
            done: |d| d.saw("js alive ok 1"),
        },
    ]
}

impl Render for Driver {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        webview("parity", &self.page)
            .size_full()
            .on_navigate(cx.listener(|d, e: &Navigate, _, cx| d.push(format!("navigate {}", e.url), cx)))
            .on_title_changed(cx.listener(|d, e: &TitleChanged, _, cx| d.push(format!("title {}", e.title), cx)))
            .on_loading_changed(
                cx.listener(|d, e: &LoadingChanged, _, cx| d.push(format!("loading {}", e.loading), cx)),
            )
            .on_load_progress(cx.listener(|d, e: &LoadProgress, _, cx| d.push(format!("progress {}", e.progress), cx)))
            .on_back_available(cx.listener(|d, e: &BackAvailable, _, cx| d.push(format!("back {}", e.available), cx)))
            .on_forward_available(
                cx.listener(|d, e: &ForwardAvailable, _, cx| d.push(format!("forward {}", e.available), cx)),
            )
            .on_focus_changed(cx.listener(|d, e: &FocusChanged, _, cx| d.push(format!("focus {}", e.focused), cx)))
            .on_load_failed(cx.listener(|d, e: &LoadFailed, _, cx| d.push(format!("failed {} {}", e.url, e.error), cx)))
            .on_new_window(cx.listener(|d, e: &NewWindow, _, cx| {
                d.push(format!("newwindow {} popup={}", e.url, e.popup.is_some()), cx)
            }))
            .on_download_requested(cx.listener(|d, e: &DownloadRequested, _, cx| {
                e.download.save_to(d.download.clone());
                d.push(format!("download-requested {}", e.download.suggested_filename), cx);
            }))
            .on_download_updated(cx.listener(|d, e: &DownloadUpdated, _, cx| {
                let state = match e.state {
                    DownloadState::Running => "running",
                    DownloadState::Done => "done",
                    DownloadState::Failed => "failed",
                    DownloadState::Cancelled => "cancelled",
                };
                d.push(format!("download {state}"), cx);
            }))
            .on_script_message(
                cx.listener(|d, e: &ScriptMessage, _, cx| d.push(format!("script {} {}", e.name, e.body), cx)),
            )
            .on_scheme_request(cx.listener(|d, e: &SchemeRequest, _, cx| {
                let html =
                    "<!doctype html><html><head><title>Scheme Page</title></head><body>from the app</body></html>";
                e.responder.respond(SchemeResponse::ok("text/html", html));
                d.push(format!("scheme {} {}", e.url, e.method), cx);
            }))
            .on_favicon_changed(
                cx.listener(|d, e: &FaviconChanged, _, cx| d.push(format!("favicon {}", e.icon_url), cx)),
            )
            .on_find_result(cx.listener(|d, e: &FindResult, _, cx| d.push(format!("find {}", e.match_found), cx)))
            .on_security_changed(
                cx.listener(|d, e: &SecurityChanged, _, cx| d.push(format!("security secure={}", e.secure), cx)),
            )
            .on_zoom_changed(cx.listener(|d, e: &ZoomChanged, _, cx| {
                let source = if e.source == ZoomSource::App { "app" } else { "page" };
                d.push(format!("zoom {} {source}", e.factor), cx)
            }))
            .on_audio_state_changed(cx.listener(|d, e: &AudioStateChanged, _, cx| {
                d.push(format!("audio playing={} muted={}", e.playing, e.muted), cx)
            }))
            .on_external_protocol(
                cx.listener(|d, e: &ExternalProtocol, _, cx| d.push(format!("external {}", e.url), cx)),
            )
            .on_close_approved(
                cx.listener(|d, e: &CloseApproved, _, cx| d.push(format!("close-approved {}", e.approved), cx)),
            )
            .on_cookies_changed(cx.listener(|d, _: &CookiesChanged, _, cx| d.push("cookies-changed".into(), cx)))
    }
}

pub fn open(cx: &mut App) {
    if let Err(e) = register_scheme(SCHEME) {
        eprintln!("ND_WEBVIEW_PARITY_FAILED register_scheme: {e}");
        std::process::exit(1);
    }
    let base = server::start();
    let watchdog = Arc::new(Mutex::new((String::from("start"), Instant::now())));
    let watched = watchdog.clone();
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_millis(250));
            let (step, since) = watched.lock().expect("watchdog").clone();
            if since.elapsed() > STEP_TIMEOUT {
                eprintln!("ND_WEBVIEW_PARITY_FAILED step={step} timed out");
                std::process::exit(1);
            }
        }
    });
    let download = std::env::temp_dir().join(format!("nd-parity-{}.txt", std::process::id()));
    let page = WebView::with_options(WebViewOptions::default().adopt_popups().profile(Profile::Private));
    eprintln!("ND_PARITY engine={}", page.engine().name());
    let options = WindowOptions::main("main").title("Parity").size(size(px(800.), px(600.)));
    cx.open_window(options, move |_window, cx| {
        cx.new(move |cx| {
            let mut driver =
                Driver { page, base, log: Vec::new(), mark: 0, step: 0, steps: steps(), watchdog, download };
            driver.start(cx);
            driver
        })
    });
}
