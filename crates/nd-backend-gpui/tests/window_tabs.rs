//! Tier 2: window tab groups and the drawn tab strip on the GPUI backend inside GPUI's headless
//! app (ARCHITECTURE.md 5.5). Windows of one `tab_group` share a GPUI window with a drawn tab bar;
//! input reaches the bar through the window's own event path. Goldens of the bar, the overview and
//! both strip presentations, light and dark, render the view builders as they are.
//!
//! `ND_UPDATE_GOLDENS=1` rewrites the goldens: `tests/goldens/<os>/tabs-<view>-<scheme>.png`.

use std::cell::RefCell;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use nd_backend_gpui::GpuiHost;
use nd_backend_gpui::nav::{Marks, theme_for};
use nd_backend_gpui::tab_bar::{Bar, BarTab, overview_view, switcher_view, tab_bar_view};
use nd_components::*;
use nd_gpui::gpui::{self, HeadlessAppContext};
use nd_theme::Scheme;
use nd_ui::backend::WindowCommand;
use nd_ui::prelude::*;
use nd_ui::semantics::{MouseButton, PointerPhase, SemanticAction, SyntheticInput};
use nd_ui::{Keystroke, Point, Rect, SemanticNode, WindowId};

fn headless() -> HeadlessAppContext {
    let text = nd_gpui::gpui_platform::current_platform(true).text_system();
    HeadlessAppContext::with_platform(text, Arc::new(()), nd_gpui::gpui_platform::current_headless_renderer)
}

fn settle(cx: &mut HeadlessAppContext, host: &GpuiHost) {
    for _ in 0..4 {
        cx.run_until_parked();
        cx.update(|cx| {
            host.pump(cx);
            host.draw(cx);
        });
    }
    cx.run_until_parked();
}

fn find<'a>(node: &'a SemanticNode, f: &impl Fn(&SemanticNode) -> bool) -> Option<&'a SemanticNode> {
    if f(node) {
        return Some(node);
    }
    node.children.iter().find_map(|c| find(c, f))
}

fn by_test_id(host: &GpuiHost, window: WindowId, id: &str) -> Option<SemanticNode> {
    host.read(|app| {
        let tree = app.semantic_window(window)?;
        find(&tree.root, &|n| n.test_id.as_deref() == Some(id)).cloned()
    })
}

fn center(r: &Rect) -> Point {
    Point { x: r.origin.x + r.size.width / 2., y: r.origin.y + r.size.height / 2. }
}

/// A tab's content: a labelled page with a nested node, so a move carries a subtree.
struct Page {
    n: usize,
}

impl Render for Page {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .id(format!("page-{}", self.n))
            .size_full()
            .child(text(format!("Page {}", self.n)))
            .child(div().id(format!("detail-{}", self.n)).child(text("detail")))
    }
}

#[derive(Default)]
struct Log {
    closes: Vec<WindowId>,
    new_tabs: Vec<WindowId>,
    held: HashSet<WindowId>,
    opened: Vec<WindowId>,
}

type Shared = Rc<RefCell<Log>>;

fn open_tab(app: &mut App, group: &str, n: usize, size: (f32, f32), log: &Shared) -> WindowId {
    let options = WindowOptions::main(format!("{group}{n}"))
        .title(format!("Tab {n}"))
        .size(gpui::size(px(size.0), px(size.1)))
        .tab_group(group.to_owned());
    let (close_log, new_log, group) = (log.clone(), log.clone(), group.to_owned());
    let id = app
        .open_window(options, move |window, cx| {
            window.on_should_close(move |window, _| {
                let mut log = close_log.borrow_mut();
                log.closes.push(window.id());
                !log.held.contains(&window.id())
            });
            window.on_new_tab(move |window, cx| {
                new_log.borrow_mut().new_tabs.push(window.id());
                let n = new_log.borrow().opened.len() + 1;
                open_tab(cx, &group, n, size, &new_log);
            });
            cx.new(|_| Page { n })
        })
        .id();
    log.borrow_mut().opened.push(id);
    id
}

struct Rig {
    cx: HeadlessAppContext,
    host: GpuiHost,
    log: Shared,
    t: Vec<WindowId>,
    u: Vec<WindowId>,
}

