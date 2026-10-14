//! Tier 2: the drawn command palette, toasts and source tree on the GPUI backend inside GPUI's
//! headless app, with input through the window's own key path; and the goldens of the drawn
//! look, light and dark, rendered by GPUI's headless renderer from the same view builders.
//!
//! `ND_UPDATE_GOLDENS=1` rewrites the goldens. Goldens are per OS (text is shaped by the
//! platform's text system): `tests/goldens/nav-<view>-<scheme>-<os>.png`.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use nd_backend_gpui::GpuiHost;
use nd_backend_gpui::nav::{Marks, banner_view, palette_overlay, source_tree_view, theme_for, toast_view};
use nd_components::*;
use nd_core::semantic::Role;
use nd_gpui::gpui::{self, HeadlessAppContext};
use nd_theme::Scheme;
use nd_ui::prelude::*;
use nd_ui::semantics::SyntheticInput;
use nd_ui::{Keystroke, NodeId, SemanticNode, WindowId};

fn headless() -> HeadlessAppContext {
    let text = nd_gpui::gpui_platform::current_platform(true).text_system();
    HeadlessAppContext::with_platform(text, Arc::new(()), nd_gpui::gpui_platform::current_headless_renderer)
}

fn find<'a>(node: &'a SemanticNode, f: &impl Fn(&SemanticNode) -> bool) -> Option<&'a SemanticNode> {
    if f(node) {
        return Some(node);
    }
    node.children.iter().find_map(|c| find(c, f))
}

fn by_test_id(host: &GpuiHost, window: WindowId, id: &str) -> SemanticNode {
    host.read(|app| {
        let tree = app.semantic_window(window).expect("window");
        find(&tree.root, &|n| n.test_id.as_deref() == Some(id)).cloned().expect(id)
    })
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

#[derive(Default)]
struct Log {
    queries: Vec<String>,
    activated: Vec<String>,
    cancels: u32,
    dismissed: Vec<(String, DismissReason)>,
}

const NOTES: [&str; 3] = ["Trip to Lisbon", "Groceries", "Trivia night"];

fn items(query: &str) -> Vec<PaletteItem> {
    let q = query.to_lowercase();
    NOTES
        .iter()
        .filter(|t| t.to_lowercase().contains(&q))
        .map(|t| {
            let item = palette_item(*t, *t).icon(Icon::FileText).subtitle("Notes").hint("Open");
            if !q.is_empty() && t.to_lowercase().starts_with(&q) { item.completion(*t) } else { item }
        })
        .collect()
}

struct Nav {
    log: Rc<RefCell<Log>>,
    open: bool,
    query: SharedString,
    toasts: Vec<Toast>,
}

impl Render for Nav {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .child(
                command_palette("palette")
                    .open(self.open)
                    .placeholder("Search notes")
                    .query(self.query.clone())
                    .items(items(&self.query))
                    .on_query_change(cx.listener(|this, e: &PaletteQueryChanged, _, cx| {
                        this.log.borrow_mut().queries.push(e.text.to_string());
                        this.query = e.text.clone();
                        cx.notify();
                    }))
                    .on_activate(cx.listener(|this, e: &PaletteActivated, _, cx| {
                        this.log.borrow_mut().activated.push(e.id.to_string());
                        this.open = false;
                        cx.notify();
                    }))
                    .on_cancel(cx.listener(|this, _: &PaletteCancelled, _, cx| {
                        this.log.borrow_mut().cancels += 1;
                        this.open = false;
                        cx.notify();
                    })),
            )
            .child(toast_overlay("toasts").toasts(self.toasts.clone()).on_dismiss(cx.listener(
                |this, e: &ToastDismissed, _, cx| {
                    this.log.borrow_mut().dismissed.push((e.id.to_string(), e.reason));
                    this.toasts.retain(|t| t.id != e.id);
                    cx.notify();
                },
            )))
    }
}

struct Opened {
    cx: HeadlessAppContext,
    host: GpuiHost,
    window: WindowId,
    log: Rc<RefCell<Log>>,
    view: Option<Entity<Nav>>,
}

