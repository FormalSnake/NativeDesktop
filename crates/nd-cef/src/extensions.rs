//! Chrome extensions (`nd_webview::extensions`) on Chromium's own extension system, driven
//! through the browser-wide DevTools session (`pipe.rs`). CEF has no extension API any more;
//! what Chrome's toolbar does is done the way the toolbar does it:
//!
//! - The list and its changes come from `chrome.developerPrivate` in a hidden
//!   `chrome://extensions` target, the page Chrome's own extension manager runs.
//! - An action click is `Extensions.triggerAction`, Chromium's `ExecuteUserAction` on the page's
//!   tab: the extension's real popup opens in the page's own Chrome window (so `activeTab`,
//!   `chrome.tabs.query({currentWindow: true})` and the popup's `window.close()` all mean that
//!   page), or `onClicked` fires, whichever the extension's live state decides at that moment.
//!   The platform moves the popup under the app's anchor (`platform::popup`).
//! - An action's badge and title are answered only to the extension itself, so they are read in
//!   a hidden target of its own `manifest.json`, which runs none of its code.
//!
//! Unpacked extensions: Chromium forgets one loaded through DevTools at its next start, so each
//! is also recorded (`nd-unpacked`, in the cache root) and passed back with `--load-extension`.
//! Chromium loads those while it builds the profile, before any page navigates, so a page's
//! first document gets their content scripts.
//!
//! Web Store installs go through `chrome.webstorePrivate` in a store page, the API the store's
//! own "Add to Chrome" calls: Chromium parses the manifest, prompts, then downloads, verifies and
//! installs the package as a store extension. An install by id does that in a hidden store page
//! with the manifest of the package the store serves. When the app answers store installs
//! itself, each store page's call is held in the page until the app has (`STORE_HOOK`).
//!
//! Startup: nothing here reads state ahead of a click and acts on it later. The list is the
//! extensions Chromium has loaded, a click goes to Chromium as a click, and a click that reaches
//! Chromium before the page's window has built its toolbar model waits for it
//! ([`TRIGGER_PATIENCE`]) instead of failing.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use base64::Engine as _;
use nd_webview::extensions::{
    ActionOutcome, ActionState, Anchor, Change, Extension, ExtensionEngine, Source, report_change,
};
use serde_json::{Value, json};

use crate::page::{self, CefPage};
use crate::pipe;

/// How long a click waits for Chromium to know the page's tab and the extension's action: at
/// launch the toolbar model of a page's window fills in after the extension system is ready.
const TRIGGER_PATIENCE: Duration = Duration::from_secs(15);
/// How long after Chromium ran a click its popup has to name itself before the click counts as
/// `onClicked`. Its URL arrives when it navigates, and CEF holds the first frame of any contents
/// that are not a CEF browser (a popup, a hidden target) for two seconds first
/// (`kNewBrowserInfoResponseTimeoutMs` in CEF's `browser_info_manager.cc`).
const POPUP_WAIT: Duration = Duration::from_millis(6000);
/// How long the platform looks for a click's popup window: the click's wait for Chromium and
/// the popup's own. A click that ends without a popup stops the search sooner.
#[cfg(target_os = "macos")]
pub(crate) const POPUP_SEARCH: Duration = TRIGGER_PATIENCE.saturating_add(POPUP_WAIT);
/// A press on the action's button that dismissed its popup (a click outside it) arrives this
/// soon after the close; it ends there, as a second click on Chrome's button does.
const RECLICK: Duration = Duration::from_millis(500);
/// The binding the hidden `chrome://extensions` page reports changes through.
const BINDING: &str = "__ndExtensions";
/// The Web Store's origin, the only one `chrome.webstorePrivate` is given to.
const STORE: &str = "https://chromewebstore.google.com/";
/// A store page `webstorePrivate` runs in that Google never sends through its cookie consent
/// page, which a fresh profile in the EU gets for every other store URL.
const STORE_QUIET: &str = "https://chromewebstore.google.com/robots.txt";
/// The binding a store page asks the app through.
const STORE_BINDING: &str = "__ndStoreAsk";
/// How long the platform watches for Chromium's install prompt after an accepted install.
pub(crate) const PROMPT_WAIT: Duration = Duration::from_secs(20);

type Then<T> = Box<dyn FnOnce(Result<T, String>)>;

#[derive(Default)]
struct State {
    /// The hidden `chrome://extensions` session, once attached and watching.
    registry: Option<String>,
    registry_waiting: Vec<Then<String>>,
    registry_opening: bool,
    /// Hidden `manifest.json` sessions by extension id.
    probes: HashMap<String, String>,
    /// Targets this module opened, which are never an extension's popup.
    own_targets: Vec<String>,
    discovering: bool,
    /// The popup a click opened: (extension id, page id, target id).
    popup: Option<Popup>,
    /// Clicks waiting for their popup's target to appear.
    awaiting_popup: Vec<AwaitingPopup>,
    /// The last popup's close: (extension id, page id, when).
    last_close: Option<(String, u64, Instant)>,
    /// Tab targets by the page target they hold.
    tabs: HashMap<String, String>,
    /// Targets auto-attached under a session, while a tab is searched.
    children: HashMap<String, Vec<String>>,
    /// Whether store pages' installs go to the app.
    watching_store: bool,
    /// Store page targets with the install hook, and their sessions.
    store_pages: HashMap<String, String>,
}

struct Popup {
    id: String,
    page: u64,
    target: String,
}

struct AwaitingPopup {
    id: String,
    page: u64,
    done: Option<Then<ActionOutcome>>,
}

thread_local! {
    static ST: RefCell<State> = RefCell::default();
    static LISTENING: Cell<bool> = const { Cell::new(false) };
}

pub(crate) struct Engine;

