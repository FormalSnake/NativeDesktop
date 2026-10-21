//! `BROWSER_SCENE=tabdrag`: two windows of tabs in a sidebar, as the tab drag gate drives them.
//! Rows drag within their list and between the Pinned and Tabs sections, into the other window,
//! and out of every window into a new one; a window whose last tab leaves closes. A tab carries
//! its live page: the counter page keeps counting and keeps its field's text, and the local
//! server counts its loads, so a move that reloaded it shows.
//!
//! Each window shows its state in the `state` text: the tabs in order (`*` marks pinned ones),
//! the selected tab and its page's title, and the counter's loads. `BROWSER_SCENE=reveal` opens
//! one window whose sidebar is hidden and slides in from the leading edge.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use nd::core::{Point, WindowId};
use nd::prelude::*;
use nd::webview::{LoadingChanged, TitleChanged, WebView, webview};

const KIND: &str = "tab";
const PINNED: &str = "pinned";
const TABS: &str = "tabs";
const COUNTER: &str = "<!doctype html><title>Counter</title><input id=\"field\"><script>let n = 0; \
const f = document.getElementById(\"field\"); const show = () => { document.title = `Counter c=${n} f=${f.value}`; }; \
setInterval(() => { n++; show(); }, 500); f.addEventListener(\"input\", show); \
window.nbTest = () => { f.value = \"typed-by-drive\"; f.dispatchEvent(new Event(\"input\")); };</script>";

type Loads = Arc<Mutex<HashMap<String, usize>>>;

/// Serves `/counter` and `/<name>` (a page titled with its name), counting each path's loads.
fn serve(loads: Loads) -> String {
    use std::io::{BufRead, BufReader, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind a local port");
    let base = format!("http://127.0.0.1:{}", listener.local_addr().expect("local addr").port());
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let loads = loads.clone();
            std::thread::spawn(move || {
                let mut stream = stream;
                let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
                let mut line = String::new();
                if reader.read_line(&mut line).is_err() {
                    return;
                }
                let mut header = String::new();
                while reader.read_line(&mut header).is_ok_and(|n| n > 2) {
                    header.clear();
                }
                let key = line.split_whitespace().nth(1).unwrap_or("/").trim_start_matches('/').to_owned();
                if key == "favicon.ico" {
                    let _ =
                        stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                    return;
                }
                *loads.lock().expect("loads").entry(key.clone()).or_default() += 1;
                let body = if key == "counter" {
                    COUNTER.to_owned()
                } else {
                    format!("<!doctype html><title>Page {key}</title><h1>{key}</h1>")
                };
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(body.as_bytes());
            });
        }
    });
    base
}

struct Tab {
    id: u64,
    page: WebView,
    title: SharedString,
    pinned: bool,
    /// The drive's field fill ran (on the counter's first load).
    filled: bool,
}

struct Win {
    id: WindowId,
    tabs: Vec<Tab>,
    selected: u64,
    view: Option<Entity<TabWindow>>,
}

pub struct Store {
    wins: Vec<Win>,
    next: u64,
    loads: Loads,
    opened: u32,
}

impl Store {
    fn tab(&mut self, url: &str, pinned: bool) -> Tab {
        self.next += 1;
        let page = WebView::new();
        page.load(url);
        Tab { id: self.next, page, title: "".into(), pinned, filled: false }
    }

    fn win(&mut self, id: WindowId) -> Option<&mut Win> {
        self.wins.iter_mut().find(|w| w.id == id)
    }

    fn take(&mut self, tab: u64) -> Option<(WindowId, usize, Tab)> {
        for w in &mut self.wins {
            if let Some(at) = w.tabs.iter().position(|t| t.id == tab) {
                let tab = w.tabs.remove(at);
                if w.selected == tab.id {
                    w.selected = w.tabs.get(at.min(w.tabs.len().saturating_sub(1))).map_or(0, |t| t.id);
                }
                return Some((w.id, at, tab));
            }
        }
        None
    }

    /// Puts `tab` in `window`'s `pinned` section (or the tabs one) at `index` among that
    /// section's tabs, and selects it.
    fn place(&mut self, window: WindowId, mut tab: Tab, pinned: bool, index: usize) {
        let Some(w) = self.win(window) else { return };
        tab.pinned = pinned;
        let section: Vec<usize> = (0..w.tabs.len()).filter(|i| w.tabs[*i].pinned == pinned).collect();
        let at = match section.get(index) {
            Some(i) => *i,
            None => section.last().map_or(if pinned { 0 } else { w.tabs.len() }, |i| i + 1),
        };
        w.selected = tab.id;
        w.tabs.insert(at.min(w.tabs.len()), tab);
    }