/// `t` tabs of group "t" at 640x480 and `u` of group "u" at 1000x300.
fn open(t: usize, u: usize, single_canvas: bool) -> Rig {
    let mut cx = headless();
    let log: Shared = Rc::default();
    type Ids = Rc<RefCell<(Vec<WindowId>, Vec<WindowId>)>>;
    let (l, ids): (Shared, Ids) = (log.clone(), Rc::default());
    let out = ids.clone();
    let host = cx.update(|cx| {
        GpuiHost::builder().single_canvas(single_canvas).quit_when_empty(false).start(cx, move |app| {
            for n in 1..=t {
                let id = open_tab(app, "t", n, (640., 480.), &l);
                out.borrow_mut().0.push(id);
            }
            for n in 1..=u {
                let id = open_tab(app, "u", 100 + n, (1000., 300.), &l);
                out.borrow_mut().1.push(id);
            }
        })
    });
    settle(&mut cx, &host);
    let (t, u) = ids.borrow().clone();
    Rig { cx, host, log, t, u }
}

impl Rig {
    fn settle(&mut self) {
        settle(&mut self.cx, &self.host);
    }

    fn group(&self, window: WindowId) -> TabGroupInfo {
        self.host.tab_group(window).unwrap_or_else(|| panic!("{window:?} is in no group"))
    }

    fn input(&mut self, window: WindowId, inputs: Vec<SyntheticInput>) {
        self.cx.update(|cx| {
            self.host.update(cx, |app| {
                for input in &inputs {
                    app.synthesize(window, input).expect("posted");
                }
            })
        });
        self.settle();
    }

    fn click(&mut self, window: WindowId, at: Point) {
        self.press(window, at, MouseButton::Left);
    }

    fn press(&mut self, window: WindowId, at: Point, button: MouseButton) {
        let inputs = [PointerPhase::Down, PointerPhase::Up]
            .into_iter()
            .map(|phase| SyntheticInput::Pointer { phase, at, button, click_count: 1 })
            .collect();
        self.input(window, inputs);
    }

    fn drag(&mut self, window: WindowId, from: Point, to: Point) {
        self.input(window, vec![SyntheticInput::Drag { from, to, steps: 12 }]);
    }

    fn tab(&self, window: WindowId, tab: WindowId) -> Rect {
        let g = self.group(window);
        let i = g.tabs.iter().position(|t| *t == tab).expect("tab in group");
        g.layout.tabs[i]
    }

    fn handle(&self, window: WindowId) -> gpui::AnyWindowHandle {
        self.host.window_handle(window).expect("open")
    }

    /// The GPUI window's bounds on screen.
    fn bounds(&mut self, window: WindowId) -> Rect {
        let handle = self.handle(window);
        let b = self.cx.update(|cx| handle.update(cx, |_, w, _| w.bounds())).expect("window");
        Rect::new(b.origin.x.as_f32(), b.origin.y.as_f32(), b.size.width.as_f32(), b.size.height.as_f32())
    }

    fn page(&self, window: WindowId, n: usize) -> Option<SemanticNode> {
        by_test_id(&self.host, window, &format!("page-{n}"))
    }
}

use nd_backend_gpui::TabGroup as TabGroupInfo;

mod gate_m6 {
    use super::*;

    pub fn gpui_window_tabs_group_by_name() {
        let r = open(3, 1, false);
        let handle = r.handle(r.t[0]);
        assert!(r.t.iter().all(|t| r.host.window_handle(*t) == Some(handle)), "the t tabs share one GPUI window");
        assert_ne!(r.handle(r.u[0]), handle, "another group gets its own window");
        let g = r.group(r.t[0]);
        assert_eq!(g.tabs, r.t);
        assert_eq!(g.selected, r.t[2], "the newest tab is selected");
        assert_eq!(g.layout.tabs.len(), 3, "the bar drew a tab per window");
        assert!(r.page(r.t[2], 3).and_then(|p| p.bounds).is_some(), "the selected tab's tree shows");
        let u = r.group(r.u[0]);
        assert_eq!(u.tabs, r.u);
        assert_eq!(u.layout.bar, None, "a one-tab group hides its bar, as AdwTabBar autohides");
        let bar = g.layout.bar.expect("bar");
        let page = r.page(r.t[2], 3).and_then(|p| p.bounds).expect("page");
        assert!(page.origin.y >= bar.origin.y + bar.size.height - 0.5, "content sits below the bar: {bar:?} {page:?}");
    }