pub(crate) fn register() {
    nd_webview::extensions::register_extension_engine(Engine);
}

fn ensure_listening() {
    if LISTENING.with(|l| l.replace(true)) {
        return;
    }
    pipe::listen(|method, params, session| match method {
        "Runtime.bindingCalled" if params["name"].as_str() == Some(STORE_BINDING) => {
            if let Some(session) = session {
                store_asked(session, params["payload"].as_str().unwrap_or_default());
            }
        }
        "Runtime.bindingCalled" if params["name"].as_str() == Some(BINDING) => {
            let registry = ST.with(|s| s.borrow().registry.clone());
            if session.is_some() && session == registry.as_deref() {
                registry_event(params["payload"].as_str().unwrap_or_default());
            }
        }
        "Target.attachedToTarget" => {
            if let (Some(parent), Some(child)) = (session, params["targetInfo"]["targetId"].as_str()) {
                ST.with(|s| s.borrow_mut().children.entry(parent.to_owned()).or_default().push(child.to_owned()));
            }
        }
        "Target.targetCreated" | "Target.targetInfoChanged" => {
            target_created(&params["targetInfo"]);
            store_page(&params["targetInfo"]);
        }
        "Target.targetDestroyed" => {
            let target = params["targetId"].as_str().unwrap_or_default();
            ST.with(|s| {
                let mut s = s.borrow_mut();
                s.tabs.retain(|page, tab| page != target && tab != target);
                s.store_pages.remove(target);
            });
            target_destroyed(target);
        }
        "Target.detachedFromTarget" => {
            let session = params["sessionId"].as_str().unwrap_or_default();
            ST.with(|s| {
                let mut s = s.borrow_mut();
                if s.registry.as_deref() == Some(session) {
                    s.registry = None;
                }
                s.probes.retain(|_, v| v != session);
                s.store_pages.retain(|_, v| v != session);
            });
        }
        _ => {}
    });
}

/// Runs `f` once Chromium (and with it the pipe) is up: the first page starts it.
fn when_started(then: impl FnOnce() -> Result<(), String> + 'static, fail: impl FnOnce(String) + 'static) {
    if let Err(e) = crate::runtime::start() {
        fail(e);
        return;
    }
    if let Some(why) = pipe::unavailable() {
        fail(why);
        return;
    }
    ensure_listening();
    if let Err(e) = then() {
        fail(e);
    }
}

/// `Runtime.evaluate` in a flat session, the promise awaited, the value returned as JSON.
fn evaluate(session: &str, expression: &str, then: impl FnOnce(Result<Value, String>) + 'static) {
    let params = json!({ "expression": expression, "awaitPromise": true, "returnByValue": true });
    pipe::call("Runtime.evaluate", params, Some(session), move |r| {
        then(r.and_then(|v| {
            match v.get("exceptionDetails") {
                Some(e) => Err(e["exception"]["description"]
                    .as_str()
                    .or_else(|| e["text"].as_str())
                    .unwrap_or("the script threw")
                    .to_owned()),
                None => Ok(v["result"]["value"].clone()),
            }
        }));
    });
}

/// Opens `url` in a hidden target (no window, no tab strip) and attaches to it. Chromium makes
/// hidden targets only while some page exists, so this waits for the first page's browser.
fn open_hidden(url: &str, then: impl FnOnce(Result<(String, String), String>) + 'static) {
    if page::browser_count() == 0 {
        let url = url.to_owned();
        crate::runtime::later(Duration::from_millis(20), move || open_hidden(&url, then));
        return;
    }
    let params = json!({ "url": url, "hidden": true, "background": true });
    pipe::call("Target.createTarget", params, None, move |r| {
        let target = match r {
            Ok(v) => v["targetId"].as_str().unwrap_or_default().to_owned(),
            Err(e) => return then(Err(e)),
        };
        ST.with(|s| s.borrow_mut().own_targets.push(target.clone()));
        let params = json!({ "targetId": target, "flatten": true });
        pipe::call("Target.attachToTarget", params, None, move |r| match r {
            Ok(v) => {
                let session = v["sessionId"].as_str().unwrap_or_default().to_owned();
                pipe::call("Runtime.enable", json!({}), Some(&session.clone()), move |_| then(Ok((target, session))));
            }
            Err(e) => then(Err(e)),
        });
    });
}

/// Evaluates `ready` in `session` until it is true, every 50 ms for up to `patience`.
fn wait_until(
    session: String,
    ready: &'static str,
    patience: Duration,
    then: impl FnOnce(Result<(), String>) + 'static,
) {
    let end = Instant::now() + patience;
    fn step(session: String, ready: &'static str, end: Instant, then: Then<()>) {
        let again = session.clone();
        evaluate(&session, ready, move |r| match r {
            Ok(Value::Bool(true)) => then(Ok(())),
            other if Instant::now() >= end => then(Err(format!("the hidden page never got ready: {other:?}"))),
            _ => crate::runtime::later(Duration::from_millis(50), move || step(again, ready, end, then)),
        });
    }
    step(session, ready, end, Box::new(then));
}

const REGISTRY_READY: &str = "typeof chrome === 'object' && !!chrome.developerPrivate && !!chrome.management";

const REGISTRY_WATCH: &str = "(() => { if (window.__ndWatching) return true; window.__ndWatching = true; \
    chrome.developerPrivate.onItemStateChanged.addListener(e => \
      __ndExtensions(JSON.stringify({ event: e.event_type, id: e.item_id }))); return true; })()";

