//! Tier 2: the drawn layout and window widgets on the GPUI backend inside GPUI's headless app:
//! the split's divider drag, `visibility: Hidden`, the status page, drawn alerts and the about
//! panel, with input through the window's own pointer and key paths; and their goldens, light
//! and dark, captured from the host's own window.
//!
//! `ND_UPDATE_GOLDENS=1` rewrites the goldens: `tests/goldens/<os>/layout-<view>-<scheme>.png`.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use nd_backend_gpui::{GpuiBackend, GpuiChrome, GpuiHost};
use nd_components::services::{self, Chrome};
use nd_components::split::Axis;
use nd_components::*;
use nd_gpui::gpui::HeadlessAppContext;
use nd_theme::{OsTheme, Preset, Scheme, Theme, ThemeRegistry};
use nd_ui::backend::Backend;
use nd_ui::prelude::*;
use nd_ui::semantics::{MouseButton, PaintKind, PointerPhase, SyntheticInput};
use nd_ui::{Keystroke, Point, SemanticNode, WindowId};

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

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Split,
    Hidden,
    Status,
    /// Nothing but a label, for the dialogs.
    Plain,
    /// A toolbar: its own title or none, and an app item before the flexible space or none.
    Bar {
        title: Option<&'static str>,
        leading_item: bool,
    },
}

#[derive(Default)]
struct Log {
    ratios: Vec<f32>,
    clicks: Vec<&'static str>,
    answers: Vec<usize>,
}

struct Scene {
    kind: Kind,
    log: Rc<RefCell<Log>>,
    ratio: f32,
    /// The app keeps its ratio whatever the user drags.
    refuse: bool,
}

impl Render for Scene {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let root = v_flex().size_full();
        match self.kind {
            Kind::Split => root.child(
                div().w_full().h(px(300.)).child(
                    split("split", Axis::Horizontal)
                        .label("Panes")
                        .ratio(self.ratio)
                        .first_pane(split_pane().min_size(px(120.)).child(text("Notes")))
                        .second_pane(split_pane().min_size(px(200.)).child(text("Editor")))
                        .on_resize(cx.listener(|this, e: &SplitResized, _, cx| {
                            this.log.borrow_mut().ratios.push(e.ratio);
                            if !this.refuse {
                                this.ratio = e.ratio;
                            }
                            cx.notify();
                        })),
                ),
            ),
            Kind::Hidden => root.child(
                div()
                    .relative()
                    .w(px(300.))
                    .h(px(200.))
                    .child(button("under", "Under").on_click(cx.listener(|this, _, _, _| {
                        this.log.borrow_mut().clicks.push("under");
                    })))
                    .child(div().id("page").absolute().top_0().left_0().size_full().invisible().child(
                        button("over", "Over").on_click(cx.listener(|this, _, _, _| {
                            this.log.borrow_mut().clicks.push("over");
                        })),
                    )),
            ),
            Kind::Status => root.child(
                status_page("No notes")
                    .id("status")
                    .icon(Icon::Search)
                    .description("Notes you create show up here. Try another search, or start a new note.")
                    .child(button("new", "New Note")),
            ),
            Kind::Plain => root.child(text("Inbox")),
            Kind::Bar { title, leading_item } => {
                let bar = toolbar().sidebar_toggle();
                let bar = match title {
                    Some(title) => bar.title(title),
                    None => bar,
                };
                let bar = if leading_item { bar.item(button("new", "New")) } else { bar };
                return scaffold()
                    .toolbar(bar.flexible_space().item(button("search", "Search")))
                    .content(text("Body"))
                    .into_any_element();
            }
        }
        .into_any_element()
    }
}

struct Opened {
    cx: HeadlessAppContext,
    host: GpuiHost,
    window: WindowId,
    log: Rc<RefCell<Log>>,
    view: Entity<Scene>,
}

fn registry(scheme: Scheme) -> ThemeRegistry {
    // The GPUI look on every OS, in the OS's own fonts, as `nd` sets it up at launch.
    let mut registry = ThemeRegistry::new(Preset::Adwaita);
    let os = OsTheme { scheme: Some(scheme), fonts: nd_theme::fonts::resolve(), ..registry.os().clone() };
    registry.set_os(os);
    registry
}