    pub fn gpui_window_tabs_click_selects() {
        let mut r = open(3, 0, false);
        let first = center(&r.tab(r.t[0], r.t[0]));
        r.click(r.t[2], first);
        assert_eq!(r.group(r.t[0]).selected, r.t[0]);
        assert!(r.page(r.t[0], 1).and_then(|p| p.bounds).is_some());
        let state = nd_ui::backend::Backend::window_state(r.host.backend(), r.t[2]).expect("state");
        assert!(!state.visible, "a tab not shown is not visible");
        r.cx.update(|cx| r.host.update(cx, |app| app.window_command(r.t[1], WindowCommand::Activate)));
        r.settle();
        assert_eq!(r.group(r.t[0]).selected, r.t[1], "activating a window selects its tab");
        let keys = SyntheticInput::Key(Keystroke::parse("ctrl-tab").expect("key"));
        r.input(r.t[1], vec![keys]);
        assert_eq!(r.group(r.t[0]).selected, r.t[2], "ctrl-tab cycles to the next tab");
        let keys = SyntheticInput::Key(Keystroke::parse("ctrl-shift-tab").expect("key"));
        r.input(r.t[2], vec![keys]);
        assert_eq!(r.group(r.t[0]).selected, r.t[1]);
    }

    pub fn gpui_window_tabs_close_holds_or_closes() {
        let mut r = open(3, 0, false);
        let (t1, t2, t3) = (r.t[0], r.t[1], r.t[2]);
        r.log.borrow_mut().held.insert(t2);
        let close = r.group(t1).layout.closes[1].expect("close button");
        r.click(t3, center(&close));
        assert_eq!(r.log.borrow().closes, [t2], "the close asked the app once");
        assert_eq!(r.group(t1).tabs, [t1, t2, t3], "a held close keeps the tab");
        r.log.borrow_mut().held.clear();
        let middle = center(&r.tab(t1, t2));
        r.press(t3, middle, MouseButton::Middle);
        assert_eq!(r.group(t1).tabs, [t1, t3], "middle click closes");
        assert_eq!(r.host.window_handle(t2), None);
        // The selected tab's close selects the tab after it.
        let close = r.group(t1).layout.closes[1].expect("close button");
        r.click(t3, center(&close));
        assert_eq!(r.group(t1).tabs, [t1]);
        assert_eq!(r.group(t1).selected, t1);
        let handle = r.handle(t1);
        r.cx.update(|cx| r.host.request_window_close(cx, t1));
        r.settle();
        assert_eq!(r.host.window_handle(t1), None, "closing the last tab closes the group window");
        let still_open = r.cx.update(|cx| handle.update(cx, |_, _, _| ()).is_ok());
        assert!(!still_open, "the GPUI window is gone");
    }

    pub fn gpui_window_tabs_window_close_asks_every_tab() {
        let mut r = open(3, 0, false);
        r.log.borrow_mut().held.insert(r.t[0]);
        r.cx.update(|cx| r.host.request_window_close(cx, r.t[1]));
        r.settle();
        assert_eq!(r.log.borrow().closes, [r.t[2], r.t[0], r.t[1]], "the selected tab first, then in bar order");
        assert_eq!(r.group(r.t[0]).tabs, [r.t[0]], "the held tab stays");
        r.cx.update(|cx| r.host.update(cx, |app| app.close_window(r.t[0])));
        r.settle();
        assert_eq!(r.host.window_handle(r.t[0]), None, "the app closing a held tab closes it");
    }

    pub fn gpui_window_tabs_add_asks_for_a_new_tab() {
        let mut r = open(2, 0, false);
        let add = r.group(r.t[0]).layout.add.expect("the + shows for a window that handles new tabs");
        r.click(r.t[1], center(&add));
        assert_eq!(r.log.borrow().new_tabs, [r.t[1]], "the selected tab's handler ran");
        let new = *r.log.borrow().opened.last().expect("opened");
        let g = r.group(r.t[0]);
        assert_eq!(g.tabs, [r.t[0], r.t[1], new]);
        assert_eq!(g.selected, new, "the new tab is selected");
        assert_eq!(r.host.window_handle(new), Some(r.handle(r.t[0])));
    }

