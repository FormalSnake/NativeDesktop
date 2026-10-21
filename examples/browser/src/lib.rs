//! A small browser: tabs in the sidebar, back, forward, reload and an address field in the
//! toolbar, devtools, downloads to `~/Downloads`, and links that open new windows as tabs that
//! keep `window.opener`. Pages belong to the app, not the view, so switching tabs never reloads
//! one. The engine is `[webview] engine` (WKWebView by default), or `ND_WEBVIEW_ENGINE=cef` in a
//! build with the `cef` feature.
//!
//! `BROWSER_SCENE` picks a gate scene instead: `overlays` (popover, panel and context menu over a
//! page that counts its input), `resize` (a page filling the window), `input` (a text field in a
//! page that scrolls), `chrome-pages` (`chrome://extensions` and `chrome://settings` in a
//! Chrome-style page), `extensions` (a Chrome extension's popup, native messaging and fill right
//! after launch, printing `ND_EXTENSIONS_OK`), `adblock` (uBlock Origin's default lists through the
//! request filter, printing `ND_ADBLOCK_OK`), `private` (a private window's tabs share one
//! in-memory profile, printing `ND_PRIVATE_OK`), `pip` (the floating video chord's script lifts a
//! playing video into picture-in-picture and back, printing `ND_PIP_OK`) or `parity` (every event and command against a local test page, printing
//! `ND_WEBVIEW_PARITY_OK`).

mod adblock;
mod extensions;
mod parity;
mod pip;
mod private;
mod scenes;
mod server;

use nd::prelude::*;
use nd::webview::{
    ContextMenuItemClicked, Contexts, DownloadRequested, DownloadState, DownloadUpdated, LoadFailed, LoadProgress,
    LoadingChanged, MenuItem, Navigate, NewWindow, TitleChanged, WebView, WebViewOptions, WindowClosed,
    resolve_address, webview,
};

actions!(browser, [NewTab, CloseTab, Reload, Back, Forward, DevTools, NextTab]);

const HOME: &str = "https://example.com";
const SEARCH: &str = "https://duckduckgo.com/?q={}";

struct Tab {
    id: u64,
    page: WebView,
    title: SharedString,
    url: SharedString,
    loading: bool,
    progress: f64,
}

pub struct Browser {
    tabs: Vec<Tab>,
    selected: u64,
    next: u64,
    address: SharedString,
    status: SharedString,
}

// docs:begin options
fn options() -> WebViewOptions {
    WebViewOptions::default().adopt_popups().inspectable(true)
}
// docs:end options

impl Browser {
    fn new(url: &str) -> Self {
        let mut this = Self { tabs: Vec::new(), selected: 0, next: 1, address: url.into(), status: "".into() };
        let page = WebView::with_options(options());
        page.load(url);
        this.push(page, true);
        this
    }

    fn push(&mut self, page: WebView, select: bool) -> u64 {
        let id = self.next;
        self.next += 1;
        self.tabs.push(Tab { id, page, title: "New Tab".into(), url: "".into(), loading: false, progress: 0. });
        if select {
            self.select(id);
        }
        id
    }

    fn select(&mut self, id: u64) {
        self.selected = id;
        if let Some(tab) = self.tab(id) {
            self.address = tab.url.clone();
        }
    }

    fn tab(&self, id: u64) -> Option<&Tab> {
        self.tabs.iter().find(|t| t.id == id)
    }