fn open_with(kind: Kind, scheme: Scheme) -> Opened {
    let mut cx = headless();
    let log: Rc<RefCell<Log>> = Rc::default();
    let l = log.clone();
    let mut opened = None;
    let host = cx.update(|cx| {
        GpuiHost::builder().quit_when_empty(false).start(cx, |app| {
            let backend = app.backend_as::<GpuiBackend>().expect("GPUI backend").clone();
            backend.set_theme(registry(scheme));
            app.set_global(Chrome(GpuiChrome::new(backend)));
            let options = WindowOptions::main("layout").size(size(px(800.), px(560.)));
            let options = if matches!(kind, Kind::Bar { .. }) { options.title("Notes") } else { options };
            let handle =
                app.open_window(options, |_, cx| cx.new(|_| Scene { kind, log: l, ratio: 0.5, refuse: false }));
            opened = Some((handle.id(), handle.root().clone()));
        })
    });
    settle(&mut cx, &host);
    let (window, view) = opened.expect("window");
    Opened { cx, host, window, log, view }
}

fn open(kind: Kind) -> Opened {
    open_with(kind, Scheme::Light)
}

impl Opened {
    fn node(&self, id: &str) -> SemanticNode {
        self.host.read(|app| {
            let tree = app.semantic_window(self.window).expect("window");
            find(&tree.root, &|n| n.test_id.as_deref() == Some(id)).cloned().unwrap_or_else(|| panic!("no {id}"))
        })
    }

    fn input(&mut self, inputs: Vec<SyntheticInput>) {
        let window = self.window;
        self.cx.update(|cx| {
            self.host.update(cx, |app| {
                for input in &inputs {
                    app.synthesize(window, input).expect("posted");
                }
            })
        });
        settle(&mut self.cx, &self.host);
    }

    fn keys(&mut self, keys: &[&str]) {
        for key in keys {
            self.input(vec![SyntheticInput::Key(Keystroke::parse(key).expect("keystroke"))]);
        }
    }

    fn click(&mut self, at: Point) {
        let inputs = [PointerPhase::Down, PointerPhase::Up]
            .into_iter()
            .map(|phase| SyntheticInput::Pointer { phase, at, button: MouseButton::Left, click_count: 1 })
            .collect();
        self.input(inputs);
    }

    /// The divider's centre: the first pane's trailing edge plus half the hairline.
    fn divider(&self) -> Point {
        let first = self.node("first").bounds.expect("first pane laid out");
        Point { x: first.origin.x + first.size.width + 0.5, y: first.origin.y + first.size.height / 2. }
    }

    fn drag_divider_to(&mut self, x: f32) {
        let from = self.divider();
        self.input(vec![SyntheticInput::Drag { from, to: Point { x, y: from.y }, steps: 8 }]);
    }

    fn ratios(&self) -> Vec<f32> {
        std::mem::take(&mut self.log.borrow_mut().ratios)
    }

    fn alert(&mut self, alert: Alert) {
        let (window, log) = (self.window, self.log.clone());
        self.cx.update(|cx| {
            self.host.update(cx, |app| {
                services::alert(app, window, alert, move |r, _| log.borrow_mut().answers.push(r.0));
            })
        });
        settle(&mut self.cx, &self.host);
    }

    fn about(&mut self, about: AboutPanel) {
        let window = self.window;
        self.cx.update(|cx| self.host.update(cx, |app| services::about(app, Some(window), about)));
        settle(&mut self.cx, &self.host);
    }

    fn showing(&self) -> Option<String> {
        self.host.backend().showing_dialog(self.window).map(|s| s.to_string())
    }

    fn theme(&self) -> Theme {
        registry(Scheme::Light).app_theme().clone()
    }
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.002
}

mod gate_m6 {
    use super::*;

