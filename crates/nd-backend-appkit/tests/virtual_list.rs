//! Tier 2: the `nd-virtual` conformance suite on a real `NSTableView` (M5 gate). Rows are `nd-ui`
//! views laid out by Taffy at the width the table gives them; user scrolls move the scroll view's
//! clip view as a wheel would; positions are read back from the table (`rectOfRow:`, the clip
//! view's bounds) and from the row views inside the cells it shows.

fn main() {
    #[cfg(target_os = "macos")]
    mac::main();
}

#[cfg(target_os = "macos")]
mod mac {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;
    use std::time::Duration;

    use nd_backend_appkit::AppHost;
    use nd_backend_appkit::objc2_app_kit::NSTableView;
    use nd_backend_appkit::objc2_foundation::NSPoint;
    use nd_ui::prelude::*;
    use nd_ui::{NodeId, WindowId};
    use nd_virtual::conformance::{Harness, Painted, Setup, test_main};
    use nd_virtual::{Align, ListState, list};

    struct Rows {
        state: ListState<u64>,
        content: Rc<RefCell<HashMap<u64, f64>>>,
        versions: HashMap<u64, u64>,
        width: f32,
        height: f32,
    }

    impl Render for Rows {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            let content = self.content.clone();
            div().size_full().child(div().w(px(self.width)).h(px(self.height)).child(list(
                "rows",
                &self.state,
                move |key: u64, _, _| {
                    let h = content.borrow().get(&key).copied().unwrap_or(10.);
                    div().w_full().h(px(h as f32))
                },
            )))
        }
    }

    pub struct Mac {
        host: AppHost,
        window: WindowId,
        view: Entity<Rows>,
        height: f64,
    }

    fn open(setup: Setup) -> Mac {
        static WINDOW_LOCK: std::sync::OnceLock<ndshell::lock::MachineLock> = std::sync::OnceLock::new();
        WINDOW_LOCK.get_or_init(|| ndshell::lock::window_test("gate_m5::appkit_conformance"));
        let mut opened = None;
        let host = AppHost::new(|app| {
            let content = Rc::new(RefCell::new(HashMap::new()));
            let handle =
                app.open_window(WindowOptions::main("rows").title("Rows").size(size(px(800.), px(900.))), |_, cx| {
                    cx.new(|cx| Rows {
                        state: ListState::new(cx)
                            .alignment(setup.alignment)
                            .follow(setup.follow)
                            .estimated_row_height(px(setup.estimate as f32)),
                        content,
                        versions: HashMap::new(),
                        width: setup.width as f32,
                        height: setup.height as f32,
                    })
                });
            opened = Some((handle.id(), handle.root().clone()));
        });
        let (window, view) = opened.expect("window");
        let mut m = Mac { host, window, view, height: setup.height };
        m.settle();
        m
    }

    impl Drop for Mac {
        fn drop(&mut self) {
            let window = self.window;
            self.host.update(|app| app.close_window(window));
            self.host.platform().pump(Duration::from_millis(1), Duration::from_millis(20));
        }
    }

    impl Mac {
        fn update(&mut self, f: impl FnOnce(&mut Rows, &mut Context<Rows>)) {
            let view = self.view.clone();
            self.host.update(|app| view.update(app, f));
        }

        fn list_handle(&self) -> nd_virtual::ListHandle {
            self.host.update(|app| self.view.read(app).state.handle())
        }

        fn list(&self) -> NodeId {
            *self.host.backend().lists().first().expect("a list")
        }
    }

    impl Harness for Mac {
        fn set_rows(&mut self, rows: &[(u64, f64)]) {
            let rows = rows.to_vec();
            self.update(move |this, cx| {
                let mut changed = Vec::new();
                {
                    let mut content = this.content.borrow_mut();
                    for (k, h) in &rows {
                        if content.insert(*k, *h).is_some_and(|old| old != *h) {
                            changed.push(*k);
                        }
                    }
                }
                for k in changed {
                    *this.versions.entry(k).or_default() += 1;
                }
                let versioned: Vec<(u64, u64)> =
                    rows.iter().map(|(k, _)| (*k, this.versions.get(k).copied().unwrap_or(0))).collect();
                this.state.set_items_versioned(versioned, cx);
            });
        }

        fn set_content_height(&mut self, key: u64, height: f64) {
            self.update(move |this, cx| {
                this.content.borrow_mut().insert(key, height);
                *this.versions.entry(key).or_default() += 1;
                this.state.invalidate(&key, cx);
            });
        }

        fn scroll_by(&mut self, dy: f64) {
            let (table, scroll) = self.host.backend().list_views(self.list()).expect("list views");
            let clip = scroll.contentView();
            let bounds = clip.bounds();
            let insets = scroll.contentInsets();
            let max = (table.frame().size.height - bounds.size.height).max(-insets.top);
            let y = (bounds.origin.y + dy).clamp(-insets.top, max);
            // What NSScrollView does for a wheel event: move the clip view, then the scroller.
            clip.scrollToPoint(NSPoint::new(bounds.origin.x, y));
            scroll.reflectScrolledClipView(&clip);
            self.settle();
        }

        fn scroll_to(&mut self, key: u64, align: Align, offset: f64) {
            self.update(move |this, cx| this.state.jump_to(key, align, px(offset as f32), cx));
        }

        fn scroll_to_end(&mut self) {
            self.update(|this, cx| this.state.scroll_to_end(cx));
        }

        fn resize(&mut self, width: f64, height: f64) {
            self.height = height;
            self.update(move |this, cx| {
                this.width = width as f32;
                this.height = height as f32;
                cx.notify();
            });
        }

        fn settle(&mut self) {
            for _ in 0..16 {
                self.host.platform().pump(Duration::from_millis(1), Duration::from_millis(30));
                self.host.update(|app| {
                    app.drain_events();
                    if app.needs_frame() {
                        app.frame();
                    }
                });
            }
        }

        fn painted(&mut self) -> Vec<Painted> {
            let backend = self.host.backend();
            let (table, scroll) = backend.list_views(self.list()).expect("list views");
            let table = table.downcast::<NSTableView>().expect("an NSTableView");
            let clip = scroll.contentView();
            let visible = clip.bounds();
            let range = table.rowsInRect(visible);
            if std::env::var_os("ND_DEBUG_LIST").is_some() {
                let h = self.list_handle();
                let core = h.0.borrow();
                eprintln!(
                    "debug: clip y={} h={} table h={} rows={} range={:?} core top={} total={} vp={} len={}",
                    visible.origin.y,
                    visible.size.height,
                    table.frame().size.height,
                    table.numberOfRows(),
                    (range.location, range.length),
                    core.scroll_top(),
                    core.total(),
                    core.viewport(),
                    core.len()
                );
                let mut shown = 0;
                for i in 0..core.len() {
                    let r = table.rectOfRow(i as isize);
                    if (r.size.height - core.height(i)).abs() > 0.01 && shown < 5 {
                        shown += 1;
                        eprintln!(
                            "debug: table row {i} h={} core h={} y={} core y={}",
                            r.size.height,
                            core.height(i),
                            r.origin.y,
                            core.top(i)
                        );
                    }
                }
            }
            let state = self.host.update(|app| self.view.read(app).state.clone());
            let mut out = Vec::new();
            for i in range.location..range.location + range.length {
                let rect = table.rectOfRow(i as isize);
                let Some(container) = table.viewAtColumn_row_makeIfNecessary(0, i as isize, false) else { continue };
                let Some(row_view) = container.subviews().firstObject() else { continue };
                let Some(node) = backend.node_for_view(&*row_view) else { continue };
                let Some(key) = backend.list_row(node).and_then(|r| state.key_of(r)) else { continue };
                // The row view's own height: the content, as Taffy measured it. A row AppKit
                // gave a different height than its content shows up as a gap or overlap.
                out.push(Painted { key, y: rect.origin.y - visible.origin.y, height: row_view.frame().size.height });
            }
            out
        }

        fn following(&mut self) -> bool {
            self.host.update(|app| self.view.read(app).state.is_following_tail())
        }

        fn renders(&mut self, key: u64) -> u64 {
            self.host.update(|app| {
                let state = self.view.read(app).state.clone();
                state.row_view(&key).map_or(0, |v| app.render_count(v))
            })
        }

        fn viewport_height(&self) -> f64 {
            self.height
        }
    }

    pub fn main() {
        test_main("gate_m5::appkit_conformance", &(open as fn(Setup) -> Mac));
    }
}
