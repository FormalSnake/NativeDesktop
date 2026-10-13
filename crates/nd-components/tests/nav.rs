//! Tier 1: the command palette, source tree, toasts and banner on `TestBackend`. A backend hook
//! stands in for the native side with the same shared models the real backends use, so the
//! checks cover the app round trip: events reach handlers outside every lease, a handler can
//! set the widget's props from inside its own event (reentrancy), and a value the app feeds
//! back never reaches the native side as a second set (echo).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use nd_components::*;
use nd_core::semantic::Role;
use nd_ui::backend::{NodeKind, WidgetKind};
use nd_ui::prelude::*;
use nd_ui::test_app::TestApp;
use nd_ui::test_backend::TestBackend;
use nd_ui::{NodeId, Size};

const WINDOW: Size = Size { width: 800., height: 600. };

fn read<V: Render, R>(cx: &TestApp, handle: &WindowHandle<V>, f: impl FnOnce(&V) -> R) -> R {
    f(handle.root().read(cx))
}

fn update<V: Render>(cx: &mut TestApp, handle: &WindowHandle<V>, f: impl FnOnce(&mut V, &mut Context<V>)) {
    handle.root().update(cx, f);
    cx.run_until_idle();
}

fn kind_is<W: 'static>(kind: NodeKind) -> bool {
    kind == NodeKind::Widget(WidgetKind::of::<W>())
}

fn node_of<W: 'static>(cx: &TestApp) -> NodeId {
    cx.backend().find(|kind, _| kind_is::<W>(kind))[0]
}

/// What the stand-in native palette saw.
#[derive(Default)]
struct NativePalette {
    model: PaletteModel,
    reseeds: u32,
    reloads: u32,
    presents: u32,
    open: bool,
}

struct Palette {
    open: bool,
    query: SharedString,
    queries: Vec<SharedString>,
    activated: Vec<SharedString>,
    submitted: Vec<SharedString>,
    cancels: u32,
}

const NOTES: [&str; 3] = ["Trip to Lisbon", "Groceries", "Trivia night"];

impl Palette {
    fn items(&self) -> Vec<PaletteItem> {
        let q = self.query.to_lowercase();
        NOTES
            .iter()
            .filter(|t| t.to_lowercase().contains(&q))
            .map(|t| {
                let item = palette_item(*t, *t).subtitle("note").hint("Open");
                if !q.is_empty() && t.to_lowercase().starts_with(&q) { item.completion(*t) } else { item }
            })
            .collect()
    }
}

impl Render for Palette {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(
            command_palette("palette")
                .open(self.open)
                .placeholder("Search notes")
                .query(self.query.clone())
                .items(self.items())
                .on_query_change(cx.listener(|this, e: &PaletteQueryChanged, _, cx| {
                    // Fed straight back, as a controlled app does.
                    this.query = e.text.clone();
                    this.queries.push(e.text.clone());
                    cx.notify();
                }))
                .on_activate(cx.listener(|this, e: &PaletteActivated, _, cx| {
                    this.activated.push(e.id.clone());
                    this.open = false;
                    cx.notify();
                }))
                .on_submit(cx.listener(|this, e: &PaletteSubmitted, _, cx| {
                    this.submitted.push(e.text.clone());
                    this.open = false;
                    cx.notify();
                }))
                .on_cancel(cx.listener(|this, _: &PaletteCancelled, _, cx| {
                    this.cancels += 1;
                    this.open = false;
                    cx.notify();
                })),
        )
    }
}

type Natives = Rc<RefCell<HashMap<NodeId, NativePalette>>>;

