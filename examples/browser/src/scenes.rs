//! The gate scenes: what `examples/overlays` shows, with the page in a `webview()` on whichever
//! engine the run picked.

use nd::prelude::*;
use nd::webview::{LoadingChanged, WebView, webview};

actions!(browser_scenes, [Choose]);

const PAGE: &str = "<!doctype html><html><body style=\"margin:0;height:100vh;background:#2f8f5b;\
font:15px -apple-system;color:white;display:flex;align-items:center;justify-content:center\">\
page</body></html>";

/// A text field and a page tall enough to scroll; the title says what reached it: the field's
/// text, the scroll offset, whether the field has focus and the last press the page saw. It reads
/// `loading` until its script runs.
const INPUT_PAGE: &str = "<!doctype html><html><head><title>loading</title></head>\
<body style=\"margin:0;height:3000px;background:#2f8f5b\">\
<input id=\"field\" style=\"margin:24px;width:240px;font:18px sans-serif\">\
<script>var f=document.getElementById('field'),at='none';\
function t(){document.title='typed='+f.value+' y='+Math.round(scrollY)+' f='+(document.activeElement===f?1:0)+' at='+at;}\
f.addEventListener('input',t);f.addEventListener('focus',t);f.addEventListener('blur',t);addEventListener('scroll',t);\
addEventListener('mousedown',function(e){at=e.clientX+','+e.clientY;t();},true);t();</script>\
</body></html>";

const EDGE_PAGE: &str = "<!doctype html><html><body style=\"margin:0;height:100vh;background:#2f8f5b\"></body></html>";

struct Overlays {
    page: WebView,
    popover: bool,
    panel: bool,
    under: u32,
    chosen: u32,
}

impl Overlays {
    fn status(&self) -> String {
        format!(
            "under={} chosen={} popover={} panel={}",
            self.under,
            self.chosen,
            if self.popover { "open" } else { "closed" },
            if self.panel { "open" } else { "closed" },
        )
    }
}

impl Render for Overlays {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .p(px(16.))
            .gap(px(12.))
            .on_action(cx.listener(|this, _: &Choose, _, cx| {
                this.chosen += 1;
                cx.notify();
            }))
            .child(
                h_flex()
                    .gap(px(8.))
                    .child(
                        div()
                            .id("anchor")
                            .w(px(320.))
                            .h(px(28.))
                            .child(button("open-popover", "Popover").on_click(cx.listener(
                                |this, _: &ClickEvent, _, cx| {
                                    this.popover = true;
                                    cx.notify();
                                },
                            )))
                            .when(self.popover, |d| {
                                d.child(
                                    popover("pop")
                                        .w(px(300.))
                                        .h(px(200.))
                                        .p(px(12.))
                                        .child(text("Clicks in here stay in here."))
                                        .on_dismiss(cx.listener(|this, _: &Dismissed, _, cx| {
                                            this.popover = false;
                                            cx.notify();
                                        })),
                                )
                            }),
                    )
                    .child(button("open-panel", "Panel").on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.panel = true;
                        cx.notify();
                    }))),
            )
            .child(
                h_flex()
                    .items_start()
                    .gap(px(12.))
                    .flex_1()
                    .child(button("under", "Under").w(px(150.)).h(px(32.)).on_click(cx.listener(
                        |this, _: &ClickEvent, _, cx| {
                            this.under += 1;
                            cx.notify();
                        },
                    )))
                    .child(
                        div()
                            .id("menu-zone")
                            .w(px(40.))
                            .h(px(260.))
                            .bg(rgb(0x8899aa))
                            .context_menu(|menu, _, _| menu.item("Choose", Choose).separator().item("Again", Choose)),
                    )
                    .child(webview("web", &self.page).count_input().flex_1().h(px(320.))),
            )
            .child(text(self.status()).id("status"))
            .when(self.panel, |d| {
                d.child(
                    panel("panel")
                        .w(px(260.))
                        .h(px(120.))
                        .p(px(12.))
                        .child(text("A modal panel. Esc closes it."))
                        .on_dismiss(cx.listener(|this, _: &Dismissed, _, cx| {
                            this.panel = false;
                            cx.notify();
                        })),
                )
            })
    }
}