    pub fn gpui_split_drag_reports_ratio_within_min_sizes() {
        let mut o = open(Kind::Split);
        let split = o.node("split").bounds.expect("split");
        assert_eq!(split.size.width, 800., "the split fills its row");
        let available = split.size.width - 1.;
        let start = o.divider();
        assert!(close((start.x - 0.5) / available, 0.5), "opens at the ratio: divider at {}", start.x);

        o.drag_divider_to(500.);
        let ratios = o.ratios();
        assert!(ratios.len() > 1, "reports while dragging: {ratios:?}");
        let last = *ratios.last().expect("a ratio");
        assert!(close(last, 499.5 / available), "{last} for a divider at 500");
        let mut dedup = ratios.clone();
        dedup.dedup();
        assert_eq!(dedup, ratios, "only real changes are reported");
        assert!(close((o.divider().x - 0.5) / available, last), "the app fed the ratio back");

        o.drag_divider_to(790.);
        let last = *o.ratios().last().expect("a ratio");
        assert!(close(last, (available - 200.) / available), "the second pane keeps its 200 pt: {last}");
        o.drag_divider_to(5.);
        let last = *o.ratios().last().expect("a ratio");
        assert!(close(last, 120. / available), "the first pane keeps its 120 pt: {last}");
        let first = o.node("first").bounds.expect("first");
        assert!((first.size.width - 120.).abs() < 1., "drawn at the clamped size: {}", first.size.width);
    }

    pub fn gpui_split_echo_and_refusal() {
        let mut o = open(Kind::Split);
        o.drag_divider_to(450.);
        assert!(!o.ratios().is_empty());
        for _ in 0..3 {
            settle(&mut o.cx, &o.host);
        }
        assert_eq!(o.ratios(), Vec::<f32>::new(), "feeding the ratio back reports nothing");
        let view = o.view.clone();
        o.cx.update(|cx| {
            o.host.update(cx, |app| {
                view.update(app, |this, cx| {
                    this.ratio = 0.3;
                    cx.notify();
                })
            })
        });
        settle(&mut o.cx, &o.host);
        assert_eq!(o.ratios(), Vec::<f32>::new(), "an app-set ratio reports nothing");
        assert!(close((o.divider().x - 0.5) / 799., 0.3), "drawn at the app's ratio");

        o.cx.update(|cx| o.host.update(cx, |app| view.update(app, |this, _| this.refuse = true)));
        o.drag_divider_to(600.);
        assert!(!o.ratios().is_empty(), "the drag reported");
        assert!(close((o.divider().x - 0.5) / 799., 0.3), "a refused ratio snaps back");
    }

    pub fn gpui_split_hidden_subtree_is_not_hit_or_painted() {
        let mut o = open(Kind::Hidden);
        let under = o.node("under").bounds.expect("under");
        assert!(!o.node("over").visible, "the hidden page's button is not visible");
        o.click(Point { x: under.origin.x + under.size.width / 2., y: under.origin.y + under.size.height / 2. });
        assert_eq!(o.log.borrow().clicks, ["under"], "the click passes the hidden page to what lies under it");
        let paints = o.host.backend().paints(o.window).expect("paints");
        assert!(paints.iter().any(|p| p.label == "Under"), "the visible button painted");
        assert!(!paints.iter().any(|p| p.label == "Over"), "nothing in the hidden page painted");
    }

    pub fn gpui_status_page_lays_out_centred() {
        let o = open(Kind::Status);
        let status = o.node("status").bounds.expect("status");
        let centre = status.origin.x + status.size.width / 2.;
        assert!((centre - 400.).abs() < 1., "centred across the window: {centre}");
        assert!(status.size.width <= 600., "no wider than the readable width: {}", status.size.width);
        let button = o.node("new").bounds.expect("action");
        assert!(button.origin.y >= status.origin.y + status.size.height, "the action sits under the header");
        let paints = o.host.backend().paints(o.window).expect("paints");
        let title = paints.iter().find(|p| p.label == "No notes").expect("title painted");
        assert!(matches!(title.kind, PaintKind::Text { size, .. } if size == 20.), "title-1: {:?}", title.kind);
        assert!(paints.iter().any(|p| p.label == "status page icon"), "the icon slot painted");
    }

    pub fn gpui_toolbar_shows_a_title_in_an_empty_leading_run() {
        let titles = |kind| {
            let o = open(kind);
            let paints = o.host.backend().paints(o.window).expect("paints");
            ["Notes", "Inbox"].into_iter().filter(|t| paints.iter().any(|p| p.label == *t)).collect::<Vec<_>>()
        };
        assert_eq!(titles(Kind::Bar { title: None, leading_item: false }), ["Notes"], "the window's title by default");
        assert!(titles(Kind::Bar { title: None, leading_item: true }).is_empty(), "not over the app's own items");
        assert_eq!(titles(Kind::Bar { title: Some("Inbox"), leading_item: true }), ["Inbox"], "the toolbar's own");
        assert!(titles(Kind::Bar { title: Some(""), leading_item: false }).is_empty(), "an empty title hides it");
    }

