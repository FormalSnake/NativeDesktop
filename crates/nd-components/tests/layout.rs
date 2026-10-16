//! Tier 1: in-window tabs, splits, the pane views, status pages and the alert queue on
//! `TestBackend`. Automation's `setValue` stands in for the user, through the same
//! `WidgetSemantics` answer the drawn backends emit.

use std::cell::RefCell;
use std::rc::Rc;

use nd_components::panes::{self, Axis, PaneTree};
use nd_components::services::{AlertCallback, OpenCallback, SaveCallback};
use nd_components::*;
use nd_core::semantic::{Role, SemanticNode};
use nd_ui::backend::{NodeKind, WidgetKind};
use nd_ui::prelude::*;
use nd_ui::semantics::SemanticAction;
use nd_ui::test_app::TestApp;
use nd_ui::transaction::Op;
use nd_ui::{NodeId, Size, WindowId};

const WINDOW: Size = Size { width: 400., height: 300. };

fn node_of<W: 'static>(cx: &TestApp) -> NodeId {
    cx.backend().find(|kind, _| kind == NodeKind::Widget(WidgetKind::of::<W>()))[0]
}

fn tree(cx: &mut TestApp, window: WindowId) -> SemanticNode {
    cx.run_until_idle();
    cx.semantic_window(window).expect("open").root
}

fn by_test_id<'a>(root: &'a SemanticNode, id: &str) -> &'a SemanticNode {
    root.iter().find(|n| n.test_id.as_deref() == Some(id)).unwrap_or_else(|| panic!("no {id}"))
}

fn set_value(cx: &mut TestApp, node: NodeId, value: &str) {
    cx.perform(node, &SemanticAction::SetValue(value.into())).expect("performed");
    cx.run_until_idle();
}

struct Tabs {
    selected: SharedString,
    picked: Vec<SharedString>,
    homogeneous: bool,
    refuse: bool,
}

impl Render for Tabs {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        tab_view("settings")
            .label("Settings")
            .selected(self.selected.clone())
            .homogeneous(self.homogeneous)
            .tab(tab_page("general", "General").child(div().id("general-body").h(px(40.))))
            .tab(tab_page("advanced", "Advanced").child(div().id("advanced-body").h(px(120.))))
            .on_select(cx.listener(|this: &mut Tabs, e: &TabChanged, _, cx| {
                this.picked.push(e.id.clone());
                if !this.refuse {
                    this.selected = e.id.clone();
                }
                cx.notify();
            }))
    }
}

fn tabs(cx: &mut TestApp, homogeneous: bool) -> WindowHandle<Tabs> {
    cx.open_window(WINDOW, |_, cx| {
        cx.new(|_| Tabs { selected: "general".into(), picked: Vec::new(), homogeneous, refuse: false })
    })
}

#[test]
fn tab_view_shows_the_selected_page_and_measures_as_the_tallest() {
    let mut cx = TestApp::new();
    let window = tabs(&mut cx, true);
    let root = tree(&mut cx, window.id());
    let strip = by_test_id(&root, "settings");
    assert_eq!(strip.role, Role::TabList);
    assert_eq!(strip.name, "Settings");
    assert_eq!(strip.value.as_deref(), Some("General"));
    let general = by_test_id(&root, "general");
    let advanced = by_test_id(&root, "advanced");
    assert_eq!((general.role, general.name.as_str()), (Role::TabPanel, "General"));
    assert!(general.visible && !advanced.visible, "only the selected page shows");
    let stack = |root: &SemanticNode| by_test_id(root, "general").bounds.expect("laid out").size.height;
    assert_eq!(stack(&root), 120., "the page stack is as tall as the tallest page");

    let pages = (cx.backend().live_count(), by_test_id(&root, "advanced").id);
    let strip = node_of::<TabStripKind>(&cx);
    set_value(&mut cx, strip, "Advanced");
    let root = tree(&mut cx, window.id());
    assert_eq!(window.root().read(&cx).picked, vec![SharedString::from("advanced")]);
    assert_eq!(by_test_id(&root, "settings").value.as_deref(), Some("Advanced"));
    assert!(by_test_id(&root, "advanced").visible && !by_test_id(&root, "general").visible);
    assert_eq!(stack(&root), 120., "switching moves nothing below the strip");
    assert_eq!((cx.backend().live_count(), by_test_id(&root, "advanced").id), pages, "pages stay mounted");

    // By id and by index too; an unknown tab is refused.
    set_value(&mut cx, strip, "general");
    assert_eq!(window.root().read(&cx).selected.as_ref(), "general");
    set_value(&mut cx, strip, "1");
    assert_eq!(window.root().read(&cx).selected.as_ref(), "advanced");
    assert!(cx.perform(strip, &SemanticAction::SetValue("Nope".into())).is_err());
}

