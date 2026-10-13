//! Tier 2: a list's row actions on AppKit are the table's own `NSTableViewRowAction`s, answered
//! from the rendered row's tag; picking one runs the app's action through the list's event, never
//! inside the AppKit callback.

mod harness;

fn main() {
    #[cfg(target_os = "macos")]
    harness::main("gate_m5::appkit_row_actions", mac::run);
    #[cfg(not(target_os = "macos"))]
    harness::main("gate_m5::appkit_row_actions", || {});
}

#[cfg(target_os = "macos")]
mod mac {
    use std::cell::Cell;
    use std::rc::Rc;
    use std::time::Duration;

    use nd_backend_appkit::AppHost;
    use nd_ui::prelude::*;
    use nd_virtual::{ListState, RowActionInvoked, RowActions, RowEdge, list, row_action};

    struct Rows {
        list: ListState<u32>,
        ran: Rc<Cell<Option<(u32, &'static str)>>>,
    }

    impl Render for Rows {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            let ran = self.ran.clone();
            list("rows", &self.list, |row: u32, _, _| div().w_full().h(px(44.)).child(format!("Row {row}")))
                .row_actions(move |row: u32, _| {
                    let (a, b) = (ran.clone(), ran.clone());
                    RowActions::new()
                        .trailing(
                            row_action("Delete")
                                .icon("trash")
                                .destructive()
                                .on_invoke(move |_, _| a.set(Some((row, "delete")))),
                        )
                        .leading(row_action("Pin").on_invoke(move |_, _| b.set(Some((row, "pin")))))
                })
                .size_full()
        }
    }

    pub fn run() {
        let ran = Rc::new(Cell::new(None));
        let r = ran.clone();
        let host = AppHost::new(|app| {
            app.open_window(WindowOptions::main("rows").size(size(px(320.), px(300.))), |_, cx| {
                cx.new(|cx| {
                    let mut list = ListState::new(cx);
                    list.set_items((0..50).collect::<Vec<u32>>(), cx);
                    Rows { list, ran: r }
                })
            });
        });
        let pump = |host: &AppHost| {
            for _ in 0..10 {
                host.platform().pump(Duration::from_millis(1), Duration::from_millis(20));
                host.update(|app| {
                    app.drain_events();
                    if app.needs_frame() {
                        app.frame();
                    }
                });
            }
        };
        pump(&host);
        let backend = host.backend();
        let list_node = *backend.lists().first().expect("list");
        assert_eq!(backend.list_row_actions(list_node, 2, false), vec!["Delete".to_owned()]);
        assert_eq!(backend.list_row_actions(list_node, 2, true), vec!["Pin".to_owned()]);
        // What the action's handler block queues when the user picks Delete on row 2.
        let row_id = row_id_of(&host, 2);
        host.update(|app| {
            app.events().emitter(list_node).emit(RowActionInvoked { row: row_id, edge: RowEdge::Trailing, index: 0 })
        });
        pump(&host);
        assert_eq!(ran.get(), Some((2, "delete")));
        println!("ND_APPKIT_ROW_ACTIONS_OK");
        std::process::exit(0);
    }

    /// The model's id for the row at `index`.
    fn row_id_of(host: &AppHost, index: usize) -> nd_virtual::RowId {
        let list = *host.backend().lists().first().expect("list");
        let (table, _) = host.backend().list_views(list).expect("views");
        let table = table.downcast::<nd_backend_appkit::objc2_app_kit::NSTableView>().expect("table");
        let container = table.viewAtColumn_row_makeIfNecessary(0, index as isize, true).expect("cell");
        let row_view = container.subviews().firstObject().expect("row view");
        let node = host.backend().node_for_view(&*row_view).expect("row node");
        host.backend().list_row(node).expect("row id")
    }
}