fn open(nav: impl FnOnce(Rc<RefCell<Log>>) -> Nav + 'static) -> Opened {
    let mut cx = headless();
    let log: Rc<RefCell<Log>> = Rc::default();
    let l = log.clone();
    type Slot = Rc<RefCell<Option<(WindowId, Entity<Nav>)>>>;
    let opened: Slot = Rc::default();
    let o = opened.clone();
    let host = cx.update(|cx| {
        GpuiHost::builder().quit_when_empty(false).start(cx, move |app| {
            let handle =
                app.open_window(WindowOptions::main("nav").size(size(px(800.), px(560.))), |_, cx| cx.new(|_| nav(l)));
            *o.borrow_mut() = Some((handle.id(), handle.root().clone()));
        })
    });
    settle(&mut cx, &host);
    let (window, view) = opened.borrow_mut().take().expect("window");
    Opened { cx, host, window, log, view: Some(view) }
}

fn press(o: &mut Opened, key: &str) {
    let keystroke = Keystroke::parse(key).expect("keystroke");
    let window = o.window;
    o.cx.update(|cx| o.host.update(cx, |app| app.synthesize(window, &SyntheticInput::Key(keystroke)))).expect("posted");
    settle(&mut o.cx, &o.host);
}

fn palette_node(o: &Opened) -> SemanticNode {
    by_test_id(&o.host, o.window, "palette")
}

mod gate_m6 {
    use super::*;

    pub fn gpui_palette_types_completes_and_activates() {
        let mut o = open(|log| Nav { log, open: true, query: "".into(), toasts: Vec::new() });
        let palette = palette_node(&o);
        assert_eq!((palette.role, palette.name.as_str()), (Role::Dialog, "Search notes"));
        assert_eq!(palette.expanded, Some(true), "open from its first render");
        for key in ["t", "r", "i"] {
            press(&mut o, key);
        }
        assert_eq!(o.log.borrow().queries, ["t", "tr", "tri"], "each keystroke reaches the app through the window");
        assert_eq!(palette_node(&o).value.as_deref(), Some("trip to Lisbon"), "the first row's completion shows");
        press(&mut o, "tab");
        assert_eq!(o.log.borrow().queries.last().map(String::as_str), Some("trip to Lisbon"), "Tab accepts it");
        press(&mut o, "enter");
        assert_eq!(o.log.borrow().activated, ["Trip to Lisbon"]);
        assert_eq!(palette_node(&o).expanded, Some(false), "the app closed it");
        assert_eq!(o.log.borrow().cancels, 0, "an app-driven close is no cancel");

        let view = o.view.clone().expect("view");
        o.cx.update(|cx| {
            o.host.update(cx, |app| {
                view.update(app, |this, cx| {
                    this.open = true;
                    this.query = "".into();
                    cx.notify();
                })
            })
        });
        settle(&mut o.cx, &o.host);
        press(&mut o, "down");
        press(&mut o, "escape");
        assert_eq!(o.log.borrow().cancels, 1, "Esc cancels once");
        assert_eq!(palette_node(&o).expanded, Some(false));
    }

    pub fn gpui_toasts_time_out_in_order() {
        let mut o = open(|log| Nav {
            log,
            open: false,
            query: "".into(),
            toasts: vec![
                toast("saved", "Saved").timeout(Duration::from_millis(500)),
                toast("synced", "Synced").timeout(Duration::from_millis(500)),
            ],
        });
        assert_eq!(by_test_id(&o.host, o.window, "toasts").name, "Saved");
        o.cx.advance_clock(Duration::from_millis(600));
        settle(&mut o.cx, &o.host);
        assert_eq!(o.log.borrow().dismissed, [("saved".to_string(), DismissReason::Timeout)]);
        assert_eq!(by_test_id(&o.host, o.window, "toasts").name, "Synced", "the queue moved on");
        o.cx.advance_clock(Duration::from_millis(600));
        settle(&mut o.cx, &o.host);
        assert_eq!(o.log.borrow().dismissed.len(), 2);
        assert_eq!(by_test_id(&o.host, o.window, "toasts").name, "");
    }

    pub fn gpui_nav_goldens() {
        // GPUI renders headless only on macOS (Metal) and Linux (wgpu); Windows captures arrive
        // with the Fluent goldens (M3).
        if nd_gpui::gpui_platform::current_headless_renderer().ok().flatten().is_none() {
            println!("skipped: GPUI has no headless renderer on this OS");
            return;
        }
        let mut failed = Vec::new();
        for scheme in [Scheme::Light, Scheme::Dark] {
            for view in [GoldenView::Sidebar, GoldenView::Palette] {
                if let Err(e) = golden(view, scheme) {
                    failed.push(e);
                }
            }
        }
        assert!(failed.is_empty(), "{failed:#?}");
    }
}

#[derive(Clone, Copy, Debug)]
enum GoldenView {
    /// The source tree, a banner and a toast.
    Sidebar,
    /// The palette with a completion, over the same window.
    Palette,
}