/// The hidden `chrome://extensions` session, opened on first use.
fn registry(then: impl FnOnce(Result<String, String>) + 'static) {
    if let Some(session) = ST.with(|s| s.borrow().registry.clone()) {
        return then(Ok(session));
    }
    let opening = ST.with(|s| {
        let mut s = s.borrow_mut();
        s.registry_waiting.push(Box::new(then));
        std::mem::replace(&mut s.registry_opening, true)
    });
    if opening {
        return;
    }
    let finish = |r: Result<String, String>| {
        let waiting = ST.with(|s| {
            let mut s = s.borrow_mut();
            s.registry_opening = false;
            if let Ok(session) = &r {
                s.registry = Some(session.clone());
            }
            std::mem::take(&mut s.registry_waiting)
        });
        for w in waiting {
            w(r.clone());
        }
    };
    open_hidden("chrome://extensions", move |r| {
        let session = match r {
            Ok((_, session)) => session,
            Err(e) => return finish(Err(e)),
        };
        let params = json!({ "name": BINDING });
        let s2 = session.clone();
        pipe::call("Runtime.addBinding", params, Some(&session), move |r| {
            if let Err(e) = r {
                return finish(Err(e));
            }
            wait_until(s2.clone(), REGISTRY_READY, Duration::from_secs(20), move |r| match r {
                Ok(()) => evaluate(&s2.clone(), REGISTRY_WATCH, move |r| finish(r.map(|_| s2))),
                Err(e) => finish(Err(e)),
            });
        });
    });
}

fn registry_event(payload: &str) {
    let Ok(v) = serde_json::from_str::<Value>(payload) else { return };
    let id = v["id"].as_str().unwrap_or_default().to_owned();
    if id.is_empty() {
        return;
    }
    let change = match v["event"].as_str().unwrap_or_default() {
        "INSTALLED" => Change::Installed(id),
        "UNINSTALLED" => {
            ST.with(|s| s.borrow_mut().probes.remove(&id));
            Change::Uninstalled(id)
        }
        "LOADED" | "UNLOADED" | "PREFS_CHANGED" | "CONFIGURATION_CHANGED" | "PERMISSIONS_CHANGED" => {
            Change::Updated(id)
        }
        _ => return,
    };
    report_change(change);
}

const LIST: &str = r#"(async () => {
  const infos = await chrome.developerPrivate.getExtensionsInfo({ includeDisabled: true, includeTerminated: true });
  const icon = async (url) => {
    if (!url || url.startsWith('data:')) return url || null;
    try {
      const blob = await (await fetch(url)).blob();
      return await new Promise((ok) => { const r = new FileReader(); r.onload = () => ok(r.result); r.onerror = () => ok(null); r.readAsDataURL(blob); });
    } catch (e) { return null; }
  };
  const out = [];
  for (const i of infos) {
    if (i.type !== 'EXTENSION') continue;
    out.push({
      id: i.id, name: i.name, version: i.version, enabled: i.state === 'ENABLED',
      options: i.optionsPage ? i.optionsPage.url : null,
      action: i.pinnedToToolbar !== undefined,
      location: i.location, icon: await icon(i.iconUrl),
    });
  }
  return out;
})()"#;

fn parse_list(v: &Value) -> Vec<Extension> {
    v.as_array()
        .map(|items| {
            items
                .iter()
                .map(|i| Extension {
                    id: i["id"].as_str().unwrap_or_default().to_owned(),
                    name: i["name"].as_str().unwrap_or_default().to_owned(),
                    version: i["version"].as_str().unwrap_or_default().to_owned(),
                    enabled: i["enabled"].as_bool().unwrap_or(false),
                    icon: i["icon"].as_str().and_then(data_url_bytes).map(Rc::from),
                    options_url: i["options"].as_str().filter(|s| !s.is_empty()).map(str::to_owned),
                    has_action: i["action"].as_bool().unwrap_or(false),
                    source: match i["location"].as_str().unwrap_or_default() {
                        "FROM_STORE" => Source::Store,
                        "UNPACKED" => Source::Unpacked,
                        _ => Source::Other,
                    },
                })
                .collect()
        })
        .unwrap_or_default()
}

fn data_url_bytes(url: &str) -> Option<Vec<u8>> {
    let (head, body) = url.split_once(',')?;
    if !head.starts_with("data:") || !head.ends_with(";base64") {
        return None;
    }
    base64::engine::general_purpose::STANDARD.decode(body).ok()
}

impl ExtensionEngine for Engine {
    fn list(&self, done: Then<Vec<Extension>>) {
        let done = Rc::new(RefCell::new(Some(done)));
        let fail = {
            let done = done.clone();
            move |e: String| {
                if let Some(d) = done.borrow_mut().take() {
                    d(Err(e));
                }
            }
        };
        when_started(
            move || {
                registry(move |r| {
                    let Some(done) = done.borrow_mut().take() else { return };
                    match r {
                        Ok(session) => evaluate(&session, LIST, move |r| done(r.map(|v| parse_list(&v)))),
                        Err(e) => done(Err(e)),
                    }
                });
                Ok(())
            },
            fail,
        );
    }

    fn load_unpacked(&self, path: PathBuf, done: Then<String>) {
        let path = match std::fs::canonicalize(&path) {
            Ok(p) => p,
            Err(e) => return done(Err(format!("{}: {e}", path.display()))),
        };
        if path.to_string_lossy().contains(',') {
            return done(Err(format!("{}: Chromium cannot load a path with a comma at startup", path.display())));
        }
        let done = Rc::new(RefCell::new(Some(done)));
        let d2 = done.clone();
        when_started(
            move || {
                pipe::call("Extensions.loadUnpacked", json!({ "path": path.to_string_lossy() }), None, move |r| {
                    let Some(done) = d2.borrow_mut().take() else { return };
                    let r = r.map(|v| v["id"].as_str().unwrap_or_default().to_owned());
                    if let Ok(id) = &r {
                        remember(id, Some(&path));
                    }
                    done(r);
                });
                Ok(())
            },
            move |e| {
                if let Some(d) = done.borrow_mut().take() {
                    d(Err(e));
                }
            },
        );
    }