    pub fn gpui_alert_keys_answer_once() {
        let mut o = open(Kind::Plain);
        o.alert(Alert::confirm("Delete the note?", "Delete").message("You cannot undo this."));
        assert_eq!(o.showing().as_deref(), Some("Delete the note?"));
        o.keys(&["tab", "enter"]);
        assert_eq!(o.log.borrow().answers, [1], "Tab moved to Cancel, Return pressed it");
        assert_eq!(o.showing(), None);
        o.keys(&["enter"]);
        assert_eq!(o.log.borrow().answers, [1], "answered once");

        o.alert(Alert::new("Save changes?").cancel("Cancel").button("Save"));
        o.keys(&["enter"]);
        assert_eq!(o.log.borrow().answers, [1, 1], "Return presses the default button");

        o.alert(Alert::new("Save changes?").button("Save").cancel("Cancel").button("Discard"));
        o.keys(&["shift-tab", "shift-tab", "escape"]);
        assert_eq!(o.log.borrow().answers, [1, 1, 1], "Esc answers the cancel button");

        o.alert(Alert::new("Copied"));
        o.keys(&["escape"]);
        assert_eq!(o.log.borrow().answers, [1, 1, 1, 0], "an alert with no buttons answers its OK");
        assert_eq!(o.showing(), None);
    }

    pub fn gpui_alert_destructive_paints_danger() {
        let mut o = open(Kind::Plain);
        o.alert(Alert::confirm("Delete the note?", "Delete"));
        let theme = o.theme();
        let paints = o.host.backend().paints(o.window).expect("paints");
        let fill = paints.iter().find(|p| p.label == "destructive button").expect("destructive button");
        assert_eq!(fill.color, theme.danger);
        assert!(paints.iter().any(|p| p.label == "dialog scrim" && p.color == theme.scrim));
        o.keys(&["escape"]);
        assert_eq!(o.log.borrow().answers, [1]);
        let paints = o.host.backend().paints(o.window).expect("paints");
        assert!(!paints.iter().any(|p| p.label == "dialog scrim"), "the scrim went with the alert");
    }

    pub fn gpui_alert_queues_and_blocks_input() {
        let mut o = open(Kind::Split);
        o.alert(Alert::new("First").button("OK"));
        o.alert(Alert::new("Second").button("OK"));
        assert_eq!(o.showing().as_deref(), Some("First"), "one alert per window");
        o.drag_divider_to(600.);
        assert_eq!(o.ratios(), Vec::<f32>::new(), "the scrim takes the drag");
        o.keys(&["enter"]);
        assert_eq!(o.showing().as_deref(), Some("Second"), "the next one shows");
        o.keys(&["enter"]);
        assert_eq!(o.log.borrow().answers, [0, 0]);
        o.drag_divider_to(600.);
        assert!(!o.ratios().is_empty(), "input reaches the window again");
    }

    pub fn gpui_alert_modal_covers_every_window() {
        let mut o = open(Kind::Plain);
        let log = o.log.clone();
        let second = o.cx.update(|cx| {
            o.host.update(cx, |app| {
                app.open_window(WindowOptions::main("second").size(size(px(400.), px(300.))), |_, cx| {
                    cx.new(|_| Scene { kind: Kind::Split, log, ratio: 0.5, refuse: false })
                })
                .id()
            })
        });
        settle(&mut o.cx, &o.host);
        o.alert(Alert::new("Quit Notes?").button("Quit").cancel("Cancel").modal());
        assert_eq!(o.showing().as_deref(), Some("Quit Notes?"));
        assert_eq!(o.host.backend().showing_dialog(second), None, "the card shows on the asking window only");
        let paints = o.host.backend().paints(second).expect("paints");
        assert!(paints.iter().any(|p| p.label == "dialog scrim"), "the other window is covered");
        let escape = Keystroke::parse("escape").expect("keystroke");
        o.cx.update(|cx| {
            o.host.update(cx, |app| app.synthesize(second, &SyntheticInput::Key(escape)).expect("posted"))
        });
        settle(&mut o.cx, &o.host);
        assert_eq!(o.log.borrow().answers, [1], "a key in the covered window reaches the modal alert");
        let paints = o.host.backend().paints(second).expect("paints");
        assert!(!paints.iter().any(|p| p.label == "dialog scrim"), "every scrim went with it");
    }

