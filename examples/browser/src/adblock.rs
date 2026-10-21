//! `BROWSER_SCENE=adblock`: an ad blocker on `nd::webview::set_request_filter`. adblock-rust
//! compiles the lists uBlock Origin enables on a fresh install, from the snapshots bundled in its
//! pinned release, with the release's `redirect=` resources. The fixture page is the 0.5.11
//! app's (`scripts/adblock-drive.ts` in nativebrowser): real ad-network URLs, answered before
//! they reach the network, so the run needs no network once the release is cached. It must
//! report what that build reported: the GPT, AdSense and Analytics scripts replaced by uBO's
//! stubs, prebid and the tracking pixel blocked. A second page checks that a `$csp=` rule's
//! policy reaches the document. The filter must never run on the main thread.
//!
//! The release is downloaded once into `ND_ADBLOCK_CACHE` (default `target/nd-ubo`) and checked
//! against its pinned SHA-256; `ND_ADBLOCK_ZIP` points at a local copy instead. Prints
//! `ND_ADBLOCK_OK` with the counts, or `ND_ADBLOCK_FAILED` and why, and exits.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread::ThreadId;

use adblock::Engine;
use adblock::lists::{FilterFormat, FilterSet, ParseOptions};
use adblock::request::Request;
use nd::prelude::*;
use nd::webview::{
    FilterRequest, FilterResponse, LoadingChanged, RequestFilter, TitleChanged, Verdict, WebView, webview,
};
use sha2::{Digest, Sha256};

use crate::server;

const UBO_VERSION: &str = "1.75.0";
const UBO_SHA256: &str = "393cf95709d1074d4022970e9014e434395c53a822387f1e25f43be97cf4b582";
/// The app's own rule on top of the lists, for the CSP leg.
const CSP_RULE: &str = "/nd-csp-probe$csp=script-src 'none'";

fn fail(why: impl std::fmt::Display) -> ! {
    eprintln!("ND_ADBLOCK_FAILED {why}");
    std::process::exit(1);
}

/// The extracted release, downloading and checking it first when it is not cached.
fn release() -> PathBuf {
    let cache = std::env::var_os("ND_ADBLOCK_CACHE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/nd-ubo"));
    let dir = cache.join(UBO_VERSION);
    if dir.join("assets/assets.json").exists() {
        return dir;
    }
    let zip = match std::env::var_os("ND_ADBLOCK_ZIP") {
        Some(path) => PathBuf::from(path),
        None => {
            let path = cache.join(format!("uBlock0_{UBO_VERSION}.chromium.zip"));
            if !path.exists() {
                let _ = std::fs::create_dir_all(&cache);
                let url = format!(
                    "https://github.com/gorhill/uBlock/releases/download/{UBO_VERSION}/uBlock0_{UBO_VERSION}.chromium.zip"
                );
                let ok = std::process::Command::new("curl")
                    .args(["-fsSL", "--max-time", "120", "-o"])
                    .arg(&path)
                    .arg(&url)
                    .status()
                    .is_ok_and(|s| s.success());
                if !ok {
                    let _ = std::fs::remove_file(&path);
                    fail(format!("downloading {url}"));
                }
            }
            path
        }
    };
    let bytes = std::fs::read(&zip).unwrap_or_else(|e| fail(format!("{}: {e}", zip.display())));
    let digest: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
    if digest != UBO_SHA256 {
        fail(format!("{}: sha256 {digest}, expected {UBO_SHA256}", zip.display()));
    }
    let mut archive =
        zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap_or_else(|e| fail(format!("{}: {e}", zip.display())));
    let staging = cache.join(format!("{UBO_VERSION}.partial"));
    let _ = std::fs::remove_dir_all(&staging);
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).unwrap_or_else(|e| fail(format!("zip entry {i}: {e}")));
        let Some(name) = entry.enclosed_name() else { fail(format!("zip entry escapes: {}", entry.name())) };
        // Everything sits in one top folder (`uBlock0.chromium/`), dropped here.
        let inner: PathBuf = name.components().skip(1).collect();
        if entry.is_dir() || inner.as_os_str().is_empty() {
            continue;
        }
        let out = staging.join(inner);
        let written = std::fs::create_dir_all(out.parent().unwrap_or(&staging))
            .and_then(|()| std::fs::File::create(&out))
            .and_then(|mut file| std::io::copy(&mut entry, &mut file));
        if let Err(e) = written {
            fail(format!("{}: {e}", out.display()));
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::rename(&staging, &dir).unwrap_or_else(|e| fail(format!("{}: {e}", dir.display())));
    dir
}