    fn state(&self, window: WindowId) -> String {
        let Some(w) = self.wins.iter().find(|w| w.id == window) else { return String::new() };
        let tabs: Vec<String> = w.tabs.iter().map(|t| format!("{}{}", if t.pinned { "*" } else { "" }, t.id)).collect();
        let title = w.tabs.iter().find(|t| t.id == w.selected).map_or("", |t| t.title.as_ref());
        let loads = self.loads.lock().map_or(0, |l| l.get("counter").copied().unwrap_or(0));
        format!("tabs={} sel={} loads={} title={}", tabs.join(","), w.selected, loads, title)
    }
}

pub struct TabWindow {
    store: Entity<Store>,
}

fn refresh_all(store: &Entity<Store>, cx: &mut App) {
    let views: Vec<Entity<TabWindow>> = store.read(cx).wins.iter().filter_map(|w| w.view.clone()).collect();
    for view in views {
        view.update(cx, |_, cx| cx.notify());
    }
}

/// Moves a dropped tab into `window` at the slot the list reported, and closes the window it
/// left when that was its last tab.
fn drop_tab(store: &Entity<Store>, window: WindowId, e: &Dropped, cx: &mut App) {
    let (Ok(tab), Some(slot)) = (e.item.id.parse::<u64>(), e.slot.clone()) else { return };
    let pinned = slot.parent.as_deref() == Some(PINNED);
    let emptied = store.update(cx, |s, _| {
        let (from, at, tab) = s.take(tab)?;
        let mut index = slot.index;
        // The slot counts the dragged row where it was.
        if from == window && tab.pinned == pinned {
            let before = s
                .wins
                .iter()
                .find(|w| w.id == window)
                .map_or(0, |w| w.tabs[..at].iter().filter(|t| t.pinned == pinned).count());
            if before < index {
                index -= 1;
            }
        }
        s.place(window, tab, pinned, index);
        let empty = s.win(from).is_some_and(|w| w.tabs.is_empty());
        if empty {
            s.wins.retain(|w| w.id != from);
        }
        empty.then_some(from)
    });
    if let Some(gone) = emptied {
        cx.close_window(gone);
    }
    refresh_all(store, cx);
}

/// A tab released outside every window opens in a window of its own there.
fn tear_off(store: &Entity<Store>, item: &DragItem, at: Point, cx: &mut App) {
    let Ok(tab) = item.id.parse::<u64>() else { return };
    let Some((from, _, tab)) = store.update(cx, |s, _| s.take(tab)) else { return };
    let emptied = store.update(cx, |s, _| {
        let empty = s.win(from).is_some_and(|w| w.tabs.is_empty());
        if empty {
            s.wins.retain(|w| w.id != from);
        }
        empty
    });
    open(store, vec![tab], Some(at), cx);
    if emptied {
        cx.close_window(from);
    }
    refresh_all(store, cx);
}