    pub fn gpui_alert_about_opens_and_closes() {
        let mut o = open(Kind::Plain);
        o.about(AboutPanel::new().name("Notes").version("1.2").copyright("© 2026 CanaryCoders"));
        assert_eq!(o.showing().as_deref(), Some("Notes"));
        let paints = o.host.backend().paints(o.window).expect("paints");
        assert!(paints.iter().any(|p| p.label == "1.2"), "the version painted");
        o.keys(&["enter"]);
        assert_eq!(o.showing().as_deref(), Some("Notes"), "Return leaves it open");
        o.keys(&["escape"]);
        assert_eq!(o.showing(), None, "Esc closes it");
    }

    pub fn gpui_layout_goldens() {
        // GPUI renders headless only on macOS (Metal) and Linux (wgpu); Windows captures arrive
        // with the Fluent goldens (M3).
        if nd_gpui::gpui_platform::current_headless_renderer().ok().flatten().is_none() {
            println!("skipped: GPUI has no headless renderer on this OS");
            return;
        }
        let mut failed = Vec::new();
        for scheme in [Scheme::Light, Scheme::Dark] {
            for view in ["split", "status", "alert", "about"] {
                if let Err(e) = golden(view, scheme) {
                    failed.push(e);
                }
            }
        }
        assert!(failed.is_empty(), "{failed:#?}");
    }
}

fn golden_path(view: &str, scheme: Scheme) -> PathBuf {
    let scheme = if scheme.is_dark() { "dark" } else { "light" };
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(format!("tests/goldens/{}/layout-{view}-{scheme}.png", std::env::consts::OS))
}

/// Per channel, out of 255, and the share of pixels allowed past it: antialiasing drifts.
const CHANNEL_TOLERANCE: i16 = 24;
const PIXEL_TOLERANCE: f64 = 0.004;

fn golden(view: &str, scheme: Scheme) -> Result<(), String> {
    let kind = match view {
        "split" => Kind::Split,
        "status" => Kind::Status,
        _ => Kind::Plain,
    };
    let mut o = open_with(kind, scheme);
    match view {
        "alert" => o.alert(
            Alert::confirm("Delete the note?", "Delete").message("The note and its attachments are removed for good."),
        ),
        "about" => o.about(
            AboutPanel::new()
                .name("Notes")
                .version("1.2.0")
                .credits("Plain notes that sync everywhere.")
                .website("https://example.com/notes")
                .copyright("© 2026 CanaryCoders"),
        ),
        _ => {}
    }
    let handle = o.host.window_handle(o.window).ok_or("no window")?;
    let image = o.cx.capture_screenshot(handle).map_err(|e| e.to_string())?;
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
        (
            "gate_m6::gpui_split_drag_reports_ratio_within_min_sizes",
            gate_m6::gpui_split_drag_reports_ratio_within_min_sizes,
        ),
        ("gate_m6::gpui_split_echo_and_refusal", gate_m6::gpui_split_echo_and_refusal),
        (
            "gate_m6::gpui_split_hidden_subtree_is_not_hit_or_painted",
            gate_m6::gpui_split_hidden_subtree_is_not_hit_or_painted,
        ),
        ("gate_m6::gpui_status_page_lays_out_centred", gate_m6::gpui_status_page_lays_out_centred),
        (
            "gate_m6::gpui_toolbar_shows_a_title_in_an_empty_leading_run",
            gate_m6::gpui_toolbar_shows_a_title_in_an_empty_leading_run,
        ),
        ("gate_m6::gpui_alert_keys_answer_once", gate_m6::gpui_alert_keys_answer_once),
        ("gate_m6::gpui_alert_destructive_paints_danger", gate_m6::gpui_alert_destructive_paints_danger),
        ("gate_m6::gpui_alert_queues_and_blocks_input", gate_m6::gpui_alert_queues_and_blocks_input),
        ("gate_m6::gpui_alert_modal_covers_every_window", gate_m6::gpui_alert_modal_covers_every_window),
        ("gate_m6::gpui_alert_about_opens_and_closes", gate_m6::gpui_alert_about_opens_and_closes),
        ("gate_m6::gpui_layout_goldens", gate_m6::gpui_layout_goldens),
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