struct Golden {
    view: GoldenView,
    scheme: Scheme,
}

fn golden_tree() -> SourceTreeProps {
    SourceTreeProps {
        nodes: vec![
            source_section("lib", "Library"),
            source_node("inbox", "Inbox").parent("lib").icon(Icon::Inbox).badge("12"),
            source_node("work", "Work").parent("lib").icon(Icon::Folder).expanded(true).actions(["close"]),
            source_node("q3", "Q3 planning").parent("work").caption("Edited today"),
            source_node("trips", "Trips").parent("work"),
            source_section("tags", "Tags"),
            source_node("starred", "Starred").parent("tags").icon(Icon::Star),
        ],
        actions: vec![source_action("close", Icon::Trash).label("Close")],
        selected: Some("q3".into()),
        action_visibility: ActionVisibility::Always,
        empty: None,
        style: Default::default(),
    }
}

impl gpui::Render for Golden {
    fn render(&mut self, window: &mut gpui::Window, _cx: &mut gpui::Context<Self>) -> impl gpui::IntoElement {
        use gpui::{IntoElement, ParentElement, Styled};
        let theme = theme_for(self.scheme);
        let viewport = window.viewport_size();
        let banner = BannerProps {
            title: "An update is ready to install".into(),
            action: Some("Restart".into()),
            revealed: true,
            style: Default::default(),
        };
        let toast = toast("deleted", "Note deleted").action("Undo");
        let mut root = gpui::div()
            .size_full()
            .flex()
            .flex_row()
            .bg(theme.surface.base)
            .text_color(theme.text.primary)
            .child(gpui::div().w(gpui::px(240.)).h_full().bg(theme.surface.sidebar).child(source_tree_view(
                NodeId(1),
                &golden_tree(),
                None,
                &theme,
                None,
                &Marks::default(),
            )))
            .child(
                gpui::div()
                    .flex_1()
                    .h_full()
                    .flex()
                    .flex_col()
                    .child(banner_view(NodeId(2), &banner, &theme, None, &Marks::default()))
                    .child(gpui::div().p(gpui::px(24.)).child("Q3 planning")),
            )
            .child(toast_view(Some(&toast), &theme, viewport, false, None, &Marks::default()));
        if let GoldenView::Palette = self.view {
            let mut model = PaletteModel::default();
            model.set_items(&items("tri"));
            model.present();
            model.edited("tri");
            model.set_items(&items("tri"));
            root = root.child(palette_overlay(
                &model,
                Some("Search notes"),
                &theme,
                viewport,
                None,
                None,
                &Marks::default(),
            ));
        }
        root.into_any_element()
    }
}

fn os() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "linux"
    }
}

fn golden_path(view: GoldenView, scheme: Scheme) -> PathBuf {
    let view = match view {
        GoldenView::Sidebar => "sidebar",
        GoldenView::Palette => "palette",
    };
    let scheme = if scheme.is_dark() { "dark" } else { "light" };
    Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/goldens/nav-{view}-{scheme}-{}.png", os()))
}

/// Per channel, out of 255, and the share of pixels allowed past it: antialiasing drifts.
const CHANNEL_TOLERANCE: i16 = 24;
const PIXEL_TOLERANCE: f64 = 0.004;

fn golden(view: GoldenView, scheme: Scheme) -> Result<(), String> {
    use gpui::AppContext as _;
    let mut cx = headless();
    let handle = cx
        .open_window(gpui::size(gpui::px(800.), gpui::px(520.)), |_, cx| cx.new(|_| Golden { view, scheme }))
        .map_err(|e| e.to_string())?;
    cx.run_until_parked();
    let image = cx.capture_screenshot(handle.into()).map_err(|e| e.to_string())?;
    let path = golden_path(view, scheme);
    if std::env::var_os("ND_UPDATE_GOLDENS").is_some_and(|v| v == "1") || !path.exists() {
        std::fs::create_dir_all(path.parent().expect("dir")).map_err(|e| e.to_string())?;
        image.save(&path).map_err(|e| e.to_string())?;
        println!("recorded {}", path.display());
        return Ok(());
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
        ("gate_m6::gpui_palette_types_completes_and_activates", gate_m6::gpui_palette_types_completes_and_activates),
        ("gate_m6::gpui_toasts_time_out_in_order", gate_m6::gpui_toasts_time_out_in_order),
        ("gate_m6::gpui_nav_goldens", gate_m6::gpui_nav_goldens),
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