    pub fn gpui_window_tabs_drag_reorders() {
        let mut r = open(3, 0, false);
        let from = center(&r.tab(r.t[0], r.t[0]));
        let last = r.tab(r.t[0], r.t[2]);
        let to = Point { x: last.origin.x + last.size.width - 4., y: from.y };
        r.drag(r.t[2], from, to);
        let g = r.group(r.t[0]);
        assert_eq!(g.tabs, [r.t[1], r.t[2], r.t[0]], "the dragged tab moved to the end");
        assert_eq!(g.selected, r.t[0]);
        assert_eq!(r.handle(r.t[0]), r.handle(r.t[1]), "a reorder opens no window");
    }

    pub fn gpui_window_tabs_tear_off_and_drop() {
        let mut r = open(3, 2, false);
        let (t1, t2, t3) = (r.t[0], r.t[1], r.t[2]);
        let source = r.handle(t1);
        let page = r.page(t2, 2).map(|p| p.id);
        assert!(page.is_some(), "a hidden tab keeps its tree");
        let detail = by_test_id(&r.host, t2, "detail-2").map(|d| d.id);
        // Tear t2 out: dragged well below the bar and released inside the window.
        let from = center(&r.tab(t1, t2));
        let bar = r.group(t1).layout.bar.expect("bar");
        let to = Point { x: from.x, y: bar.origin.y + bar.size.height + 200. };
        r.drag(t1, from, to);
        let torn = r.handle(t2);
        assert_ne!(torn, source, "the torn tab has its own GPUI window");
        assert_eq!(r.group(t2).tabs, [t2], "with the same nd window");
        assert_eq!(r.group(t2).group, "t", "in the same group");
        assert_eq!(r.group(t1).tabs, [t1, t3]);
        assert_eq!(r.page(t2, 2).map(|p| p.id), page, "the content's node is the same node");
        assert_eq!(by_test_id(&r.host, t2, "detail-2").map(|d| d.id), detail, "and the nodes under it");
        let torn_at = r.bounds(t2);
        let source_at = r.bounds(t1);
        assert!(torn_at.origin.y > source_at.origin.y + 100., "it opened at the release point: {torn_at:?}");

        // A drop on another group's bar is refused.
        let u_bar = r.group(r.u[0]).layout.bar.expect("u bar");
        let u_at = r.bounds(r.u[0]);
        let global =
            Point { x: u_at.origin.x + u_bar.origin.x + u_bar.size.width - 20., y: u_at.origin.y + center(&u_bar).y };
        let local = Point { x: global.x - source_at.origin.x, y: global.y - source_at.origin.y };
        assert!(
            local.y > bar.size.height * 2. || local.x > source_at.size.width,
            "the drop point is off the source bar"
        );
        let windows = r.cx.update(|cx| cx.windows().len());
        r.drag(t1, center(&r.tab(t1, t3)), local);
        assert_eq!(r.group(t1).tabs, [t1, t3], "a tab never joins another group");
        assert_eq!(r.group(r.u[0]).tabs, r.u);
        assert_eq!(r.cx.update(|cx| cx.windows().len()), windows, "and no window opened");

        // A new tab of the group joins the window used last, here the torn one, so its bar shows.
        let log = r.log.clone();
        let t4 = r.cx.update(|cx| {
            r.host.update(cx, |app| {
                app.activate_window(t2);
                app.flush_effects();
                open_tab(app, "t", 4, (640., 480.), &log)
            })
        });
        r.settle();
        assert_eq!(r.group(t4).tabs, [t2, t4]);
        // Drop t3 on that bar: it moves there.
        let torn_bar = r.group(t2).layout.bar.expect("torn bar");
        let torn_at = r.bounds(t2);
        let global = Point { x: torn_at.origin.x + torn_bar.origin.x + 8., y: torn_at.origin.y + center(&torn_bar).y };
        let local = Point { x: global.x - source_at.origin.x, y: global.y - source_at.origin.y };
        let page3 = r.page(t3, 3).map(|p| p.id);
        r.drag(t1, center(&r.tab(t1, t3)), local);
        assert_eq!(r.group(t3).tabs, [t3, t2, t4], "dropped at the bar's start");
        assert_eq!(r.group(t3).selected, t3);
        assert_eq!(r.handle(t3), r.handle(t2));
        assert_eq!(r.group(t1).tabs, [t1]);
        assert_eq!(r.page(t3, 3).map(|p| p.id), page3, "the moved tab keeps its nodes");
    }