#[test]
fn a_refused_tab_change_keeps_the_old_selection() {
    let mut cx = TestApp::new();
    let window = tabs(&mut cx, true);
    window.root().update(&mut cx, |t, _| t.refuse = true);
    let strip = node_of::<TabStripKind>(&cx);
    cx.backend_mut().take_log();
    set_value(&mut cx, strip, "Advanced");
    assert_eq!(window.root().read(&cx).picked.len(), 1);
    let root = tree(&mut cx, window.id());
    assert_eq!(by_test_id(&root, "settings").value.as_deref(), Some("General"));
    let reaffirmed = cx.backend().ops().any(|op| matches!(op, Op::Reaffirm { node } if *node == strip));
    assert!(reaffirmed, "the strip shows the app's selection again");
}

#[test]
fn a_tab_view_that_is_not_homogeneous_mounts_only_the_selected_page() {
    let mut cx = TestApp::new();
    let window = tabs(&mut cx, false);
    let root = tree(&mut cx, window.id());
    assert!(root.iter().all(|n| n.test_id.as_deref() != Some("advanced")));
    assert_eq!(by_test_id(&root, "general").bounds.unwrap().size.height, 40.);
}

struct Splits {
    tree: PaneTree<&'static str>,
    resizes: u32,
}

impl Render for Splits {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        panes::pane_tree("panes", &self.tree, |leaf, focused| {
            let name = if focused { format!("leaf-{}-focused", leaf.data) } else { format!("leaf-{}", leaf.data) };
            div().id(name).size_full().into_any_element()
        })
        .on_resize(cx.listener(|this: &mut Splits, e: &panes::PaneResized, _, cx| {
            this.resizes += 1;
            let next = panes::set_pane_ratio(&this.tree, &e.split, e.ratio);
            if !next.same(&this.tree) {
                this.tree = next;
                cx.notify();
            }
        }))
    }
}

#[test]
fn a_pane_tree_renders_nested_splits_and_a_divider_echo_changes_nothing() {
    let mut cx = TestApp::new();
    let seeded = panes::seed_panes(vec!["a", "b"]);
    let leaf = panes::pane_id_at(&seeded, 0).unwrap();
    let tree = panes::split_pane(&seeded, &leaf, Axis::Vertical, "c", None);
    let window = cx.open_window(WINDOW, move |_, cx| cx.new(|_| Splits { tree, resizes: 0 }));
    let root = tree_of(&mut cx, &window);
    let splits: Vec<&SemanticNode> = root.iter().filter(|n| n.role == Role::SplitGroup).collect();
    assert_eq!(splits.len(), 2, "two splits for three leaves");
    let leaves: Vec<&str> =
        root.iter().filter_map(|n| n.test_id.as_deref()).filter(|t| t.starts_with("leaf-")).collect();
    assert_eq!(leaves.len(), 3, "{leaves:?}");
    assert_eq!(leaves.iter().filter(|t| t.ends_with("-focused")).count(), 1, "one leaf has focus: {leaves:?}");
    // The outer split lays its panes out by the ratio.
    let outer = splits[0];
    let first = &outer.children[0];
    assert_eq!(first.bounds.unwrap().size.width, WINDOW.width * 0.5);

    let nodes = cx.backend().find(|kind, _| kind == NodeKind::Widget(WidgetKind::of::<SplitKind>()));
    let outer_node = nodes[0];
    set_value(&mut cx, outer_node, "0.25");
    let root = tree_of(&mut cx, &window);
    let outer = root.iter().find(|n| n.id == outer_node).unwrap();
    assert_eq!(outer.value.as_deref(), Some("0.250"));
    assert_eq!(outer.children[0].bounds.unwrap().size.width, WINDOW.width * 0.25);

    // The same ratio again is an echo: the model answers with the same tree, nothing renders.
    let before = window.root().read(&cx).tree.clone();
    cx.backend_mut().take_log();
    set_value(&mut cx, outer_node, "0.25");
    assert!(window.root().read(&cx).tree.same(&before));
    assert_eq!(window.root().read(&cx).resizes, 2);
    assert!(!cx.backend().ops().any(|op| matches!(op, Op::Update { .. })), "no prop reaches the backend twice");
}