    fn install_package(&self, path: PathBuf, done: Then<String>) {
        let unpacked = crate::crx::unpack(&path, &crate::runtime::root_cache().join("nd-packages"));
        match unpacked {
            Ok(dir) => self.load_unpacked(dir, done),
            Err(e) => done(Err(e)),
        }
    }

    fn uninstall(&self, id: String, done: Then<()>) {
        let code = format!(
            "chrome.management.uninstall({}, {{ showConfirmDialog: false }}).then(() => true)",
            Value::String(id.clone())
        );
        self.registry_call(
            code,
            Box::new(move |r| {
                if r.is_ok() {
                    remember(&id, None);
                }
                done(r);
            }),
        );
    }

    fn set_enabled(&self, id: String, enabled: bool, done: Then<()>) {
        let code = format!("chrome.management.setEnabled({}, {enabled}).then(() => true)", Value::String(id));
        self.registry_call(code, done);
    }

    fn watch_store(&self) {
        watch_store();
    }

    fn install_from_store(&self, id: String, done: Then<String>) {
        let done = Rc::new(RefCell::new(Some(done)));
        let d2 = done.clone();
        let finish = move |r: Result<String, String>| {
            if let Some(d) = d2.borrow_mut().take() {
                d(r);
            }
        };
        let f2 = finish.clone();
        when_started(
            move || {
                pipe::call("Browser.getVersion", json!({}), None, move |r| {
                    let version = r
                        .ok()
                        .and_then(|v| v["product"].as_str().and_then(|p| p.split('/').nth(1)).map(str::to_owned))
                        .unwrap_or_else(|| "154.0.0.0".into());
                    store_manifest(id.clone(), version, move |r| match r {
                        Ok(manifest) => install_in_hidden_store(id, manifest, Box::new(f2)),
                        Err(e) => f2(Err(e)),
                    });
                });
                Ok(())
            },
            move |e| finish(Err(e)),
        );
    }
}

/// The manifest of the package the store serves for `id`, fetched off the main thread.
fn store_manifest(id: String, version: String, then: impl FnOnce(Result<String, String>) + 'static) {
    let url = format!(
        "https://clients2.google.com/service/update2/crx?response=redirect&prodversion={version}\
         &acceptformat=crx3&x=id%3D{id}%26installsource%3Dondemand%26uc"
    );
    let slot = std::sync::Arc::new(std::sync::Mutex::new(None));
    let s2 = slot.clone();
    let fetched = std::thread::Builder::new().name("nd-cef-store".into()).spawn(move || {
        let r = ureq::get(&url)
            .call()
            .map_err(|e| format!("the Web Store: {e}"))
            .and_then(|mut r| r.body_mut().with_config().limit(256 << 20).read_to_vec().map_err(|e| e.to_string()))
            .and_then(|bytes| crate::crx::manifest(&bytes))
            .and_then(
                |(got, manifest)| {
                    if got == id { Ok(manifest) } else { Err(format!("the store served {got} for {id}")) }
                },
            );
        if let Ok(mut s) = s2.lock() {
            *s = Some(r);
        }
    });
    if let Err(e) = fetched {
        return then(Err(e.to_string()));
    }
    fn poll(slot: std::sync::Arc<std::sync::Mutex<Option<Result<String, String>>>>, then: Then<String>) {
        match slot.lock().ok().and_then(|mut s| s.take()) {
            Some(r) => then(r),
            None => crate::runtime::later(Duration::from_millis(50), move || poll(slot, then)),
        }
    }
    poll(slot, Box::new(then));
}

/// `true` once the API is there; otherwise where the page is, for the error.
const STORE_READY: &str = "(typeof chrome === 'object' && !!chrome.webstorePrivate) || location.href";

/// `chrome.webstorePrivate`'s two steps, run in a store page: answers the error or `true`.
fn store_install_code(id: &str, manifest: &str) -> String {
    let details = json!({ "id": id, "manifest": manifest });
    format!(
        r#"(async () => {{
  const wp = chrome.webstorePrivate;
  const call = (f, ...args) => new Promise((ok) => f.call(wp, ...args, (r) => ok({{ r, e: chrome.runtime.lastError && chrome.runtime.lastError.message }})));
  const begin = await call(wp.beginInstallWithManifest3, {details});
  if (begin.e || (begin.r && begin.r !== 'success')) return 'begin: ' + (begin.e || begin.r);
  const done = await call(wp.completeInstall, {id});
  if (done.e) return 'install: ' + done.e;
  return true;
}})()"#,
        id = Value::String(id.to_owned())
    )
}

/// Opens the quiet store page in a browser of nd-cef's own (`store_browser`) and attaches to it.
fn open_store_browser(then: impl FnOnce(Result<(cef::Browser, String), String>) + 'static) {
    thread_local! {
        static OPENED: Cell<u64> = const { Cell::new(0) };
    }
    let n = OPENED.with(|o| {
        o.set(o.get() + 1);
        o.get()
    });
    let url = format!("{STORE_QUIET}?nd={}-{n}", std::process::id());
    let browser = match crate::store_browser::create(&url) {
        Ok(b) => b,
        Err(e) => return then(Err(e)),
    };
    fn find(url: String, end: Instant, then: Then<String>) {
        pipe::call("Target.getTargets", json!({}), None, move |r| {
            let found = r.ok().and_then(|v| {
                let infos = v["targetInfos"].as_array()?;
                let info = infos.iter().find(|t| t["url"].as_str() == Some(url.as_str()))?;
                info["targetId"].as_str().map(str::to_owned)
            });
            match found {
                Some(target) => then(Ok(target)),
                None if Instant::now() >= end => then(Err("the store page never loaded".into())),
                None => crate::runtime::later(Duration::from_millis(50), move || find(url, end, then)),
            }
        });
    }
    let b2 = browser.clone();
    find(
        url,
        Instant::now() + Duration::from_secs(30),
        Box::new(move |r| {
            let target = match r {
                Ok(t) => t,
                Err(e) => {
                    crate::store_browser::close(&b2);
                    return then(Err(e));
                }
            };
            ST.with(|s| s.borrow_mut().own_targets.push(target.clone()));
            let params = json!({ "targetId": target, "flatten": true });
            pipe::call("Target.attachToTarget", params, None, move |r| match r {
                Ok(v) => {
                    let session = v["sessionId"].as_str().unwrap_or_default().to_owned();
                    let s2 = session.clone();
                    pipe::call("Runtime.enable", json!({}), Some(&s2), move |_| then(Ok((b2, session))));
                }
                Err(e) => {
                    crate::store_browser::close(&b2);
                    then(Err(e))
                }
            });
        }),
    );
}

