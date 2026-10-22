//! Tables, trees and grids on the GPUI presenter (M5), the same API the AppKit presenters map to
//! NSTableView columns, NSOutlineView and NSCollectionView: a table lays cells out at its
//! columns' widths under a header whose sortable titles report `SortChanged`; a tree shows
//! children only under expanded rows and a disclosure click expands one; a grid fills lanes and
//! reaches its last item.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use nd_backend_gpui::GpuiHost;
use nd_gpui::gpui::{
    HeadlessAppContext, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PlatformInput, point,
};
use nd_ui::WindowId;
use nd_ui::prelude::*;
use nd_virtual::{ListState, SortChanged, SortDirection, TreeNode, column, grid, list, table, tree};

fn headless() -> HeadlessAppContext {
    let text = nd_gpui::gpui_platform::current_platform(true).text_system();
    HeadlessAppContext::with_platform(text, Arc::new(()), nd_gpui::gpui_platform::current_headless_renderer)
}

fn frames(cx: &mut HeadlessAppContext, host: &GpuiHost, n: usize) {
    for _ in 0..n {
        cx.run_until_parked();
        cx.update(|cx| {
            host.pump(cx);
            host.draw(cx);
        });
        std::thread::sleep(Duration::from_millis(4));
    }
}

fn click(cx: &mut HeadlessAppContext, host: &GpuiHost, window: WindowId, x: f32, y: f32) {
    let handle = host.window_handle(window).expect("gpui window");
    let position = point(px(x), px(y));
    let down = MouseDownEvent {
        button: MouseButton::Left,
        position,
        modifiers: Modifiers::default(),
        click_count: 1,
        first_mouse: false,
    };
    let up = MouseUpEvent { button: MouseButton::Left, position, modifiers: Modifiers::default(), click_count: 1 };
    cx.update(|cx| {
        let _ = handle.update(cx, |_, window, cx| {
            window.dispatch_event(PlatformInput::MouseDown(down), cx);
            window.dispatch_event(PlatformInput::MouseUp(up), cx);
        });
    });
}

fn painted(host: &GpuiHost) -> (nd_ui::Rect, Vec<(nd_virtual::RowId, nd_ui::Rect)>) {
    let list = *host.backend().lists().first().expect("list");
    host.backend().painted_rows(list).expect("painted")
}

fn open<V: Render>(
    width: f32,
    build: impl FnOnce(&mut Context<V>) -> V + 'static,
) -> (HeadlessAppContext, GpuiHost, WindowId, Entity<V>) {
    let mut cx = headless();
    let mut opened = None;
    let host = cx.update(|cx| {
        GpuiHost::builder().quit_when_empty(false).start(cx, |app| {
            let handle =
                app.open_window(WindowOptions::main("kinds").size(size(px(width), px(300.))), |_, cx| cx.new(build));
            opened = Some((handle.id(), handle.root().clone()));
        })
    });
    let (window, view) = opened.expect("window");
    frames(&mut cx, &host, 6);
    (cx, host, window, view)
}

struct Table {
    list: ListState<u32>,
    sort: Option<SortDirection>,
    sorted: Rc<RefCell<Vec<SortChanged>>>,
}

impl Render for Table {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut name = column("name", "Name").width(150.);
        if let Some(d) = self.sort {
            name = name.sorted(d);
        }
        let columns = vec![name.sortable(), column("size", "Size").width(80.), column("kind", "Kind").width(90.)];
        let sorted = self.sorted.clone();
        table("files", &self.list, columns, |row: u32, col: usize, _, _| {
            div().h(px(24.)).child(match col {
                0 => format!("file-{row}"),
                1 => format!("{} KB", row * 3),
                _ => "Document".to_owned(),
            })
        })
        .on_sort(cx.listener(move |this, e: &SortChanged, _, cx| {
            sorted.borrow_mut().push(e.clone());
            this.sort = Some(e.direction);
            cx.notify();
        }))
    }
}