fn tree_of<V: Render>(cx: &mut TestApp, window: &WindowHandle<V>) -> SemanticNode {
    tree(cx, window.id())
}

#[test]
fn split_ratios_parse_as_fractions_or_percent() {
    let props = SplitProps { axis: Axis::Horizontal, ratio: 0.5, label: None, style: Default::default() };
    let ratio = |v: &str| {
        let r = nd_ui::semantics::WidgetSemantics::respond(&props, &SemanticAction::SetValue(v.into())).unwrap();
        r.ok().and_then(|e| e.downcast_ref::<SplitResized>().map(|e| e.ratio))
    };
    assert_eq!(ratio("0.3"), Some(0.3));
    assert_eq!(ratio("30%"), Some(0.3));
    assert_eq!(ratio("75"), Some(0.75));
    assert_eq!(ratio("2.5"), Some(0.025));
    assert_eq!(ratio("wide"), None);
}

struct Empty;

impl Render for Empty {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        status_page("No Results")
            .id("empty")
            .icon(Icon::Search)
            .description("Try a different search.")
            .child(button("clear", "Clear Search"))
    }
}

#[test]
fn a_status_page_is_a_status_with_its_actions_under_it() {
    let mut cx = TestApp::new();
    let window = cx.open_window(WINDOW, |_, cx| cx.new(|_| Empty));
    let root = tree(&mut cx, window.id());
    let header = by_test_id(&root, "empty");
    assert_eq!((header.role, header.name.as_str()), (Role::Status, "No Results"));
    assert_eq!(header.value.as_deref(), Some("Try a different search."));
    let action = root.iter().find(|n| n.role == Role::Button).expect("the action");
    assert_eq!(action.name, "Clear Search");
    assert!(action.bounds.unwrap().origin.y > header.bounds.unwrap().origin.y, "actions sit under the text");
}

/// A platform that records what it was asked to show and answers when the test says.
#[derive(Default)]
struct FakeChrome {
    shown: RefCell<Vec<(WindowId, SharedString)>>,
    pending: RefCell<Vec<AlertCallback>>,
    abouts: RefCell<Vec<AboutPanel>>,
}

impl ChromeServices for FakeChrome {
    fn set_menus(&self, _cx: &mut App, _menus: Vec<Menu>) {}
    fn show_context_menu(&self, _: &mut App, _: WindowId, _: NodeId, _: f32, _: f32, _: ContextMenu) {}
    fn alert(&self, _cx: &mut App, window: WindowId, alert: Alert, done: AlertCallback) {
        self.shown.borrow_mut().push((window, alert.title));
        self.pending.borrow_mut().push(done);
    }
    fn open_dialog(&self, cx: &mut App, _: WindowId, _: OpenDialog, done: OpenCallback) {
        done(None, cx);
    }
    fn save_dialog(&self, cx: &mut App, _: WindowId, _: SaveDialog, done: SaveCallback) {
        done(None, cx);
    }
    fn about(&self, _cx: &mut App, _window: Option<WindowId>, about: AboutPanel) {
        self.abouts.borrow_mut().push(about);
    }
}