fn install_in_hidden_store(id: String, manifest: String, done: Then<String>) {
    open_store_browser(move |r| {
        let (browser, session) = match r {
            Ok(v) => v,
            Err(e) => return done(Err(e)),
        };
        let s2 = session.clone();
        wait_until(session, STORE_READY, Duration::from_secs(30), move |r| {
            if let Err(e) = r {
                crate::store_browser::close(&browser);
                return done(Err(format!("the Web Store page: {e}")));
            }
            crate::platform::install_prompt::arm(PROMPT_WAIT);
            evaluate(&s2, &store_install_code(&id, &manifest), move |r| {
                crate::platform::install_prompt::disarm();
                crate::store_browser::close(&browser);
                done(match r {
                    Ok(Value::Bool(true)) => Ok(id),
                    Ok(other) => Err(other.as_str().map_or_else(|| other.to_string(), str::to_owned)),
                    Err(e) => Err(e),
                })
            });
        });
    });
}

/// Store pages hold `beginInstallWithManifest3` until the app answers through `__ndStoreAnswer`.
/// Main world, document start, so the store never holds the function unwrapped.
const STORE_HOOK: &str = r#"(() => {
  if (window.__ndStoreAnswer) return true;
  const pending = new Map();
  let begin = null, api = null;
  window.__ndStoreAnswer = (id, accept) => {
    const held = pending.get(id);
    if (!held) return 'none';
    pending.delete(id);
    if (!accept) { held.cancel(); return 'cancelled'; }
    if (held.callback) begin.call(api, held.details, held.callback);
    else begin.call(api, held.details).then(held.resolve, held.reject);
    return 'sent';
  };
  const hook = () => {
    const wp = window.chrome && window.chrome.webstorePrivate;
    if (!wp || typeof wp.beginInstallWithManifest3 !== 'function') return false;
    api = wp;
    begin = wp.beginInstallWithManifest3;
    wp.beginInstallWithManifest3 = function (details, callback) {
      if (typeof __ndStoreAsk !== 'function') return begin.call(api, ...arguments);
      const ask = (held) => {
        pending.set(details.id, held);
        __ndStoreAsk(JSON.stringify({ id: details.id, name: details.localizedName || '', iconUrl: details.iconUrl || '', manifest: details.manifest || '' }));
      };
      if (typeof callback === 'function') {
        ask({ details, callback, cancel: () => callback('user_cancelled') });
        return undefined;
      }
      return new Promise((resolve, reject) => ask({ details, resolve, reject, cancel: () => reject(new Error('User cancelled install')) }));
    };
    return true;
  };
  if (!hook()) {
    const timer = setInterval(() => { if (hook()) clearInterval(timer); }, 20);
    setTimeout(() => clearInterval(timer), 60000);
  }
  return true;
})()"#;

/// Store installs go to the app from now on: every store page gets the hook.
pub(crate) fn watch_store() {
    if ST.with(|s| std::mem::replace(&mut s.borrow_mut().watching_store, true)) {
        return;
    }
    if crate::runtime::started() {
        started();
    }
}

/// Chromium is up (`runtime::start`).
pub(crate) fn started() {
    if ST.with(|s| s.borrow().watching_store) && pipe::unavailable().is_none() {
        ensure_listening();
        discover(|| {});
    }
}

/// A target appeared or navigated: a store page gets the hook. The script registered for new
/// documents does not reach every one (none ran in a store page loaded right after an install),
/// so each navigation runs the hook again, and a page whose session is gone is attached anew.
/// Without the binding the hook leaves the install to Chrome's own prompt.
fn store_page(info: &Value) {
    let (Some(target), Some(url)) = (info["targetId"].as_str(), info["url"].as_str()) else { return };
    // `targetCreated` for a hidden page of nd's own can arrive before its id is in `own_targets`.
    if info["type"].as_str() != Some("page") || !url.starts_with(STORE) || url.starts_with(STORE_QUIET) {
        return;
    }
    enum Next {
        Skip,
        Attach,
        Rehook(String),
    }
    let next = ST.with(|s| {
        let mut s = s.borrow_mut();
        if !s.watching_store || s.own_targets.iter().any(|t| t == target) {
            return Next::Skip;
        }
        match s.store_pages.get(target) {
            Some(session) if session.is_empty() => Next::Skip,
            Some(session) => Next::Rehook(session.clone()),
            None => {
                s.store_pages.insert(target.to_owned(), String::new());
                Next::Attach
            }
        }
    });
    match next {
        Next::Skip => return,
        Next::Attach => {}
        // A binding can be gone from the new document too.
        Next::Rehook(session) => {
            let (target, info, s2) = (target.to_owned(), info.clone(), session.clone());
            pipe::call("Runtime.addBinding", json!({ "name": STORE_BINDING }), Some(&session), move |_| {
                evaluate(&s2, STORE_HOOK, move |r| {
                    if r.is_err() {
                        ST.with(|s| s.borrow_mut().store_pages.remove(&target));
                        store_page(&info);
                    }
                });
            });
            return;
        }
    }
    let target = target.to_owned();
    pipe::call("Target.attachToTarget", json!({ "targetId": target, "flatten": true }), None, move |r| {
        let session = match r {
            Ok(v) => v["sessionId"].as_str().unwrap_or_default().to_owned(),
            Err(e) => {
                tracing::warn!("nd-cef: a Web Store page: {e}");
                return;
            }
        };
        ST.with(|s| s.borrow_mut().store_pages.insert(target, session.clone()));
        let s2 = session.clone();
        pipe::call("Runtime.addBinding", json!({ "name": STORE_BINDING }), Some(&session), move |_| {
            let s3 = s2.clone();
            pipe::call(
                "Page.addScriptToEvaluateOnNewDocument",
                json!({ "source": STORE_HOOK }),
                Some(&s2),
                move |_| {
                    evaluate(&s3, STORE_HOOK, |r| {
                        if let Err(e) = r {
                            tracing::warn!("nd-cef: the Web Store hook: {e}");
                        }
                    });
                },
            );
        });
    });
}