pub fn table_columns_and_sort() {
    let sorted = Rc::new(RefCell::new(Vec::new()));
    let s = sorted.clone();
    let (mut cx, host, window, _) = open(400., move |cx| {
        let mut list = ListState::new(cx);
        list.set_items((0..200).collect::<Vec<u32>>(), cx);
        Table { list, sort: None, sorted: s }
    });
    let (viewport, rows) = painted(&host);
    assert!((viewport.origin.y - 24.).abs() < 0.5, "rows start under the 24pt header: {viewport:?}");
    let (_, first) = rows[0];
    assert!((first.size.height - 24.).abs() < 0.5, "a row is as tall as its tallest cell: {first:?}");
    // The header's Name title sorts.
    click(&mut cx, &host, window, 20., 12.);
    frames(&mut cx, &host, 4);
    assert_eq!(
        sorted.borrow().iter().map(|e| (e.column.to_string(), e.direction)).collect::<Vec<_>>(),
        vec![("name".to_owned(), SortDirection::Ascending)]
    );
    click(&mut cx, &host, window, 20., 12.);
    frames(&mut cx, &host, 4);
    assert_eq!(sorted.borrow().last().map(|e| e.direction), Some(SortDirection::Descending));
}

struct Tree {
    list: ListState<&'static str>,
}

impl Render for Tree {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        tree("outline", &self.list, |key: &'static str, _, _| div().h(px(22.)).child(key))
    }
}

fn node(key: &'static str, parent: Option<&'static str>) -> TreeNode<&'static str> {
    TreeNode { key, parent, version: 0 }
}

pub fn tree_disclosure_expands_children() {
    let (mut cx, host, window, view) = open(300., |cx| {
        let mut list = ListState::new(cx);
        list.set_tree(
            [
                node("Documents", None),
                node("report.pdf", Some("Documents")),
                node("notes", Some("Documents")),
                node("todo.md", Some("notes")),
                node("Pictures", None),
            ],
            cx,
        );
        Tree { list }
    });
    let items = |host: &GpuiHost| host.read(|app| view.read(app).list.items());
    assert_eq!(items(&host), vec!["Documents", "Pictures"], "collapsed: roots only");
    // The disclosure column is the first INDENT points of a root row.
    click(&mut cx, &host, window, 8., 11.);
    frames(&mut cx, &host, 4);
    assert_eq!(items(&host), vec!["Documents", "report.pdf", "notes", "Pictures"]);
    let (_, rows) = painted(&host);
    let x = |i: usize| rows[i].1.origin.x;
    assert!(x(1) > x(0), "children are indented: {} vs {}", x(1), x(0));
    click(&mut cx, &host, window, 8., 11.);
    frames(&mut cx, &host, 4);
    assert_eq!(items(&host), vec!["Documents", "Pictures"], "collapsed again");
}

struct Grid {
    list: ListState<u32>,
}

impl Render for Grid {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        grid("photos", &self.list, (px(80.), px(80.)), |item: u32, _, _| {
            div().size_full().bg(hsla((item % 360) as f32 / 360., 0.5, 0.5, 1.))
        })
        .gap(px(8.))
    }
}

pub fn grid_fills_lanes_and_reaches_the_end() {
    let (mut cx, host, _window, view) = open(360., |cx| {
        let mut list = ListState::new(cx);
        list.set_items((0..1000).collect::<Vec<u32>>(), cx);
        Grid { list }
    });
    let (_, cells) = painted(&host);
    let first_row: Vec<_> = cells.iter().filter(|(_, r)| r.origin.y < 1.).collect();
    assert_eq!(first_row.len(), 4, "four 80pt cells with 8pt gaps fit 360pt");
    let host2 = host.clone();
    cx.update(|cx| host2.update(cx, |app| view.update(app, |this, cx| this.list.scroll_to_end(cx))));
    frames(&mut cx, &host, 6);
    let (viewport, cells) = painted(&host);
    let state = host.read(|app| view.read(app).list.clone());
    let last = cells.iter().find(|(r, _)| state.key_of(*r) == Some(999)).expect("last item painted");
    let bottom = last.1.origin.y + last.1.size.height;
    assert!(bottom <= viewport.origin.y + viewport.size.height + 0.5, "last item fully in view: {last:?}");
}