fn palette_app() -> (TestApp, Natives, WindowHandle<Palette>) {
    let natives: Natives = Rc::default();
    let mut backend = TestBackend::new();
    let seen = natives.clone();
    backend.on_apply(move |node, kind, props, _| {
        if !kind_is::<CommandPaletteKind>(kind) {
            return;
        }
        let p = props.downcast_ref::<CommandPaletteProps>().expect("palette props");
        let mut natives = seen.borrow_mut();
        let n = natives.entry(node).or_default();
        if n.model.set_query(&p.query) {
            n.reseeds += 1;
        }
        if n.model.set_items(&p.items) {
            n.reloads += 1;
        }
        if p.open != n.open {
            n.open = p.open;
            if p.open {
                n.model.present();
                n.presents += 1;
            } else {
                assert_eq!(n.model.dismiss(true), None, "an app-driven close reports nothing");
            }
        }
    });
    let mut cx = TestApp::with_backend(backend);
    let handle = cx.open_window(WINDOW, |_, cx| {
        cx.new(|_| Palette {
            open: true,
            query: "".into(),
            queries: Vec::new(),
            activated: Vec::new(),
            submitted: Vec::new(),
            cancels: 0,
        })
    });
    (cx, natives, handle)
}

/// Runs a user action on the stand-in native palette and delivers what it reports.
fn user(cx: &mut TestApp, natives: &Natives, f: impl FnOnce(&mut PaletteModel) -> Vec<PaletteEvent>) {
    let node = node_of::<CommandPaletteKind>(cx);
    let events = f(&mut natives.borrow_mut().get_mut(&node).expect("native palette").model);
    let emit = cx.backend().events().emitter(node);
    for event in events {
        emit_palette_event(&emit, event);
    }
    cx.run_until_idle();
}

#[test]
fn palette_is_a_dialog_named_by_its_placeholder() {
    let (cx, ..) = palette_app();
    let tree = cx.semantic_tree();
    let (_, palette) = tree.nodes().find(|(_, n)| n.test_id.as_deref() == Some("palette")).expect("palette node");
    assert_eq!((palette.role, palette.name.as_str()), (Role::Dialog, "Search notes"));
    assert_eq!(palette.bounds.map(|b| (b.size.width, b.size.height)), Some((0., 0.)), "takes no space where it sits");
}

#[test]
fn the_search_box_is_a_node_of_its_own_and_takes_typing() {
    let (mut cx, natives, handle) = palette_app();
    user(&mut cx, &natives, |m| m.edited("tri"));
    let tree = cx.semantic_tree();
    let (_, palette) = tree.nodes().find(|(_, n)| n.test_id.as_deref() == Some("palette")).expect("palette node");
    let search = palette.children.iter().find(|n| n.role == Role::SearchBox).expect("search box under the palette");
    assert_eq!(search.test_id.as_deref(), Some("palette.search"));
    assert_eq!(search.value.as_deref(), Some("tri|p to Lisbon"), "the completion shows after the caret");
    let id = search.id;
    cx.perform(id, &nd_ui::semantics::SemanticAction::Type("p".into())).expect("typing into the search box");
    cx.run_until_idle();
    read(&cx, &handle, |p| assert_eq!(p.queries.last().map(|s| s.as_ref()), Some("trip")));
}

#[test]
fn typing_round_trips_and_the_fed_back_query_never_reseeds_the_field() {
    let (mut cx, natives, handle) = palette_app();
    user(&mut cx, &natives, |m| m.edited("tri"));
    let node = node_of::<CommandPaletteKind>(&cx);
    {
        let n = &natives.borrow()[&node];
        assert_eq!(n.reseeds, 0, "the app's echo of the typed text is no reseed");
        assert_eq!(n.model.rows().len(), 2, "the app's filtered rows reached the native side");
        assert_eq!(n.model.completion(), "p to Lisbon", "the completion is shown, the app never saw it");
    }
    read(&cx, &handle, |p| assert_eq!(p.queries, ["tri"]));
    user(&mut cx, &natives, |m| m.key(PaletteKey::Tab).unwrap_or_default());
    read(&cx, &handle, |p| assert_eq!(p.queries.last().map(|s| s.as_ref()), Some("trip to Lisbon")));
    assert_eq!(natives.borrow()[&node].reseeds, 0);
}