    pub fn gpui_window_tabs_overview() {
        let mut r = open(3, 0, false);
        r.cx.update(|cx| r.host.update(cx, |app| app.window_command(r.t[0], WindowCommand::ToggleTabOverview)));
        r.settle();
        let g = r.group(r.t[0]);
        assert!(g.overview);
        assert_eq!(g.layout.cards.len(), 3);
        r.click(r.t[2], center(&g.layout.cards[1]));
        let g = r.group(r.t[0]);
        assert!(!g.overview, "picking a card closes the overview");
        assert_eq!(g.selected, r.t[1]);
        let button = g.layout.overview.expect("overview button");
        r.click(r.t[1], center(&button));
        assert!(r.group(r.t[0]).overview, "the bar's button opens it");
        r.input(r.t[1], vec![SyntheticInput::Key(Keystroke::parse("escape").expect("key"))]);
        assert!(!r.group(r.t[0]).overview, "Esc closes it");
        assert_eq!(r.group(r.t[0]).selected, r.t[1]);
    }

    pub fn gpui_window_tabs_single_canvas() {
        let mut r = open(2, 1, true);
        let handle = r.handle(r.t[0]);
        assert_eq!(r.handle(r.u[0]), handle, "one canvas");
        assert_eq!(r.group(r.t[0]).tabs, r.t);
        assert!(r.group(r.t[0]).layout.bar.is_some(), "the group's in-canvas window draws its bar");
        let first = center(&r.tab(r.t[0], r.t[0]));
        r.click(r.t[1], first);
        assert_eq!(r.group(r.t[0]).selected, r.t[0]);
        assert!(r.page(r.t[0], 1).and_then(|p| p.bounds).is_some());
    }

    pub fn gpui_window_tabs_view_moves_between_windows() {
        let mut cx = headless();
        type Slot = Rc<RefCell<Option<(Entity<Shelf>, Entity<Shelf>, WindowId, WindowId)>>>;
        let slot: Slot = Rc::default();
        let out = slot.clone();
        let host = cx.update(|cx| {
            GpuiHost::builder().quit_when_empty(false).start(cx, move |app| {
                let a = app.open_window(WindowOptions::main("a").size(gpui::size(px(400.), px(300.))), |_, cx| {
                    let page = cx.new(|_| Page { n: 7 });
                    cx.new(|_| Shelf { page: Some(page) })
                });
                let b = app.open_window(WindowOptions::main("b").size(gpui::size(px(400.), px(300.))), |_, cx| {
                    cx.new(|_| Shelf { page: None })
                });
                *out.borrow_mut() = Some((a.root().clone(), b.root().clone(), a.id(), b.id()));
            })
        });
        settle(&mut cx, &host);
        let (a, b, wa, wb) = slot.borrow_mut().take().expect("windows");
        let before = by_test_id(&host, wa, "page-7").expect("page in a");
        let detail = by_test_id(&host, wa, "detail-7").expect("detail in a").id;
        let live = host.backend().live_count();
        cx.update(|cx| {
            host.update(cx, |app| {
                let page = a.update(app, |s, cx| {
                    cx.notify();
                    s.page.take()
                });
                b.update(app, |s, cx| {
                    s.page = page;
                    cx.notify();
                });
            })
        });
        settle(&mut cx, &host);
        assert!(by_test_id(&host, wa, "page-7").is_none(), "the source window let it go");
        let after = by_test_id(&host, wb, "page-7").expect("page in b");
        assert_eq!(after.id, before.id, "the page keeps its node");
        assert_eq!(by_test_id(&host, wb, "detail-7").map(|d| d.id), Some(detail));
        assert_eq!(host.backend().live_count(), live, "nothing created or released");
        let bounds = host.backend().bounds(after.id).expect("drawn in b");
        assert!(bounds.size.width > 0., "it rendered in the target window");
        assert_eq!(after.bounds, Some(bounds), "at the bounds the target window laid it out at");
    }