struct Hovered {
    list: ListState<u32>,
}

/// The wash a hovered row paints, which nothing else in the window does.
const WASH: u32 = 0xff0000;

impl Render for Hovered {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        v_flex().size_full().child(div().h(px(200.)).w_full().child(list("songs", &self.list, |row: u32, _, _| {
            // As an app writes a track row: an id, and a hover style on the row itself.
            div().id(format!("song-{row}")).h(px(40.)).w_full().hover(|s| s.bg(rgb(WASH))).child(format!("song {row}"))
        })))
    }
}

/// Draws the window only when GPUI would: after its root view was notified. A forced draw
/// repaints hover styles from the current pointer and would hide a move that asked for no frame.
fn frame_if_asked(cx: &mut HeadlessAppContext, host: &GpuiHost, asked: &Rc<std::cell::Cell<bool>>) {
    cx.run_until_parked();
    cx.update(|cx| host.pump(cx));
    if asked.replace(false) {
        cx.update(|cx| host.draw(cx));
    }
    cx.run_until_parked();
}

fn move_to(cx: &mut HeadlessAppContext, host: &GpuiHost, window: WindowId, x: f32, y: f32) {
    let handle = host.window_handle(window).expect("gpui window");
    let event = MouseMoveEvent { position: point(px(x), px(y)), pressed_button: None, modifiers: Modifiers::default() };
    cx.update(|cx| {
        let _ = handle.update(cx, |_, window, cx| {
            window.dispatch_event(PlatformInput::MouseMove(event), cx);
        });
    });
}

/// The keys of the painted rows that show the hover wash.
fn washed(cx: &mut HeadlessAppContext, host: &GpuiHost, window: WindowId, state: &ListState<u32>) -> Vec<u32> {
    let handle = host.window_handle(window).expect("gpui window");
    let image = cx.capture_screenshot(handle).expect("screenshot");
    let scale = image.width() as f32 / 300.;
    let (viewport, rows) = painted(host);
    rows.iter()
        .filter(|(_, b)| b.origin.y + b.size.height / 2. < viewport.origin.y + viewport.size.height)
        .filter(|(_, b)| {
            let (x, y) = ((b.origin.x + 4.) * scale, (b.origin.y + b.size.height / 2.) * scale);
            let p = image.get_pixel(x as u32, y as u32).0;
            p[0] > 200 && p[1] < 60 && p[2] < 60
        })
        .filter_map(|(r, _)| state.key_of(*r))
        .collect()
}

/// A row's hover style follows the pointer on the frame its move asks for: exactly the row
/// under the pointer shows it, also after the list scrolled, and none once the pointer leaves.
pub fn row_hover_follows_the_pointer() {
    let (mut cx, host, window, view) = open(300., |cx| {
        let mut list = ListState::new(cx);
        list.set_items((0..60).collect::<Vec<u32>>(), cx);
        Hovered { list }
    });
    let asked = Rc::new(std::cell::Cell::new(false));
    let root = host.window_handle(window).expect("gpui window");
    let flag = asked.clone();
    cx.update(|cx| {
        let view = root.update(cx, |root, _, _| root.downcast::<nd_backend_gpui::NdView>()).expect("window");
        cx.observe(&view.expect("nd root view"), move |_, _| flag.set(true)).detach();
    });
    let state = host.read(|app| view.read(app).list.clone());
    // A headless window starts with its pointer at the origin, over the first row.
    move_to(&mut cx, &host, window, 150., 260.);
    frame_if_asked(&mut cx, &host, &asked);
    assert!(washed(&mut cx, &host, window, &state).is_empty(), "no row is hovered while the pointer is below the list");
    for (row, y) in [(0, 20.), (1, 60.), (3, 140.), (2, 100.), (4, 180.)] {
        move_to(&mut cx, &host, window, 150., y);
        frame_if_asked(&mut cx, &host, &asked);
        assert_eq!(washed(&mut cx, &host, window, &state), vec![row], "pointer at y {y}");
    }
    // The list jumps ten rows under a pointer that stays put.
    let host2 = host.clone();
    cx.update(|cx| {
        host2.update(cx, |app| view.update(app, |this, cx| this.list.jump_to(10, nd_virtual::Align::Start, px(0.), cx)))
    });
    frames(&mut cx, &host, 6);
    asked.set(false);
    assert_eq!(washed(&mut cx, &host, window, &state), vec![14], "after a scroll, the row now under the pointer");
    for (row, y) in [(11, 60.), (12, 100.), (10, 20.)] {
        move_to(&mut cx, &host, window, 150., y);
        frame_if_asked(&mut cx, &host, &asked);
        assert_eq!(washed(&mut cx, &host, window, &state), vec![row], "after the scroll, pointer at y {y}");
    }
    move_to(&mut cx, &host, window, 150., 260.);
    frame_if_asked(&mut cx, &host, &asked);
    assert!(washed(&mut cx, &host, window, &state).is_empty(), "no stale wash once the pointer leaves the list");
}