    fn tab_mut(&mut self, id: u64) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|t| t.id == id)
    }

    fn current(&self) -> Option<&WebView> {
        self.tab(self.selected).map(|t| &t.page)
    }

    fn open_tab(&mut self, url: &str, select: bool) {
        let page = WebView::with_options(options());
        page.load(url);
        self.push(page, select);
    }

    fn close(&mut self, id: u64) {
        let Some(at) = self.tabs.iter().position(|t| t.id == id) else { return };
        self.tabs.remove(at);
        if self.selected == id {
            let next = self.tabs.get(at.min(self.tabs.len().saturating_sub(1))).map_or(0, |t| t.id);
            self.select(next);
        }
    }

    fn page_view(&self, tab: &Tab, cx: &mut Context<Self>) -> AnyElement {
        let id = tab.id;
        let update = move |cx: &mut Context<Self>, this: &mut Self, f: &dyn Fn(&mut Tab)| {
            if let Some(tab) = this.tab_mut(id) {
                f(tab);
            }
            cx.notify();
        };
        // docs:begin mount
        webview(SharedString::from(format!("page-{id}")), &tab.page)
            .size_full()
            .context_menu_items(vec![
                MenuItem::new("open-link-tab", "Open Link in New Tab").contexts(Contexts::LINK),
                MenuItem::new("copy-address", "Copy Page Address"),
            ])
            .on_navigate(cx.listener(move |this, e: &Navigate, _, cx| {
                if this.selected == id {
                    this.address = e.url.clone().into();
                }
                update(cx, this, &|t| t.url = e.url.clone().into());
            }))
            .on_title_changed(cx.listener(move |this, e: &TitleChanged, _, cx| {
                let title = if e.title.is_empty() { "Untitled".to_owned() } else { e.title.clone() };
                update(cx, this, &|t| t.title = title.clone().into());
            }))
            // docs:end mount
            .on_loading_changed(cx.listener(move |this, e: &LoadingChanged, _, cx| {
                update(cx, this, &|t| t.loading = e.loading);
            }))
            .on_load_progress(cx.listener(move |this, e: &LoadProgress, _, cx| {
                update(cx, this, &|t| t.progress = e.progress);
            }))
            .on_load_failed(cx.listener(|this, e: &LoadFailed, _, cx| {
                this.status = format!("Could not load {}: {}", e.url, e.error).into();
                cx.notify();
            }))
            // docs:begin popups
            .on_new_window(cx.listener(move |this, e: &NewWindow, _, cx| {
                let select = e.disposition != nd::webview::Disposition::BackgroundTab;
                match &e.popup {
                    Some(popup) => {
                        this.push(popup.clone(), select);
                    }
                    None => this.open_tab(&e.url, select),
                }
                cx.notify();
            }))
            .on_window_closed(cx.listener(move |this, _: &WindowClosed, _, cx| {
                this.close(id);
                cx.notify();
            }))
            // docs:end popups
            // docs:begin downloads
            .on_download_requested(cx.listener(|this, e: &DownloadRequested, _, cx| {
                let path = e.download.save_to_downloads();
                this.status = format!("Downloading {}", path.display()).into();
                cx.notify();
            }))
            .on_download_updated(cx.listener(|this, e: &DownloadUpdated, _, cx| {
                this.status = match e.state {
                    DownloadState::Running => format!("Downloaded {} KB", e.received / 1024),
                    DownloadState::Done => {
                        format!("Saved {}", e.path.as_ref().map(|p| p.display().to_string()).unwrap_or_default())
                    }
                    DownloadState::Failed => format!("Download failed: {}", e.error.clone().unwrap_or_default()),
                    DownloadState::Cancelled => "Download cancelled".into(),
                }
                .into();
                cx.notify();
            }))
            // docs:end downloads
            .on_context_menu_item_clicked(cx.listener(|this, e: &ContextMenuItemClicked, _, cx| {
                match e.id.as_str() {
                    "open-link-tab" => {
                        if let Some(link) = &e.link_url {
                            this.open_tab(link, false);
                        }
                    }
                    "copy-address" => nd::clipboard::write_text(cx, &e.page_url),
                    _ => {}
                }
                cx.notify();
            }))
            .into_any_element()
    }
}