/// The lists uBO enables on a fresh install, from the release's own `assets.json`, each from
/// its bundled snapshot.
fn engine(release: &Path) -> (Engine, usize) {
    let assets: serde_json::Value = std::fs::read_to_string(release.join("assets/assets.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| fail("assets.json is unreadable"));
    let mut set = FilterSet::new(false);
    let mut lists = 0;
    for (key, entry) in assets.as_object().into_iter().flatten() {
        if entry["content"] != "filters" || entry["off"] == true {
            continue;
        }
        let urls: Vec<&str> = match &entry["contentURL"] {
            serde_json::Value::String(u) => vec![u.as_str()],
            serde_json::Value::Array(a) => a.iter().filter_map(|u| u.as_str()).collect(),
            _ => Vec::new(),
        };
        let Some(text) = urls
            .iter()
            .filter(|u| u.starts_with("assets/"))
            .find_map(|u| std::fs::read_to_string(release.join(u)).ok())
        else {
            continue;
        };
        // Peter Lowe's list ships in hosts-file form, like uBO reads it.
        let format = if key.starts_with("plowe") { FilterFormat::Hosts } else { FilterFormat::Standard };
        set.add_filter_list(text, ParseOptions { format, ..ParseOptions::default() });
        lists += 1;
    }
    set.add_filter_list(CSP_RULE.to_owned(), ParseOptions::default());
    let mut engine = Engine::new_with_filter_set(set);
    engine.use_resources(adblock::resources::resource_assembler::assemble_web_accessible_resources(
        &release.join("web_accessible_resources"),
        &release.join("js/redirect-resources.js"),
    ));
    (engine, lists)
}

/// `data:<mime>;base64,<body>`, the form adblock-rust hands a `redirect=` resource in.
fn data_url(url: &str) -> Option<(String, Vec<u8>)> {
    use base64::Engine as _;
    let (head, body) = url.strip_prefix("data:")?.split_once(',')?;
    let mime = head.strip_suffix(";base64")?;
    let body = base64::engine::general_purpose::STANDARD.decode(body).ok()?;
    Some((mime.to_owned(), body))
}

struct Blocker {
    engine: Engine,
    main: ThreadId,
    blocked: AtomicUsize,
    redirected: AtomicUsize,
    on_main: AtomicUsize,
}

impl Blocker {
    fn request<'a>(&self, r: &FilterRequest<'a>) -> Option<Request> {
        let source = if r.first_party.is_empty() { r.initiator } else { r.first_party };
        Request::new(r.url, source, r.kind.name(), r.method).ok()
    }
}

/// The filter Chromium holds; the scene keeps the counts.
struct Shared(Arc<Blocker>);

impl RequestFilter for Shared {
    fn request(&self, r: &FilterRequest<'_>) -> Verdict {
        let b = &self.0;
        if std::thread::current().id() == b.main {
            b.on_main.fetch_add(1, Ordering::Relaxed);
        }
        let Some(request) = b.request(r) else { return Verdict::Allow };
        let result = b.engine.check_network_request(&request);
        if let Some((mime, body)) = result.redirect.as_deref().and_then(data_url) {
            b.redirected.fetch_add(1, Ordering::Relaxed);
            return Verdict::Respond { mime, body: body.into() };
        }
        if result.should_block() {
            b.blocked.fetch_add(1, Ordering::Relaxed);
            return Verdict::Block;
        }
        match result.rewritten_url {
            Some(url) => Verdict::Redirect(url),
            None => Verdict::Allow,
        }
    }

    fn response(&self, r: &FilterRequest<'_>, _response: &FilterResponse<'_>) -> Option<String> {
        self.0.engine.get_csp_directives(&self.0.request(r)?)
    }
}

/// What the fixture page reports through its title, and what the 0.5.11 build reported.
const EXPECTED: [(&str, bool); 5] =
    [("gptStub", true), ("adsbygoogle", true), ("analytics", true), ("prebid", false), ("pixel", false)];

struct Scene {
    page: WebView,
    base: String,
    blocker: Arc<Blocker>,
    report: Option<String>,
}

impl Scene {
    fn title(&mut self, title: &str) {
        if let Some(raw) = title.strip_prefix('R') {
            if self.report.is_some() {
                return;
            }
            let v: serde_json::Value = serde_json::from_str(raw).unwrap_or_else(|e| fail(format!("{raw}: {e}")));
            for (key, want) in EXPECTED {
                if v[key] != want {
                    fail(format!("{key} is {}, the 0.5.11 build saw {want}: {raw}", v[key]));
                }
            }
            eprintln!("ND_ADBLOCK fixture {raw}");
            self.report = Some(raw.to_owned());
            self.page.load(format!("{}/nd-csp-probe", self.base));
        }
    }

    /// The probe page's inline script sets `window.ran`; the policy the `$csp=` rule added must
    /// have stopped it.
    fn loaded(&mut self) {
        if self.report.is_none() || !self.page.url().ends_with("/nd-csp-probe") {
            return;
        }
        let blocker = self.blocker.clone();
        self.page.execute_javascript("window.ran === true", move |result, _cx| {
            if result != Ok(serde_json::Value::Bool(false)) {
                fail(format!("the $csp rule did not reach the document: window.ran is {result:?}"));
            }
            let on_main = blocker.on_main.load(Ordering::Relaxed);
            if on_main > 0 {
                fail(format!("the filter ran on the main thread {on_main} times"));
            }
            eprintln!(
                "ND_ADBLOCK_OK blocked={} redirected={} csp=ok",
                blocker.blocked.load(Ordering::Relaxed),
                blocker.redirected.load(Ordering::Relaxed),
            );
            std::process::exit(0);
        });
    }
}

impl Render for Scene {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        webview("web", &self.page)
            .size_full()
            .on_title_changed(cx.listener(|this, e: &TitleChanged, _, _| this.title(&e.title)))
            .on_loading_changed(cx.listener(|this, e: &LoadingChanged, _, _| {
                if !e.loading {
                    this.loaded();
                }
            }))
    }
}

pub fn open(cx: &mut App) {
    let release = release();
    let started = std::time::Instant::now();
    let (engine, lists) = engine(&release);
    eprintln!("ND_ADBLOCK lists={lists} compiled_ms={}", started.elapsed().as_millis());
    let blocker = Arc::new(Blocker {
        engine,
        main: std::thread::current().id(),
        blocked: AtomicUsize::new(0),
        redirected: AtomicUsize::new(0),
        on_main: AtomicUsize::new(0),
    });
    nd::webview::set_request_filter(Shared(blocker.clone()));
    let base = server::start();
    let page = WebView::new();
    page.load(format!("{base}/adblock"));
    let watchdog = std::time::Duration::from_secs(90);
    std::thread::spawn(move || {
        std::thread::sleep(watchdog);
        fail("no verdict within 90 s");
    });
    let options = WindowOptions::main("main").title("Adblock").size(size(px(900.), px(640.)));
    cx.open_window(options, move |_window, cx| cx.new(move |_cx| Scene { page, base, blocker, report: None }));
}