#[test]
fn activating_closes_without_a_cancel_echo() {
    let (mut cx, natives, handle) = palette_app();
    user(&mut cx, &natives, |m| m.key(PaletteKey::Down).unwrap_or_default());
    user(&mut cx, &natives, |m| m.key(PaletteKey::Return).unwrap_or_default());
    read(&cx, &handle, |p| {
        assert_eq!(p.activated, ["Groceries"]);
        assert_eq!(p.cancels, 0, "an app-driven close is not a cancel");
    });
    let node = node_of::<CommandPaletteKind>(&cx);
    assert!(!natives.borrow()[&node].model.is_presented());
}

#[test]
fn escape_reports_one_cancel_and_the_reopen_starts_from_the_query() {
    let (mut cx, natives, handle) = palette_app();
    user(&mut cx, &natives, |m| m.edited("gro"));
    user(&mut cx, &natives, |m| m.key(PaletteKey::Escape).unwrap_or_default());
    read(&cx, &handle, |p| assert_eq!(p.cancels, 1));
    update(&mut cx, &handle, |p, cx| {
        p.open = true;
        cx.notify();
    });
    cx.run_until_idle();
    let node = node_of::<CommandPaletteKind>(&cx);
    let n = &natives.borrow()[&node];
    assert_eq!(n.presents, 2);
    assert_eq!(n.model.field_text(), "gro", "a present starts from the app's last query");
}

#[test]
fn submitting_typed_text_that_matches_nothing() {
    let (mut cx, natives, handle) = palette_app();
    user(&mut cx, &natives, |m| m.edited("zebra"));
    user(&mut cx, &natives, |m| m.key(PaletteKey::Return).unwrap_or_default());
    read(&cx, &handle, |p| assert_eq!(p.submitted, ["zebra"]));
}

#[test]
fn equal_items_each_render_do_not_reload() {
    let (mut cx, natives, handle) = palette_app();
    let node = node_of::<CommandPaletteKind>(&cx);
    let before = natives.borrow()[&node].reloads;
    // A render that rebuilds the same rows (a fresh vector) sends no reload.
    update(&mut cx, &handle, |_, cx| cx.notify());
    cx.run_until_idle();
    assert_eq!(natives.borrow()[&node].reloads, before);
}

struct Tree {
    nodes: Vec<SourceNode>,
    selected: Option<SharedString>,
    selections: u32,
    actions: Vec<(SharedString, SharedString)>,
}

impl Render for Tree {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        source_tree("sidebar")
            .nodes(self.nodes.clone())
            .actions([source_action("close", Icon::Trash).tooltip("Close")])
            .selected_opt(self.selected.clone())
            .empty_state(Some(Icon::Folder), "No folders", None)
            .on_select(cx.listener(|this, e: &SourceSelected, _, cx| {
                this.selections += 1;
                this.selected = Some(e.id.clone());
                cx.notify();
            }))
            .on_expand(cx.listener(|this, e: &SourceExpanded, _, cx| {
                set_expanded(&mut this.nodes, &e.id, true);
                cx.notify();
            }))
            .on_collapse(cx.listener(|this, e: &SourceCollapsed, _, cx| {
                set_expanded(&mut this.nodes, &e.id, false);
                cx.notify();
            }))
            .on_action(cx.listener(|this, e: &SourceActionClicked, _, cx| {
                this.actions.push((e.id.clone(), e.action.clone()));
                this.nodes.retain(|n| n.id != e.id);
                cx.notify();
            }))
    }
}

fn set_expanded(nodes: &mut [SourceNode], id: &str, expanded: bool) {
    if let Some(n) = nodes.iter_mut().find(|n| n.id == id) {
        n.expanded = expanded;
    }
}

fn tree_nodes() -> Vec<SourceNode> {
    vec![
        source_section("lib", "Library"),
        source_node("inbox", "Inbox").parent("lib").icon(Icon::Inbox).badge("3"),
        source_node("work", "Work").parent("lib").icon(Icon::Folder).actions(["close"]),
        source_node("q3", "Q3").parent("work"),
    ]
}