/// A popover, a modal panel and a context menu over a native button and a page counting the
/// pointer input it sees (`gate_m8_webview::real_overlay_leak_*`).
pub fn overlays(cx: &mut App) {
    let page = WebView::new();
    page.load_html(PAGE, None);
    let options = WindowOptions::main("main").title("Overlays").size(size(px(720.), px(520.)));
    cx.open_window(options, move |_window, cx| {
        cx.new(move |_cx| Overlays { page, popover: false, panel: false, under: 0, chosen: 0 })
    });
}

struct Edge {
    page: WebView,
}

impl Render for Edge {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        // The page is the root's only child: with `ND_DEBUG_EXPOSE` any frame where it lags the
        // window shows the magenta window background.
        webview("web", &self.page).size_full()
    }
}

/// A flat page filling the window, for the real-drag resize gate.
pub fn resize(cx: &mut App) {
    let page = WebView::new();
    page.load_html(EDGE_PAGE, None);
    let options = WindowOptions::main("main").title("Resize").size(size(px(720.), px(520.)));
    cx.open_window(options, move |_window, cx| cx.new(move |_cx| Edge { page }));
}

/// A page with a text field filling the window, for the input gate: clicks focus the field,
/// typed keys reach it and the wheel scrolls the page.
pub fn input(cx: &mut App) {
    let page = WebView::new();
    page.load_html(INPUT_PAGE, None);
    let options = WindowOptions::main("main").title("Input").size(size(px(720.), px(520.)));
    cx.open_window(options, move |_window, cx| cx.new(move |_cx| Edge { page }));
}

/// Chrome's own pages, which only a Chrome-style page has: each must load and lay out its WebUI
/// root (`gate_m8_webview::real_chrome_pages`, `real_webview::real_chrome_pages_cef`). Prints
/// `ND_CHROME_PAGES_OK`, or `ND_CHROME_PAGES_FAILED` with the page, and exits.
const CHROME_PAGES: [(&str, &str); 2] = [("chrome://extensions", "extensions-manager"), ("chrome://settings", "settings-ui")];

struct ChromePages {
    page: WebView,
    at: usize,
    checking: bool,
    tries: u32,
}

impl ChromePages {
    fn loaded(&mut self, cx: &mut Context<Self>) {
        let Some((url, _)) = CHROME_PAGES.get(self.at).copied() else { return };
        if self.checking || !self.page.url().starts_with(url) {
            return;
        }
        self.checking = true;
        self.tries = 0;
        self.check(cx);
    }

    /// WebUI elements upgrade after the load: the root counts once it has a shadow tree and a
    /// size. Asked again until then, each answer a round trip to the page.
    fn check(&mut self, cx: &mut Context<Self>) {
        let Some((url, root)) = CHROME_PAGES.get(self.at).copied() else { return };
        let code = format!(
            "(() => {{ const e = document.querySelector('{root}'); \
             return !!(e && e.shadowRoot && e.getBoundingClientRect().width > 0); }})()"
        );
        let this = cx.weak_entity();
        self.page.execute_javascript(code, move |result, cx| {
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(v) if v == true => eprintln!("ND_CHROME_PAGES {url} ok"),
                    Ok(_) if this.tries < 3000 => {
                        this.tries += 1;
                        return this.check(cx);
                    }
                    other => {
                        eprintln!("ND_CHROME_PAGES_FAILED {url}: {other:?}");
                        std::process::exit(1);
                    }
                }
                this.at += 1;
                this.checking = false;
                match CHROME_PAGES.get(this.at) {
                    Some((next, _)) => this.page.load(*next),
                    None => {
                        eprintln!("ND_CHROME_PAGES_OK");
                        std::process::exit(0);
                    }
                }
            });
        });
    }
}

impl Render for ChromePages {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        webview("web", &self.page).size_full().on_loading_changed(cx.listener(
            |this, e: &LoadingChanged, _, cx| {
                if !e.loading {
                    this.loaded(cx);
                }
            },
        ))
    }
}

pub fn chrome_pages(cx: &mut App) {
    let page = WebView::new();
    page.load(CHROME_PAGES[0].0);
    let options = WindowOptions::main("main").title("Chrome pages").size(size(px(900.), px(640.)));
    cx.open_window(options, move |_window, cx| cx.new(move |_cx| ChromePages { page, at: 0, checking: false, tries: 0 }));
}
