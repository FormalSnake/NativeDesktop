//! The `nd-virtual` conformance suite on the GPUI presenter (M5 gate), in GPUI's headless app
//! with the platform's real text system and renderer. Rows are real `nd-ui` views reconciled into
//! the backend's tree, laid out and painted by GPUI; user scrolls go in as `ScrollWheelEvent`s
//! through the window's input path, and every position is read back from GPUI's own layout.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use nd_backend_gpui::GpuiHost;
use nd_gpui::gpui::{self, HeadlessAppContext, Modifiers, PlatformInput, ScrollDelta, ScrollWheelEvent, TouchPhase};
use nd_ui::prelude::*;
use nd_ui::{NodeId, WindowId};
use nd_virtual::conformance::{Harness, Painted, Setup};
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

struct Gpui {
    cx: HeadlessAppContext,
    host: GpuiHost,
    window: WindowId,
    view: Entity<Rows>,
    height: f64,
}

fn headless() -> HeadlessAppContext {
    let text = nd_gpui::gpui_platform::current_platform(true).text_system();
    HeadlessAppContext::with_platform(text, Arc::new(()), nd_gpui::gpui_platform::current_headless_renderer)
}

fn open(setup: Setup) -> Gpui {
    let mut cx = headless();
    let content = Rc::new(RefCell::new(HashMap::new()));
    let mut opened = None;
    let host = cx.update(|cx| {
        GpuiHost::builder().quit_when_empty(false).start(cx, |app| {
            let handle =
                app.open_window(WindowOptions::main("main").title("Rows").size(size(px(800.), px(900.))), |_, cx| {
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
        })
    });
    let (window, view) = opened.expect("window");
    let mut g = Gpui { cx, host, window, view, height: setup.height };
    g.settle();
    g
}

impl Gpui {
    fn update(&mut self, f: impl FnOnce(&mut Rows, &mut Context<Rows>)) {
        let view = self.view.clone();
        let host = self.host.clone();
        self.cx.update(|cx| host.update(cx, |app| view.update(app, f)));
    }

    fn list_node(&self) -> NodeId {
        *self.host.backend().lists().first().expect("a list was drawn")
    }

    fn painted_rows(&mut self) -> (nd_ui::Rect, Vec<(u64, nd_ui::Rect)>) {
        let node = self.list_node();
        let (viewport, rows) = self.host.backend().painted_rows(node).expect("list painted");
        let state = self.host.read(|app| self.view.read(app).state.clone());
        (viewport, rows.into_iter().map(|(row, r)| (state.key_of(row).expect("row key").to_owned(), r)).collect())
    }
}

impl Harness for Gpui {
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
        let (viewport, _) = self.painted_rows();
        let at = gpui::point(px(viewport.origin.x + 10.), px(viewport.origin.y + 10.));
        let handle = self.host.window_handle(self.window).expect("gpui window");
        let event = ScrollWheelEvent {
            position: at,
            // A notched wheel: offsets move by exactly the delta, clamped, no momentum or rubber band.
            delta: ScrollDelta::Lines(gpui::point(0., -dy as f32 / 20.)),
            modifiers: Modifiers::default(),
            touch_phase: TouchPhase::Moved,
        };
        self.cx.update(|cx| {
            let _ = handle.update(cx, |_, window, cx| {
                window.dispatch_event(PlatformInput::ScrollWheel(event), cx);
            });
        });
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
        for _ in 0..12 {
            self.cx.run_until_parked();
            let host = self.host.clone();
            self.cx.update(|cx| {
                host.pump(cx);
                host.draw(cx);
            });
        }
        self.cx.run_until_parked();
    }

    fn painted(&mut self) -> Vec<Painted> {
        let (viewport, rows) = self.painted_rows();
        rows.into_iter()
            .map(|(key, r)| Painted {
                key,
                y: f64::from(r.origin.y - viewport.origin.y),
                height: f64::from(r.size.height),
            })
            .collect()
    }

    fn following(&mut self) -> bool {
        self.host.read(|app| self.view.read(app).state.is_following_tail())
    }

    fn renders(&mut self, key: u64) -> u64 {
        self.host.read(|app| {
            let state = &self.view.read(app).state;
            state.row_view(&key).map_or(0, |v| app.render_count(v))
        })
    }

    fn viewport_height(&self) -> f64 {
        self.height
    }
}

fn main() {
    nd_virtual::conformance::test_main("gate_m5::gpui_conformance", &(open as fn(Setup) -> Gpui));
}