#[test]
fn selection_fed_back_from_its_own_event_reports_once() {
    let natives_sets = Rc::new(Cell::new(0));
    let mut backend = TestBackend::new();
    let sets = natives_sets.clone();
    let selected: Rc<RefCell<HashMap<NodeId, Option<SharedString>>>> = Rc::default();
    let native = selected.clone();
    backend.on_apply(move |node, kind, props, events| {
        if !kind_is::<SourceTreeKind>(kind) {
            return;
        }
        let p = props.downcast_ref::<SourceTreeProps>().expect("tree props");
        let mut native = native.borrow_mut();
        let current = native.entry(node).or_default();
        if *current != p.selected {
            // The outline view fires its delegate inside the setter; the backend drops it.
            sets.set(sets.get() + 1);
            let emit = events.emitter(node);
            let id = p.selected.clone().unwrap_or_default();
            emit.programmatic(|| emit.emit(SourceSelected { id }));
            *current = p.selected.clone();
        }
    });
    let mut cx = TestApp::with_backend(backend);
    let handle = cx.open_window(WINDOW, |_, cx| {
        cx.new(|_| Tree { nodes: tree_nodes(), selected: None, selections: 0, actions: Vec::new() })
    });
    let node = node_of::<SourceTreeKind>(&cx);
    // The user selects "inbox": the native side holds it and reports it.
    selected.borrow_mut().insert(node, Some("inbox".into()));
    cx.emit(node, SourceSelected { id: "inbox".into() });
    read(&cx, &handle, |t| {
        assert_eq!(t.selections, 1, "one selection, no echo");
        assert_eq!(t.selected.as_deref(), Some("inbox"));
    });
    assert_eq!(natives_sets.get(), 0, "the fed-back selection is what the outline already holds");

    let tree = cx.semantic_tree();
    let (_, sidebar) = tree.nodes().find(|(_, n)| n.test_id.as_deref() == Some("sidebar")).expect("tree node");
    assert_eq!(sidebar.role, Role::Tree);
}

#[test]
fn expansion_and_actions_are_the_apps() {
    let mut cx = TestApp::with_backend(TestBackend::new());
    let handle = cx.open_window(WINDOW, |_, cx| {
        cx.new(|_| Tree { nodes: tree_nodes(), selected: None, selections: 0, actions: Vec::new() })
    });
    let node = node_of::<SourceTreeKind>(&cx);
    let props = |cx: &TestApp| {
        cx.backend().props(node).and_then(|p| p.downcast_ref::<SourceTreeProps>().cloned()).expect("props")
    };
    let visible = |p: &SourceTreeProps| SourceIndex::new(&p.nodes).visible(&p.nodes).len();
    assert_eq!(visible(&props(&cx)), 3, "Work starts collapsed");
    cx.emit(node, SourceExpanded { id: "work".into() });
    assert_eq!(visible(&props(&cx)), 4, "the app's expanded flag reached the widget");
    cx.emit(node, SourceActionClicked { id: "work".into(), action: "close".into() });
    read(&cx, &handle, |t| assert_eq!(t.actions, [("work".into(), "close".into())]));
    assert_eq!(visible(&props(&cx)), 3, "the closed row is gone and its orphaned child is a root");
}

struct Toasts {
    toasts: Vec<Toast>,
    dismissed: Vec<(SharedString, DismissReason)>,
    actions: Vec<SharedString>,
}

impl Render for Toasts {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(
            toast_overlay("toasts")
                .toasts(self.toasts.clone())
                .on_dismiss(cx.listener(|this, e: &ToastDismissed, _, cx| {
                    this.dismissed.push((e.id.clone(), e.reason));
                    this.toasts.retain(|t| t.id != e.id);
                    cx.notify();
                }))
                .on_action(cx.listener(|this, e: &ToastActionClicked, _, cx| {
                    this.actions.push(e.id.clone());
                    cx.notify();
                })),
        )
    }
}