impl Render for TabWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let me = window.id();
        let store = self.store.read(cx);
        let Some(w) = store.wins.iter().find(|w| w.id == me) else { return div().into_any_element() };
        let mut nodes = vec![source_section(PINNED, "Pinned"), source_section(TABS, "Tabs")];
        for t in &w.tabs {
            let title = if t.title.is_empty() { SharedString::from(format!("Tab {}", t.id)) } else { t.title.clone() };
            nodes.push(source_node(t.id.to_string(), title).parent(if t.pinned { PINNED } else { TABS }));
        }
        let state = store.state(me);
        let selected = w.tabs.iter().find(|t| t.id == w.selected);
        let page = selected.map(|t| {
            let id = t.id;
            let title = cx.listener(move |this, e: &TitleChanged, _, cx| {
                this.store.update(cx, |s, _| {
                    for w in &mut s.wins {
                        if let Some(t) = w.tabs.iter_mut().find(|t| t.id == id) {
                            t.title = e.title.clone().into();
                        }
                    }
                });
                cx.notify();
            });
            let loaded = cx.listener(move |this, e: &LoadingChanged, _, cx| {
                if e.loading {
                    return;
                }
                this.store.update(cx, |s, _| {
                    for w in &mut s.wins {
                        if let Some(t) = w.tabs.iter_mut().find(|t| t.id == id && !t.filled) {
                            t.filled = true;
                            t.page.execute_javascript("window.nbTest && window.nbTest()", |_, _| {});
                        }
                    }
                });
            });
            webview(SharedString::from(format!("page-{id}")), &t.page)
                .size_full()
                .on_title_changed(title)
                .on_loading_changed(loaded)
        });
        let (drop_store, end_store, select_store) = (self.store.clone(), self.store.clone(), self.store.clone());
        scaffold()
            .sidebar(
                sidebar().width(px(240.)).child(
                    source_list("tabs")
                        .nodes(nodes)
                        .selected(w.selected.to_string())
                        .draggable_rows(KIND)
                        .accepts_drops([KIND])
                        .on_select(move |e: &SourceSelected, window, cx| {
                            let (me, id) = (window.id(), e.id.parse().unwrap_or(0));
                            select_store.update(cx, |s, _| {
                                if let Some(w) = s.win(me) {
                                    w.selected = id;
                                }
                            });
                            refresh_all(&select_store, cx);
                        })
                        .on_drop(move |e: &Dropped, window, cx| drop_tab(&drop_store, window.id(), e, cx))
                        .on_drag_end(move |e: &DragEnded, _, cx| {
                            if let DragOutcome::Outside { screen } = e.outcome {
                                tear_off(&end_store, &e.item, screen, cx);
                            }
                        }),
                ),
            )
            .content(v_flex().size_full().child(text(state).id("state")).child(div().flex_1().min_h_0().children(page)))
            .into_any_element()
    }
}

fn open(store: &Entity<Store>, tabs: Vec<Tab>, at: Option<Point>, cx: &mut App) -> WindowId {
    let name = store.update(cx, |s, _| {
        s.opened += 1;
        format!("w{}", s.opened)
    });
    let mut options = WindowOptions::main(name.clone()).title(name).size(size(px(640.), px(480.)));
    if let Some(at) = at {
        options = options.origin(at);
    }
    let selected = tabs.first().map_or(0, |t| t.id);
    let handle = {
        let store = store.clone();
        cx.open_window(options, move |_, cx| cx.new(|_| TabWindow { store }))
    };
    let (id, view) = (handle.id(), handle.root());
    store.update(cx, |s, _| s.wins.push(Win { id, tabs, selected, view: Some(view.clone()) }));
    id
}

/// Two windows: the counter, a pinned page and page A in `w1`, page B in `w2`.
pub fn tabdrag(cx: &mut App) {
    let loads: Loads = Arc::default();
    let base = serve(loads.clone());
    let store = cx.new(|_| Store { wins: Vec::new(), next: 0, loads, opened: 0 });
    let (first, second) = store.update(cx, |s, _| {
        let first = vec![
            s.tab(&format!("{base}/pin"), true),
            s.tab(&format!("{base}/counter"), false),
            s.tab(&format!("{base}/a"), false),
        ];
        (first, vec![s.tab(&format!("{base}/b"), false)])
    });
    let counter = first[1].id;
    let w1 = open(&store, first, Some(Point { x: 80., y: 120. }), cx);
    store.update(cx, |s, _| {
        if let Some(w) = s.win(w1) {
            w.selected = counter;
        }
    });
    open(&store, second, Some(Point { x: 760., y: 120. }), cx);
}

struct Reveal {
    revealed: bool,
}

impl Render for Reveal {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        scaffold()
            .sidebar(
                sidebar()
                    .width(px(240.))
                    .collapsed(true)
                    .edge_reveal()
                    .on_reveal_change(cx.listener(|this, e: &SidebarRevealed, _, cx| {
                        this.revealed = e.revealed;
                        cx.notify();
                    }))
                    .child(
                        v_flex()
                            .size_full()
                            .child(div().id("lights").child(window_controls().embedded()))
                            .child(div().id("tiles").h(px(120.)).w_full()),
                    ),
            )
            .content(v_flex().size_full().child(text(format!("revealed={}", self.revealed)).id("state")))
    }
}

/// One window whose sidebar is hidden and slides in from the leading edge, its traffic lights
/// riding on it.
pub fn reveal(cx: &mut App) {
    let options = WindowOptions::main("reveal").title("Reveal").size(size(px(900.), px(600.)));
    cx.open_window(options, |_, cx| cx.new(|_| Reveal { revealed: false }));
}