struct Plain {
    list: ListState<u32>,
}

impl Render for Plain {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(list("plain", &self.list, |row: u32, _, _| {
            div().h(px(30.)).child(div().id(format!("cell-{row}")).child(format!("row {row}")))
        }))
    }
}

/// Automation's ScrollIntoView on a node inside a row below the fold scrolls the list until
/// that row shows, as a test reaching a sidebar's last chat needs.
pub fn scroll_into_view_reaches_a_row_below_the_fold() {
    let (mut cx, host, window, view) = open(300., |cx| {
        let mut list = ListState::new(cx);
        list.set_items((0..16).collect::<Vec<u32>>(), cx);
        Plain { list }
    });
    let state = host.read(|app| view.read(app).list.clone());
    let shows = |host: &GpuiHost| {
        let (viewport, rows) = painted(host);
        rows.iter().any(|(r, b)| {
            state.key_of(*r) == Some(14)
                && b.origin.y >= viewport.origin.y - 0.5
                && b.origin.y + b.size.height <= viewport.origin.y + viewport.size.height + 0.5
        })
    };
    assert!(!shows(&host), "row 14 starts below the fold");
    let node = host.read(|app| {
        let tree = app.semantic_window(window).expect("window");
        tree.root.iter().find(|n| n.test_id.as_deref() == Some("cell-14")).map(|n| n.id).expect("row 14's node")
    });
    let host2 = host.clone();
    cx.update(|cx| host2.update(cx, |app| app.perform(node, &nd_ui::semantics::SemanticAction::ScrollIntoView)))
        .expect("scrolled");
    frames(&mut cx, &host, 6);
    assert!(shows(&host), "row 14 shows whole after ScrollIntoView");
}

fn main() {
    let tests: [(&str, fn()); 5] = [
        ("gate_m5::gpui_kinds::row_hover_follows_the_pointer", row_hover_follows_the_pointer),
        (
            "gate_m5::gpui_kinds::scroll_into_view_reaches_a_row_below_the_fold",
            scroll_into_view_reaches_a_row_below_the_fold,
        ),
        ("gate_m5::gpui_kinds::table_columns_and_sort", table_columns_and_sort),
        ("gate_m5::gpui_kinds::tree_disclosure_expands_children", tree_disclosure_expands_children),
        ("gate_m5::gpui_kinds::grid_fills_lanes_and_reaches_the_end", grid_fills_lanes_and_reaches_the_end),
    ];
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--list") {
        if !args.iter().any(|a| a == "--ignored") {
            for (n, _) in &tests {
                println!("{n}: test");
            }
        }
        return;
    }
    if args.iter().any(|a| a == "--ignored") {
        return;
    }
    let filters: Vec<&String> = args.iter().filter(|a| !a.starts_with('-')).collect();
    let mut failed = 0;
    for (name, test) in tests {
        if !filters.is_empty() && !filters.iter().any(|f| name.contains(f.as_str())) {
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