/// A store page asked to install: the app answers, then the page's held call goes on or ends.
fn store_asked(session: &str, payload: &str) {
    let Ok(v) = serde_json::from_str::<Value>(payload) else { return };
    let id = v["id"].as_str().unwrap_or_default().to_owned();
    if !nd_webview::extensions::is_extension_id(&id) {
        return;
    }
    let session = session.to_owned();
    let icon = v["iconUrl"].as_str().filter(|u| !u.is_empty()).map(str::to_owned);
    let name = v["name"].as_str().unwrap_or_default().to_owned();
    let manifest = v["manifest"].as_str().unwrap_or_default().to_owned();
    let answer_id = id.clone();
    nd_webview::extensions::ask_store_install(id, name, icon, &manifest, move |decision| {
        use nd_webview::extensions::StoreDecision;
        let accept = decision != StoreDecision::Cancel;
        if decision == StoreDecision::Install {
            crate::platform::install_prompt::arm(PROMPT_WAIT);
        }
        let code = format!("window.__ndStoreAnswer({}, {accept})", Value::String(answer_id));
        evaluate(&session, &code, |r| {
            if let Err(e) = r {
                tracing::warn!("nd-cef: answering the Web Store: {e}");
            }
        });
    });
}

fn record_path() -> PathBuf {
    crate::runtime::root_cache().join("nd-unpacked")
}

/// The recorded unpacked extensions: (id, directory), one `id<TAB>path` per line.
fn recorded() -> Vec<(String, PathBuf)> {
    let text = std::fs::read_to_string(record_path()).unwrap_or_default();
    text.lines().filter_map(|l| l.split_once('\t')).map(|(id, path)| (id.to_owned(), PathBuf::from(path))).collect()
}

/// Records extension `id` as loaded from `path`, or forgets it.
fn remember(id: &str, path: Option<&Path>) {
    let mut list: Vec<_> = recorded().into_iter().filter(|(i, _)| i != id).collect();
    if let Some(path) = path {
        list.push((id.to_owned(), path.to_owned()));
    }
    let text: String = list.iter().map(|(id, path)| format!("{id}\t{}\n", path.display())).collect();
    if let Err(e) = std::fs::write(record_path(), text) {
        tracing::warn!("nd-cef: recording unpacked extensions: {e}");
    }
}

/// `--load-extension` for the recorded unpacked extensions that are still on disk.
pub(crate) fn switches(line: &mut cef::CommandLine) {
    use cef::ImplCommandLine as _;
    let dirs: Vec<String> = recorded()
        .into_iter()
        .filter(|(_, path)| path.join("manifest.json").is_file())
        .map(|(_, path)| path.to_string_lossy().into_owned())
        .collect();
    if dirs.is_empty() {
        return;
    }
    let name = cef::CefString::from("load-extension");
    line.append_switch_with_value(Some(&name), Some(&cef::CefString::from(dirs.join(",").as_str())));
}

impl Engine {
    fn registry_call(&self, code: String, done: Then<()>) {
        let done = Rc::new(RefCell::new(Some(done)));
        let d2 = done.clone();
        when_started(
            move || {
                registry(move |r| {
                    let Some(done) = d2.borrow_mut().take() else { return };
                    match r {
                        Ok(session) => evaluate(&session, &code, move |r| done(r.map(|_| ()))),
                        Err(e) => done(Err(e)),
                    }
                });
                Ok(())
            },
            move |e| {
                if let Some(d) = done.borrow_mut().take() {
                    d(Err(e));
                }
            },
        );
    }
}

/// The hidden session of extension `id`'s `manifest.json`, where `chrome.action` answers.
fn probe(id: &str, then: impl FnOnce(Result<String, String>) + 'static) {
    if let Some(session) = ST.with(|s| s.borrow().probes.get(id).cloned()) {
        return then(Ok(session));
    }
    let id = id.to_owned();
    open_hidden(&format!("chrome-extension://{id}/manifest.json"), move |r| match r {
        Ok((_, session)) => {
            let ready = "typeof chrome === 'object' && !!(chrome.action || chrome.browserAction || chrome.pageAction)";
            let s2 = session.clone();
            wait_until(session, ready, Duration::from_secs(10), move |r| match r {
                Ok(()) => {
                    ST.with(|s| s.borrow_mut().probes.insert(id, s2.clone()));
                    then(Ok(s2));
                }
                Err(_) => then(Err(format!("extension {id} has no toolbar action"))),
            });
        }
        Err(e) => then(Err(e)),
    });
}