#[test]
fn alerts_wait_their_turn_per_window_and_each_answer_reaches_its_own_callback() {
    let mut cx = TestApp::new();
    let chrome = Rc::new(FakeChrome::default());
    cx.set_global(Chrome(chrome.clone()));
    let a = cx.open_window(WINDOW, |_, cx| cx.new(|_| Empty)).id();
    let b = cx.open_window(WINDOW, |_, cx| cx.new(|_| Empty)).id();
    let answers = Rc::new(RefCell::new(Vec::new()));
    let ask = |cx: &mut TestApp, window, alert: Alert| {
        let answers = answers.clone();
        let title = alert.title.clone();
        services::alert(cx, window, alert, move |r, _| answers.borrow_mut().push((title, r.0)));
    };
    ask(&mut cx, a, Alert::confirm("Delete note?", "Delete"));
    ask(&mut cx, a, Alert::new("Second on a"));
    ask(&mut cx, b, Alert::new("First on b"));
    ask(&mut cx, a, Alert::new("Quit?").modal());
    let titles = |c: &FakeChrome| c.shown.borrow().iter().map(|(_, t)| t.to_string()).collect::<Vec<_>>();
    assert_eq!(titles(&chrome), ["Delete note?", "First on b", "Quit?"], "one sheet per window, modal apart");

    let first = chrome.pending.borrow_mut().remove(0);
    first(AlertResponse(0), &mut cx);
    assert_eq!(answers.borrow()[0], (SharedString::from("Delete note?"), 0));
    assert_eq!(titles(&chrome).last().map(String::as_str), Some("Second on a"), "the next one on a shows");

    // A queued alert whose window closed answers as dismissed.
    ask(&mut cx, b, Alert::new("Orphan").cancel("Never mind").button("Go"));
    cx.close_window(b);
    let first_on_b = chrome.pending.borrow_mut().remove(0);
    first_on_b(AlertResponse(0), &mut cx);
    assert!(answers.borrow().contains(&(SharedString::from("Orphan"), 0)), "{:?}", answers.borrow());

    let confirm = Alert::confirm("Delete?", "Delete");
    assert_eq!(confirm.buttons[0].role, ButtonRole::Destructive);
    assert_eq!(confirm.cancel_index(), 1);

    services::about(&mut cx, Some(a), AboutPanel::new().name("Notes").version("1.0"));
    assert_eq!(chrome.abouts.borrow()[0].name.as_deref(), Some("Notes"));
}

struct TitleRow {
    title: Option<&'static str>,
    leading: bool,
}

impl Render for TitleRow {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let row = titlebar().trailing(button("share", "Share"));
        let row = match self.title {
            Some(title) => row.title(title),
            None => row,
        };
        let row = if self.leading { row.leading(button("back", "Back")) } else { row };
        v_flex().size_full().child(row)
    }
}

fn texts(cx: &mut TestApp, window: WindowId) -> Vec<String> {
    tree(cx, window).iter().filter(|n| n.kind == "text").map(|n| n.name.clone()).collect()
}

fn title_row(title: Option<&'static str>, leading: bool) -> Vec<String> {
    let mut cx = TestApp::new();
    let options = WindowOptions::main("mail").title("Mail").titlebar(nd_ui::Titlebar::Custom);
    let handle = cx.open_window_with(options, |_, cx| cx.new(|_| TitleRow { title, leading }));
    texts(&mut cx, handle.id())
}

#[test]
fn a_title_row_with_nothing_leading_shows_the_window_title() {
    assert_eq!(title_row(None, false), ["Mail"]);
    // The app's own title replaces it, and a leading item takes the space instead.
    assert_eq!(title_row(Some("Inbox"), false), ["Inbox"]);
    assert!(title_row(None, true).is_empty());
}