#[test]
fn toasts_queue_and_the_apps_removal_after_a_dismissal_changes_nothing_on_screen() {
    let queues: Rc<RefCell<HashMap<NodeId, ToastQueue>>> = Rc::default();
    let shows: Rc<RefCell<Vec<SharedString>>> = Rc::default();
    let reported = Rc::new(Cell::new(0));
    let mut backend = TestBackend::new();
    let (q, s, r) = (queues.clone(), shows.clone(), reported.clone());
    backend.on_apply(move |node, kind, props, events| {
        if !kind_is::<ToastOverlayKind>(kind) {
            return;
        }
        let p = props.downcast_ref::<ToastOverlayProps>().expect("overlay props");
        let changes = q.borrow_mut().entry(node).or_default().sync(&p.toasts);
        for change in changes {
            match change {
                ToastChange::Show(t) => s.borrow_mut().push(t.id),
                ToastChange::Dismissed(e) => {
                    r.set(r.get() + 1);
                    events.emitter(node).emit(e);
                }
                _ => {}
            }
        }
    });
    let mut cx = TestApp::with_backend(backend);
    let handle = cx.open_window(WINDOW, |_, cx| {
        cx.new(|_| Toasts {
            toasts: vec![toast("saved", "Saved"), toast("deleted", "Note deleted").action("Undo")],
            dismissed: Vec::new(),
            actions: Vec::new(),
        })
    });
    let node = node_of::<ToastOverlayKind>(&cx);
    assert_eq!(*shows.borrow(), ["saved"], "one toast at a time, oldest first");

    // The first times out; the app drops it from its list; the queue moves on, and the sync
    // that removal causes reports nothing again.
    let expired = queues.borrow_mut().get_mut(&node).unwrap().expire("saved");
    for change in expired {
        match change {
            ToastChange::Dismissed(e) => cx.backend().events().emitter(node).emit(e),
            ToastChange::Show(t) => shows.borrow_mut().push(t.id),
            _ => {}
        }
    }
    cx.run_until_idle();
    read(&cx, &handle, |t| assert_eq!(t.dismissed, [("saved".into(), DismissReason::Timeout)]));
    assert_eq!(*shows.borrow(), ["saved", "deleted"]);
    assert_eq!(reported.get(), 0, "no dismissal came back out of the app's own update");

    // A high-priority toast interrupts the one on screen.
    update(&mut cx, &handle, |t, cx| {
        t.toasts.push(toast("offline", "You are offline").high_priority().persistent());
        cx.notify();
    });
    cx.run_until_idle();
    read(&cx, &handle, |t| {
        assert_eq!(t.dismissed.last(), Some(&("deleted".into(), DismissReason::Interrupted)));
        assert_eq!(t.toasts.iter().map(|t| t.id.as_ref()).collect::<Vec<_>>(), ["offline"]);
    });
    assert_eq!(shows.borrow().last().map(|s| s.as_ref()), Some("offline"));

    let tree = cx.semantic_tree();
    let (_, overlay) = tree.nodes().find(|(_, n)| n.test_id.as_deref() == Some("toasts")).expect("overlay");
    assert_eq!((overlay.role, overlay.name.as_str()), (Role::Status, "You are offline"));
}

struct Banners {
    revealed: bool,
    clicks: u32,
}

impl Render for Banners {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex().size_full().child(
            banner("update", "An update is ready").action("Restart").revealed(self.revealed).on_action(cx.listener(
                |this, _: &BannerActionClicked, _, cx| {
                    this.clicks += 1;
                    this.revealed = false;
                    cx.notify();
                },
            )),
        )
    }
}

#[test]
fn banner_reports_its_button_and_hides_from_its_own_handler() {
    let mut cx = TestApp::with_backend(TestBackend::new());
    let handle = cx.open_window(WINDOW, |_, cx| cx.new(|_| Banners { revealed: true, clicks: 0 }));
    let node = node_of::<BannerKind>(&cx);
    cx.emit(node, BannerActionClicked);
    read(&cx, &handle, |b| assert_eq!((b.clicks, b.revealed), (1, false)));
    let props = cx.backend().props(node).and_then(|p| p.downcast_ref::<BannerProps>().cloned()).expect("props");
    assert!(!props.revealed);
    let tree = cx.semantic_tree();
    let (_, banner) = tree.nodes().find(|(_, n)| n.test_id.as_deref() == Some("update")).expect("banner");
    assert_eq!((banner.role, banner.name.as_str()), (Role::Status, "An update is ready"));
}