/// `window` is the page's Chrome window: extension APIs number tabs by Chromium's session ids,
/// which CEF's browser identifier is not, and each Chrome-style page is a window of one tab.
fn action_state_code(window: i64) -> String {
    format!(
        r#"(async () => {{
  const a = chrome.action || chrome.browserAction || chrome.pageAction;
  const [t] = await chrome.tabs.query({{ windowId: {window}, active: true }});
  if (!t) throw new Error('Chromium lists no tab for the page');
  const tab = t.id;
  const d = {{ tabId: tab }};
  const p = (f, ...args) => new Promise((ok) => {{ try {{ const r = f.call(a, ...args, (v) => ok(v)); if (r && r.then) r.then(ok, () => ok(null)); }} catch (e) {{ ok(null); }} }});
  const enabled = a.isEnabled ? await p(a.isEnabled, tab) : true;
  return {{
    title: await p(a.getTitle, d), badge: a.getBadgeText ? await p(a.getBadgeText, d) : '',
    color: a.getBadgeBackgroundColor ? await p(a.getBadgeBackgroundColor, d) : null,
    popup: await p(a.getPopup, d), enabled: enabled !== false,
  }};
}})()"#
    )
}

fn parse_action(v: &Value) -> ActionState {
    let color = v["color"].as_array().filter(|c| c.len() == 4).map(|c| {
        let b = |i: usize| c[i].as_u64().unwrap_or(0).min(255) as u8;
        [b(0), b(1), b(2), b(3)]
    });
    ActionState {
        title: v["title"].as_str().unwrap_or_default().to_owned(),
        badge: v["badge"].as_str().unwrap_or_default().to_owned(),
        badge_color: color,
        has_popup: v["popup"].as_str().is_some_and(|p| !p.is_empty()),
        enabled: v["enabled"].as_bool().unwrap_or(true),
    }
}

/// `chrome.action`'s state of extension `id` for `page`'s tab.
pub(crate) fn action_state(page: &CefPage, id: String, done: Then<ActionState>) {
    let done = Rc::new(RefCell::new(Some(done)));
    let d2 = done.clone();
    let page_id = page.id;
    when_started(
        move || {
            let Some(p) = page::get(page_id) else { return Err("the page closed".into()) };
            p.target_id(move |target| {
                let d3 = d2.clone();
                let fail = move |e: String| {
                    if let Some(d) = d3.borrow_mut().take() {
                        d(Err(e));
                    }
                };
                let target = match target {
                    Ok(t) => t,
                    Err(e) => return fail(e),
                };
                pipe::call("Browser.getWindowForTarget", json!({ "targetId": target }), None, move |r| {
                    let window = match r.map(|v| v["windowId"].as_i64()) {
                        Ok(Some(w)) => w,
                        Ok(None) => return fail("Chromium names no window for the page".into()),
                        Err(e) => return fail(e),
                    };
                    probe(&id, move |r| {
                        let Some(done) = d2.borrow_mut().take() else { return };
                        match r {
                            Ok(session) => evaluate(&session, &action_state_code(window), move |r| {
                                done(r.map(|v| parse_action(&v)))
                            }),
                            Err(e) => done(Err(e)),
                        }
                    });
                });
            });
            Ok(())
        },
        move |e| {
            if let Some(d) = done.borrow_mut().take() {
                d(Err(e));
            }
        },
    );
}

fn discover(then: impl FnOnce() + 'static) {
    if ST.with(|s| std::mem::replace(&mut s.borrow_mut().discovering, true)) {
        return then();
    }
    pipe::call("Target.setDiscoverTargets", json!({ "discover": true }), None, move |r| {
        if let Err(e) = r {
            tracing::warn!("nd-cef: target discovery: {e}");
        }
        then();
    });
}

/// A click on extension `id`'s action for `page`, its popup placed under `anchor`.
pub(crate) fn run_action(page: &CefPage, id: String, anchor: Anchor, done: Then<ActionOutcome>) {
    let page_id = page.id;
    if let Err(e) = page.view.extension_actions() {
        return done(Err(e));
    }
    let reclick = ST.with(|s| {
        let s = s.borrow();
        s.last_close.as_ref().is_some_and(|(i, p, at)| *i == id && *p == page_id && at.elapsed() < RECLICK)
    });
    if reclick {
        return done(Ok(ActionOutcome::PopupClosed));
    }
    if let Some(open) =
        ST.with(|s| s.borrow().popup.as_ref().filter(|p| p.page == page_id).map(|p| (p.id.clone(), p.target.clone())))
    {
        close_target(&open.1);
        if open.0 == id {
            return done(Ok(ActionOutcome::PopupClosed));
        }
    }
    let done = Rc::new(RefCell::new(Some(done)));
    let d2 = done.clone();
    when_started(
        move || {
            crate::native_messaging::link_hosts(&crate::runtime::root_cache());
            discover(move || {
                let Some(page) = page::get(page_id) else {
                    if let Some(d) = d2.borrow_mut().take() {
                        d(Err("the page closed".into()));
                    }
                    return;
                };
                page.target_id(move |r| {
                    let Some(done) = d2.borrow_mut().take() else { return };
                    match r {
                        Ok(target) => tab_target(target, move |r| match r {
                            Ok(tab) => trigger(id, page_id, tab, anchor, Instant::now() + TRIGGER_PATIENCE, 0, done),
                            Err(e) => done(Err(e)),
                        }),
                        Err(e) => done(Err(e)),
                    }
                });
            });
            Ok(())
        },
        move |e| {
            if let Some(d) = done.borrow_mut().take() {
                d(Err(e));
            }
        },
    );
}