    pub fn gpui_tab_strip_both_presentations() {
        for presentation in [TabStyle::Strip, TabStyle::Automatic] {
            let mut s = strip(presentation);
            assert_eq!(s.node().value.as_deref(), Some("Alpha"));
            let tabs = s.layout().tabs;
            assert_eq!(tabs.len(), 3, "{presentation:?}");
            s.click(center(&tabs[1]));
            assert_eq!(s.take(), ["select b"], "{presentation:?}: one click, one event");
            assert_eq!(s.node().value.as_deref(), Some("Beta"));
            s.click(center(&s.layout().tabs[1]));
            assert_eq!(s.take(), Vec::<String>::new(), "the selected tab reports nothing");
            s.state.borrow_mut().accept = false;
            s.click(center(&s.layout().tabs[2]));
            assert_eq!(s.take(), ["select c"]);
            assert_eq!(s.node().value.as_deref(), Some("Beta"), "a refused change shows the old selection");
            s.state.borrow_mut().accept = true;
            let node = s.node().id;
            s.cx.update(|cx| s.host.update(cx, |app| app.perform(node, &SemanticAction::SetValue("Gamma".into()))))
                .expect("set");
            s.settle();
            assert_eq!(s.take(), ["select c"], "SetValue reports the change");
            assert_eq!(s.node().value.as_deref(), Some("Gamma"));
            if presentation == TabStyle::Strip {
                let layout = s.layout();
                assert!(layout.closes[0].is_none(), "Alpha is not closable");
                s.click(center(&layout.closes[1].expect("Beta closes")));
                assert_eq!(s.take(), ["close b"]);
                s.click(center(&layout.add.expect("+")));
                assert_eq!(s.take(), ["add"]);
            } else {
                assert!(s.layout().add.is_none(), "a view switcher has no +");
            }
        }
    }

    pub fn gpui_tab_goldens() {
        // GPUI renders headless only on macOS (Metal) and Linux (wgpu); Windows captures arrive
        // with the Fluent goldens (M3).
        if nd_gpui::gpui_platform::current_headless_renderer().ok().flatten().is_none() {
            println!("skipped: GPUI has no headless renderer on this OS");
            return;
        }
        let mut failed = Vec::new();
        for scheme in [Scheme::Light, Scheme::Dark] {
            for view in [GoldenView::Bar, GoldenView::Overview, GoldenView::Strip, GoldenView::Switcher] {
                if let Err(e) = golden(view, scheme) {
                    failed.push(e);
                }
            }
        }
        assert!(failed.is_empty(), "{failed:#?}");
    }
}

struct Shelf {
    page: Option<Entity<Page>>,
}

impl Render for Shelf {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        v_flex().size_full().child(text("shelf")).children(self.page.clone())
    }
}

struct StripState {
    selected: SharedString,
    accept: bool,
    log: Vec<String>,
}

struct StripView {
    presentation: TabStyle,
    state: Rc<RefCell<StripState>>,
}

impl Render for StripView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tabs = [("a", "Alpha", false), ("b", "Beta", true), ("c", "Gamma", true)]
            .map(|(id, title, closable)| TabSpec { id: id.into(), title: title.into(), icon: None, closable });
        let (select, close, add) = (self.state.clone(), self.state.clone(), self.state.clone());
        v_flex().size_full().child(
            tab_strip("strip")
                .tabs(tabs)
                .selected(self.state.borrow().selected.clone())
                .presentation(self.presentation)
                .add_button(true)
                .label("Sections")
                .on_select(cx.listener(move |_, e: &TabChanged, _, cx| {
                    let mut s = select.borrow_mut();
                    s.log.push(format!("select {}", e.id));
                    if s.accept {
                        s.selected = e.id.clone();
                    }
                    cx.notify();
                }))
                .on_close(move |e: &TabCloseRequested, _, _| close.borrow_mut().log.push(format!("close {}", e.id)))
                .on_add(move |_: &TabAddRequested, _, _| add.borrow_mut().log.push("add".into())),
        )
    }
}

struct StripRig {
    cx: HeadlessAppContext,
    host: GpuiHost,
    window: WindowId,
    state: Rc<RefCell<StripState>>,
}

fn strip(presentation: TabStyle) -> StripRig {
    let mut cx = headless();
    let state = Rc::new(RefCell::new(StripState { selected: "a".into(), accept: true, log: Vec::new() }));
    let (s, slot) = (state.clone(), Rc::new(RefCell::new(None)));
    let out = slot.clone();
    let host = cx.update(|cx| {
        GpuiHost::builder().quit_when_empty(false).start(cx, move |app| {
            let w = app.open_window(WindowOptions::main("strip").size(gpui::size(px(640.), px(240.))), |_, cx| {
                cx.new(|_| StripView { presentation, state: s })
            });
            *out.borrow_mut() = Some(w.id());
        })
    });
    settle(&mut cx, &host);
    let window = slot.borrow_mut().take().expect("window");
    StripRig { cx, host, window, state }
}