impl Render for Browser {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (secondary, margin) = {
            let theme = cx.theme();
            (theme.text.secondary, theme.spacing.content_margin)
        };
        let current = self.tab(self.selected);
        let (loading, fraction) = current.map_or((false, 0.), |t| (t.loading, t.progress));
        let content = match current {
            // docs:begin conditional
            Some(tab) => v_flex()
                .size_full()
                .when(loading, |d| d.child(progress(Some(fraction as f32)).id("progress").w_full()))
                .child(div().flex_1().w_full().child(self.page_view(tab, cx)))
                .when(!self.status.is_empty(), |d| {
                    d.child(text(self.status.clone()).id("status").text_color(secondary).p(margin))
                })
                .into_any_element(),
            // docs:end conditional
            None => v_flex().size_full().items_center().justify_center().child(text("No open tabs")).into_any_element(),
        };
        let items: Vec<SourceItem> =
            self.tabs.iter().map(|t| source_item(t.id.to_string(), t.title.clone()).icon(Icon::Globe)).collect();
        scaffold()
            .key_context("Browser")
            .on_action(cx.listener(|this, _: &NewTab, _, cx| {
                this.open_tab(HOME, true);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &CloseTab, _, cx| {
                let id = this.selected;
                this.close(id);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &NextTab, _, cx| {
                if let Some(at) = this.tabs.iter().position(|t| t.id == this.selected) {
                    let next = this.tabs[(at + 1) % this.tabs.len()].id;
                    this.select(next);
                    cx.notify();
                }
            }))
            // docs:begin commands
            .on_action(cx.listener(|this, _: &Reload, _, _| {
                if let Some(page) = this.current() {
                    if page.is_loading() { page.stop() } else { page.reload() }
                }
            }))
            .on_action(cx.listener(|this, _: &Back, _, _| {
                if let Some(page) = this.current() {
                    page.go_back();
                }
            }))
            .on_action(cx.listener(|this, _: &Forward, _, _| {
                if let Some(page) = this.current() {
                    page.go_forward();
                }
            }))
            .on_action(cx.listener(|this, _: &DevTools, _, _| {
                if let Some(page) = this.current() {
                    page.open_dev_tools();
                }
            }))
            // docs:end commands
            .toolbar(
                toolbar()
                    .title(current.map_or_else(|| SharedString::from("Browser"), |t| t.title.clone()))
                    .sidebar_toggle()
                    .item(icon_button("back", Icon::ChevronLeft).tooltip("Back").dispatch(Back))
                    .item(icon_button("forward", Icon::ChevronRight).tooltip("Forward").dispatch(Forward))
                    .item(
                        icon_button("reload", if loading { Icon::X } else { Icon::RotateCw })
                            .tooltip(if loading { "Stop" } else { "Reload" })
                            .dispatch(Reload),
                    )
                    .item(
                        text_field("address")
                            .text(self.address.clone())
                            .placeholder("Search or enter address")
                            .w(px(460.))
                            .on_change(cx.listener(|this, e: &TextChanged, _, _| this.address = e.text.clone()))
                            .on_submit(cx.listener(|this, e: &TextSubmitted, _, cx| {
                                let url = resolve_address(&e.text, SEARCH);
                                this.address = url.clone().into();
                                if let Some(page) = this.current() {
                                    page.load(url);
                                    page.focus();
                                }
                                cx.notify();
                            })),
                    )
                    .flexible_space()
                    .item(icon_button("devtools", Icon::Code).tooltip("Developer Tools").dispatch(DevTools))
                    .item(icon_button("new-tab", Icon::Plus).tooltip("New Tab").dispatch(NewTab)),
            )
            .sidebar(sidebar().min_w(px(200.)).child(
                source_list("tabs").section("Tabs", items).selected(self.selected.to_string()).on_select(cx.listener(
                    |this, e: &SourceSelected, _, cx| {
                        if let Ok(id) = e.id.parse() {
                            this.select(id);
                            cx.notify();
                        }
                    },
                )),
            ))
            .content(content)
    }
}

pub fn app() -> Application {
    Application::new(nd::manifest!())
}

fn bind_keys(cx: &mut App) {
    let m = if cfg!(target_os = "macos") { "cmd" } else { "ctrl" };
    cx.bind_keys([
        KeyBinding::new(&format!("{m}-t"), NewTab, Some("Browser")),
        KeyBinding::new(&format!("{m}-w"), CloseTab, Some("Browser")),
        KeyBinding::new(&format!("{m}-r"), Reload, Some("Browser")),
        KeyBinding::new(&format!("{m}-["), Back, Some("Browser")),
        KeyBinding::new(&format!("{m}-]"), Forward, Some("Browser")),
        KeyBinding::new(&format!("{m}-alt-i"), DevTools, Some("Browser")),
        KeyBinding::new("ctrl-tab", NextTab, Some("Browser")),
    ]);
}

pub fn init(cx: &mut App) {
    match std::env::var("BROWSER_SCENE").as_deref() {
        Ok("overlays") => scenes::overlays(cx),
        Ok("resize") => scenes::resize(cx),
        Ok("input") => scenes::input(cx),
        Ok("chrome-pages") => scenes::chrome_pages(cx),
        Ok("parity") => parity::open(cx),
        Ok("extensions") => extensions::open(cx),
        Ok("adblock") => adblock::open(cx),
        Ok("private") => private::open(cx),
        Ok("pip") => pip::open(cx),
        _ => {
            bind_keys(cx);
            let url = std::env::var("BROWSER_URL").unwrap_or_else(|_| HOME.into());
            cx.open_window(
                WindowOptions::main("main").title("Browser").size(size(px(1100.), px(720.))),
                move |_window, cx| cx.new(move |_cx| Browser::new(&url)),
            );
        }
    }
}
