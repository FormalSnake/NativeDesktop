//! Tier 2: tables, trees and grids map to AppKit's own views (M5): a table's columns are
//! NSTableColumns with the cells in them and a header click reports `SortChanged`; a tree is an
//! NSOutlineView whose disclosure expands the app's tree; a grid is an NSCollectionView that
//! scrolls to its last item. One test per process, chosen by name.

mod harness;

fn main() {
    #[cfg(target_os = "macos")]
    {
        let name = std::env::args().skip(1).find(|a| a.starts_with("gate_m5::"));
        let tests: [(&str, fn()); 3] = [
            ("gate_m5::appkit_kinds_table", mac::table),
            ("gate_m5::appkit_kinds_tree", mac::tree),
            ("gate_m5::appkit_kinds_grid", mac::grid),
        ];
        if std::env::args().any(|a| a == "--list") {
            if !std::env::args().any(|a| a == "--ignored") {
                for (n, _) in tests {
                    println!("{n}: test");
                }
            }
            return;
        }
        for (n, f) in tests {
            if name.as_deref().is_none_or(|x| x == n) {
                harness::main(n, f);
            }
        }
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::time::Duration;

    use nd_backend_appkit::AppHost;
    use nd_backend_appkit::objc2::msg_send;
    use nd_backend_appkit::objc2::rc::Retained;
    use nd_backend_appkit::objc2_app_kit::{NSCollectionView, NSOutlineView, NSTableView};
    use nd_ui::prelude::*;
    use nd_virtual::{
        ListState, SortChanged, TreeNode, column, grid as grid_list, table as table_list, tree as tree_list,
    };

    fn pump(host: &AppHost) {
        for _ in 0..12 {
            host.platform().pump(Duration::from_millis(1), Duration::from_millis(20));
            host.update(|app| {
                app.drain_events();
                if app.needs_frame() {
                    app.frame();
                }
            });
        }
    }

    fn open<V: Render>(build: impl FnOnce(&mut Context<V>) -> V + 'static) -> (AppHost, Entity<V>) {
        let mut view = None;
        let host = AppHost::new(|app| {
            let handle =
                app.open_window(WindowOptions::main("kinds").size(size(px(400.), px(300.))), |_, cx| cx.new(build));
            view = Some(handle.root().clone());
        });
        pump(&host);
        (host, view.expect("view"))
    }

    fn document(host: &AppHost) -> Retained<nd_backend_appkit::objc2_app_kit::NSView> {
        let list = *host.backend().lists().first().expect("list");
        host.backend().list_views(list).expect("views").0
    }

    struct Table {
        list: ListState<u32>,
        sorted: Rc<RefCell<Vec<SortChanged>>>,
    }

    impl Render for Table {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            let columns = vec![
                column("name", "Name").width(150.).sortable(),
                column("size", "Size").width(80.),
                column("kind", "Kind").width(90.),
            ];
            let sorted = self.sorted.clone();
            table_list("files", &self.list, columns, |row: u32, col: usize, _, _| {
                div().h(px(24.)).child(format!("r{row}c{col}"))
            })
            .on_sort(move |e: &SortChanged, _, _| sorted.borrow_mut().push(e.clone()))
        }
    }

    pub fn table() {
        let sorted = Rc::new(RefCell::new(Vec::new()));
        let s = sorted.clone();
        let (host, _) = open(move |cx| {
            let mut list = ListState::new(cx);
            list.set_items((0..100).collect::<Vec<u32>>(), cx);
            Table { list, sorted: s }
        });
        let table = document(&host).downcast::<NSTableView>().expect("NSTableView");
        let columns = table.tableColumns();
        let titles: Vec<String> = columns.iter().map(|c| c.title().to_string()).collect();
        assert_eq!(titles, ["Name", "Size", "Kind"]);
        assert!(table.headerView().is_some(), "a table has a header");
        assert!(table.usesAlternatingRowBackgroundColors(), "a multicolumn table alternates its rows");
        for col in 0..3 {
            let cell = table.viewAtColumn_row_makeIfNecessary(col, 0, true).expect("cell");
            assert_eq!(cell.subviews().len(), 1, "column {col} holds its cell's view");
        }
        assert!((table.rectOfRow(0).size.height - 24.).abs() < 0.5, "row height from the cells");
        let delegate = unsafe { table.delegate() }.expect("delegate");
        let name = columns.objectAtIndex(0);
        let _: () = unsafe { msg_send![&*delegate, tableView: &*table, didClickTableColumn: &*name] };
        pump(&host);
        assert_eq!(sorted.borrow().len(), 1, "a sortable header click reports SortChanged");
        assert_eq!(sorted.borrow()[0].column.as_ref(), "name");
        println!("ND_APPKIT_TABLE_OK");
        std::process::exit(0);
    }

    struct Tree {
        list: ListState<&'static str>,
    }

    impl Render for Tree {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            tree_list("outline", &self.list, |key: &'static str, _, _| div().h(px(22.)).child(key))
        }
    }

    fn node(key: &'static str, parent: Option<&'static str>) -> TreeNode<&'static str> {
        TreeNode { key, parent, version: 0 }
    }

    pub fn tree() {
        let (host, view) = open(|cx| {
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
        let outline = document(&host).downcast::<NSOutlineView>().expect("NSOutlineView");
        assert_eq!(outline.numberOfRows(), 2, "roots only");
        // The user opens Documents with its disclosure triangle.
        let item = outline.itemAtRow(0).expect("item");
        unsafe { outline.expandItem(Some(&item)) };
        pump(&host);
        let items = host.update(|app| view.read(app).list.items());
        assert_eq!(items, ["Documents", "report.pdf", "notes", "Pictures"], "the app's tree followed");
        assert_eq!(outline.numberOfRows(), 4, "the outline shows the children");
        assert_eq!(outline.levelForRow(1), 1, "children one level in");
        println!("ND_APPKIT_TREE_OK");
        std::process::exit(0);
    }

    struct Grid {
        list: ListState<u32>,
    }

    impl Render for Grid {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            grid_list("photos", &self.list, (px(80.), px(80.)), |item: u32, _, _| {
                div().size_full().bg(hsla((item % 360) as f32 / 360., 0.5, 0.5, 1.))
            })
            .gap(px(8.))
        }
    }

    pub fn grid() {
        let (host, view) = open(|cx| {
            let mut list = ListState::new(cx);
            list.set_items((0..1000).collect::<Vec<u32>>(), cx);
            Grid { list }
        });
        let collection = document(&host).downcast::<NSCollectionView>().expect("NSCollectionView");
        assert_eq!(collection.numberOfItemsInSection(0), 1000);
        host.update(|app| view.update(app, |this, cx| this.list.scroll_to_end(cx)));
        pump(&host);
        let list = *host.backend().lists().first().expect("list");
        let (_, scroll) = host.backend().list_views(list).expect("views");
        let visible = scroll.contentView().documentVisibleRect();
        let last = collection.frameForItemAtIndex(999);
        assert!(
            last.origin.y + last.size.height <= visible.origin.y + visible.size.height + 0.5
                && last.origin.y >= visible.origin.y - 0.5,
            "last item {last:?} in the visible rect {visible:?}"
        );
        println!("ND_APPKIT_GRID_OK");
        std::process::exit(0);
    }
}