impl StripRig {
    fn settle(&mut self) {
        settle(&mut self.cx, &self.host);
    }

    fn node(&self) -> SemanticNode {
        by_test_id(&self.host, self.window, "strip").expect("strip")
    }

    fn layout(&self) -> nd_backend_gpui::tab_bar::BarLayout {
        self.host.backend().tab_strip_layout(self.node().id).expect("drawn")
    }

    fn click(&mut self, at: Point) {
        let window = self.window;
        self.cx.update(|cx| {
            self.host.update(cx, |app| {
                for phase in [PointerPhase::Down, PointerPhase::Up] {
                    let input = SyntheticInput::Pointer { phase, at, button: MouseButton::Left, click_count: 1 };
                    app.synthesize(window, &input).expect("posted");
                }
            })
        });
        self.settle();
    }

    fn take(&self) -> Vec<String> {
        std::mem::take(&mut self.state.borrow_mut().log)
    }
}

#[derive(Clone, Copy, Debug)]
enum GoldenView {
    /// A group window's bar: three tabs, the `+` and the overview button.
    Bar,
    Overview,
    /// `tab_strip` as document tabs.
    Strip,
    /// `tab_strip` as a view switcher.
    Switcher,
}

struct Golden {
    view: GoldenView,
    scheme: Scheme,
}

fn golden_tabs() -> Vec<BarTab> {
    ["Inbox", "Q3 planning notes", "Trips"].map(|t| BarTab { title: t.into(), closable: true }).to_vec()
}

impl gpui::Render for Golden {
    fn render(&mut self, _window: &mut gpui::Window, _cx: &mut gpui::Context<Self>) -> impl gpui::IntoElement {
        use gpui::{IntoElement, ParentElement, Styled};
        let theme = theme_for(self.scheme);
        let marks = Marks::default();
        let mut bar = Bar {
            tabs: golden_tabs(),
            selected: Some(1),
            add: true,
            overview: true,
            enabled: true,
            insets: (0., 0.),
            drag_region: false,
            label: None,
        };
        let content = match self.view {
            GoldenView::Bar => tab_bar_view("golden", &bar, &theme, None, None, &marks),
            GoldenView::Overview => gpui::div()
                .relative()
                .size_full()
                .child(overview_view("golden", &bar.tabs, Some(1), &theme, None, None, &marks))
                .into_any_element(),
            GoldenView::Strip => {
                bar.overview = false;
                bar.tabs[0].closable = false;
                tab_bar_view("golden", &bar, &theme, None, None, &marks)
            }
            GoldenView::Switcher => {
                bar.tabs =
                    ["General", "Appearance", "Accounts"].map(|t| BarTab { title: t.into(), closable: false }).to_vec();
                gpui::div()
                    .flex()
                    .justify_center()
                    .pt(gpui::px(24.))
                    .child(switcher_view("golden", &bar, &theme, None, None, &marks))
                    .into_any_element()
            }
        };
        gpui::div()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.surface.base)
            .text_color(theme.text.primary)
            .child(content)
            .into_any_element()
    }
}

fn golden_size(view: GoldenView) -> (f32, f32) {
    match view {
        GoldenView::Overview => (800., 420.),
        _ => (640., 96.),
    }
}

fn golden_path(view: GoldenView, scheme: Scheme) -> PathBuf {
    let view = match view {
        GoldenView::Bar => "bar",
        GoldenView::Overview => "overview",
        GoldenView::Strip => "strip",
        GoldenView::Switcher => "switcher",
    };
    let scheme = if scheme.is_dark() { "dark" } else { "light" };
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(format!("tests/goldens/{}/tabs-{view}-{scheme}.png", std::env::consts::OS))
}

/// Per channel, out of 255, and the share of pixels allowed past it: antialiasing drifts.
const CHANNEL_TOLERANCE: i16 = 24;
const PIXEL_TOLERANCE: f64 = 0.004;