/// The tab target holding page target `page`. Chromium lists tabs without their pages, so each
/// tab is attached in turn and auto-attach names the page inside it.
fn tab_target(page: String, then: impl FnOnce(Result<String, String>) + 'static) {
    if let Some(tab) = ST.with(|s| s.borrow().tabs.get(&page).cloned()) {
        return then(Ok(tab));
    }
    pipe::call("Target.getTargets", json!({ "filter": [{ "type": "tab" }] }), None, move |r| {
        let tabs: Vec<String> = match r {
            Ok(v) => v["targetInfos"]
                .as_array()
                .map(|a| a.iter().filter_map(|t| t["targetId"].as_str().map(str::to_owned)).collect())
                .unwrap_or_default(),
            Err(e) => return then(Err(e)),
        };
        search_tabs(page, tabs, Box::new(then));
    });
}

fn search_tabs(page: String, mut tabs: Vec<String>, then: Then<String>) {
    let Some(tab) = tabs.pop() else { return then(Err("Chromium lists no tab for the page".into())) };
    let params = json!({ "targetId": tab, "flatten": true });
    pipe::call("Target.attachToTarget", params, None, move |r| {
        let session = match r {
            Ok(v) => v["sessionId"].as_str().unwrap_or_default().to_owned(),
            Err(_) => return search_tabs(page, tabs, then),
        };
        let params = json!({ "autoAttach": true, "waitForDebuggerOnStart": false, "flatten": true });
        let s2 = session.clone();
        pipe::call("Target.setAutoAttach", params, Some(&session), move |_| {
            let children = ST.with(|s| s.borrow_mut().children.remove(&s2)).unwrap_or_default();
            pipe::call("Target.detachFromTarget", json!({ "sessionId": s2 }), None, |_| {});
            if children.contains(&page) {
                ST.with(|s| s.borrow_mut().tabs.insert(page, tab.clone()));
                then(Ok(tab));
            } else {
                search_tabs(page, tabs, then);
            }
        });
    });
}

fn trigger(id: String, page: u64, tab: String, anchor: Anchor, end: Instant, tries: u32, done: Then<ActionOutcome>) {
    let params = json!({ "id": id, "targetId": tab });
    pipe::call("Extensions.triggerAction", params, None, move |r| match r {
        Ok(_) => {
            if tries > 0 {
                tracing::info!("nd-cef: extension {id}'s action ran after {tries} retries while Chromium was starting");
            }
            if let Some(p) = page::get(page) {
                p.view.track_extension_popup(&id, anchor);
            }
            await_popup(id, page, done);
        }
        // Chromium builds the page window's toolbar model once the extension system is ready.
        Err(e) if Instant::now() < end && (e.contains("No action found") || e.contains("not attached")) => {
            crate::runtime::later(Duration::from_millis(50), move || {
                trigger(id, page, tab, anchor, end, tries + 1, done)
            });
        }
        Err(e) => done(Err(e)),
    });
}

fn await_popup(id: String, page: u64, done: Then<ActionOutcome>) {
    ST.with(|s| s.borrow_mut().awaiting_popup.push(AwaitingPopup { id: id.clone(), page, done: Some(done) }));
    crate::runtime::later(POPUP_WAIT, move || {
        let waiting = ST.with(|s| {
            let mut s = s.borrow_mut();
            let at = s.awaiting_popup.iter().position(|a| a.id == id && a.page == page)?;
            Some(s.awaiting_popup.remove(at))
        });
        if let Some(mut w) = waiting {
            if let Some(p) = page::get(page) {
                p.view.track_extension_popup_cancel();
            }
            if let Some(done) = w.done.take() {
                done(Ok(ActionOutcome::Clicked));
            }
        }
    });
}

/// A target appeared or changed: a popup's contents exist before they navigate, so the URL that
/// names its extension arrives with `Target.targetInfoChanged`.
fn target_created(info: &Value) {
    let (Some(target), Some(url)) = (info["targetId"].as_str(), info["url"].as_str()) else { return };
    let Some(rest) = url.strip_prefix("chrome-extension://") else { return };
    let ext = rest.split('/').next().unwrap_or_default().to_owned();
    if matches!(info["type"].as_str(), Some("service_worker" | "background_page" | "worker" | "shared_worker")) {
        return;
    }
    let waiting = ST.with(|s| {
        let mut s = s.borrow_mut();
        if s.own_targets.iter().any(|t| t == target) {
            return None;
        }
        let at = s.awaiting_popup.iter().position(|a| a.id == ext)?;
        let w = s.awaiting_popup.remove(at);
        s.popup = Some(Popup { id: ext.clone(), page: w.page, target: target.to_owned() });
        Some(w)
    });
    if let Some(mut w) = waiting
        && let Some(done) = w.done.take()
    {
        done(Ok(ActionOutcome::PopupOpened));
    }
}

fn target_destroyed(target: &str) {
    let closed = ST.with(|s| {
        let mut s = s.borrow_mut();
        s.own_targets.retain(|t| t != target);
        if s.popup.as_ref().is_some_and(|p| p.target == target) {
            let p = s.popup.take()?;
            s.last_close = Some((p.id.clone(), p.page, Instant::now()));
            return Some(p);
        }
        None
    });
    if let Some(p) = closed
        && let Some(page) = page::get(p.page)
    {
        page.view.track_extension_popup_cancel();
        page.page.emit(nd_webview::ExtensionPopupClosed { id: p.id });
    }
}

fn close_target(target: &str) {
    pipe::call("Target.closeTarget", json!({ "targetId": target }), None, |_| {});
}

/// Closes the popup a click opened for `page`, if one is open.
pub(crate) fn close_popup(page: u64) {
    if let Some(target) = ST.with(|s| s.borrow().popup.as_ref().filter(|p| p.page == page).map(|p| p.target.clone())) {
        close_target(&target);
    }
}