fn golden(view: GoldenView, scheme: Scheme) -> Result<(), String> {
    use gpui::AppContext as _;
    let mut cx = headless();
    let (w, h) = golden_size(view);
    let handle = cx
        .open_window(gpui::size(gpui::px(w), gpui::px(h)), |_, cx| cx.new(|_| Golden { view, scheme }))
        .map_err(|e| e.to_string())?;
    cx.run_until_parked();
    let image = cx.capture_screenshot(handle.into()).map_err(|e| e.to_string())?;
    let path = golden_path(view, scheme);
    if std::env::var_os("ND_UPDATE_GOLDENS").is_some_and(|v| v == "1") {
        std::fs::create_dir_all(path.parent().expect("dir")).map_err(|e| e.to_string())?;
        image.save(&path).map_err(|e| e.to_string())?;
        println!("recorded {}", path.display());
        return Ok(());
    }
    if !path.exists() {
        return Err(format!("{} is missing (record it with ND_UPDATE_GOLDENS=1)", path.display()));
    }
    let expected = image::open(&path).map_err(|e| e.to_string())?.to_rgba8();
    let actual_path = path.with_extension("actual.png");
    if expected.dimensions() != image.dimensions() {
        let _ = image.save(&actual_path);
        return Err(format!("{}: size {:?}, golden {:?}", path.display(), image.dimensions(), expected.dimensions()));
    }
    let off = expected
        .pixels()
        .zip(image.pixels())
        .filter(|(a, b)| a.0.iter().zip(b.0.iter()).any(|(x, y)| (*x as i16 - *y as i16).abs() > CHANNEL_TOLERANCE))
        .count();
    let share = off as f64 / (image.width() * image.height()).max(1) as f64;
    if share > PIXEL_TOLERANCE {
        let _ = image.save(&actual_path);
        return Err(format!(
            "{}: {:.2}% of pixels differ (see {})",
            path.display(),
            share * 100.,
            actual_path.display()
        ));
    }
    let _ = std::fs::remove_file(&actual_path);
    println!("ok   {} ({:.3}% off)", path.display(), share * 100.);
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let tests: Vec<(&str, fn())> = vec![
        ("gate_m6::gpui_window_tabs_group_by_name", gate_m6::gpui_window_tabs_group_by_name),
        ("gate_m6::gpui_window_tabs_click_selects", gate_m6::gpui_window_tabs_click_selects),
        ("gate_m6::gpui_window_tabs_close_holds_or_closes", gate_m6::gpui_window_tabs_close_holds_or_closes),
        (
            "gate_m6::gpui_window_tabs_window_close_asks_every_tab",
            gate_m6::gpui_window_tabs_window_close_asks_every_tab,
        ),
        ("gate_m6::gpui_window_tabs_add_asks_for_a_new_tab", gate_m6::gpui_window_tabs_add_asks_for_a_new_tab),
        ("gate_m6::gpui_window_tabs_drag_reorders", gate_m6::gpui_window_tabs_drag_reorders),
        ("gate_m6::gpui_window_tabs_tear_off_and_drop", gate_m6::gpui_window_tabs_tear_off_and_drop),
        ("gate_m6::gpui_window_tabs_overview", gate_m6::gpui_window_tabs_overview),
        ("gate_m6::gpui_window_tabs_single_canvas", gate_m6::gpui_window_tabs_single_canvas),
        ("gate_m6::gpui_window_tabs_view_moves_between_windows", gate_m6::gpui_window_tabs_view_moves_between_windows),
        ("gate_m6::gpui_tab_strip_both_presentations", gate_m6::gpui_tab_strip_both_presentations),
        ("gate_m6::gpui_tab_goldens", gate_m6::gpui_tab_goldens),
    ];
    let has = |flag: &str| args.iter().any(|a| a == flag);
    if has("--list") {
        if !has("--ignored") {
            for (name, _) in &tests {
                println!("{name}: test");
            }
        }
        return;
    }
    if has("--ignored") {
        return;
    }
    let exact = has("--exact");
    let mut skip = false;
    let filters: Vec<&String> = args
        .iter()
        .filter(|a| {
            if std::mem::take(&mut skip) {
                return false;
            }
            if matches!(a.as_str(), "--format" | "--test-threads" | "--color" | "--skip" | "-Z") {
                skip = true;
            }
            !a.starts_with('-')
        })
        .collect();
    let mut failed = 0;
    for (name, test) in &tests {
        let selected =
            filters.is_empty() || filters.iter().any(|f| if exact { *f == name } else { name.contains(f.as_str()) });
        if !selected {
            continue;
        }
        match std::panic::catch_unwind(test) {
            Ok(()) => println!("test {name} ... ok"),
            Err(_) => {
                println!("test {name} ... FAILED");
                failed += 1;
            }
        }
    }
    if failed > 0 {
        std::process::exit(101);
    }
}
