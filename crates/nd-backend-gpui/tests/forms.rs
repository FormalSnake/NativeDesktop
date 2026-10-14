//! Tier 2: the drawn forms controls on the GPUI backend inside GPUI's headless app. Pointer and
//! key input go through the window's own input path (`App::synthesize`), automation through
//! `App::perform`; checks read the app's state and the events it heard. Goldens compare the
//! gallery in light and dark with `tests/goldens/forms-{light,dark}.png` (`ND_UPDATE_GOLDENS=1`
//! records them; a mismatch writes `<golden>.actual.png`).

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use nd_backend_gpui::GpuiHost;
use nd_components::forms::*;
use nd_gpui::gpui::{HeadlessAppContext, WindowAppearance};
use nd_theme::{OsTheme, Preset, Scheme, ThemeRegistry};
use nd_ui::prelude::*;
use nd_ui::semantics::{ActionError, MouseButton, PointerPhase, SemanticAction, SyntheticInput};
use nd_ui::{Keystroke, NodeId, Point, SemanticNode, WindowId};

type Log = Rc<RefCell<Vec<String>>>;

struct Gallery {
    log: Log,
    animated: bool,
    text: SharedString,
    secure: SharedString,
    search: SharedString,
    area: SharedString,
    combo: SharedString,
    nd: SharedString,
    check: bool,
    mixed: bool,
    on: bool,
    select: usize,
    seg: usize,
    radio: usize,
    slider: f32,
    stepper: f64,
    date: CivilDateTime,
    day: CivilDateTime,
    color: Hsla,
}

impl Gallery {
    fn new(log: Log) -> Self {
        Self {
            log,
            animated: false,
            text: "".into(),
            secure: "".into(),
            search: "".into(),
            area: "First line\nSecond line".into(),
            combo: "".into(),
            nd: "".into(),
            check: false,
            mixed: true,
            on: false,
            select: 1,
            seg: 0,
            radio: 0,
            slider: 0.25,
            stepper: 3.,
            date: CivilDateTime::date(2026, 10, 13),
            day: CivilDateTime::date(2026, 10, 13),
            color: rgb(0x3584e4).into(),
        }
    }
}

/// A handler that logs `name:value`, applies the value and re-renders. Applying inside the
/// handler is the reentrancy case: the new prop must not make the control report again.
macro_rules! on {
    ($cx:ident, $ev:ty, $name:literal, |$this:ident, $e:ident| $value:expr $(, $apply:expr)?) => {
        $cx.listener(move |$this: &mut Gallery, $e: &$ev, _window, cx| {
            let shown = format!("{}:{}", $name, $value);
            $this.log.borrow_mut().push(shown);
            $($apply;)?
            cx.notify();
        })
    };
}

impl Render for Gallery {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let row = || h_flex().gap(px(12.)).items_center();
        v_flex()
            .size_full()
            .p(px(16.))
            .gap(px(12.))
            .child(
                row()
                    .child(
                        text_field("text")
                            .text(self.text.clone())
                            .placeholder("Name")
                            .w(px(180.))
                            .on_change(on!(cx, TextChanged, "text", |this, e| e.text, this.text = e.text.clone()))
                            .on_submit(on!(cx, TextSubmitted, "submit", |this, e| e.text)),
                    )
                    .child(
                        secure_field("secure")
                            .text(self.secure.clone())
                            .placeholder("Password")
                            .w(px(150.))
                            .on_change(on!(cx, TextChanged, "secure", |this, e| e.text, this.secure = e.text.clone())),
                    )
                    .child(
                        text_field("search")
                            .search()
                            .text(self.search.clone())
                            .placeholder("Search")
                            .w(px(150.))
                            .on_change(on!(cx, TextChanged, "search", |this, e| e.text, this.search = e.text.clone())),
                    ),
            )
            .child(
                row()
                    .child(text_area("area").text(self.area.clone()).min_lines(3).w(px(220.)).on_change(on!(
                        cx,
                        TextChanged,
                        "area",
                        |this, e| e.text,
                        this.area = e.text.clone()
                    )))
                    .child(
                        v_flex()
                            .gap(px(8.))
                            .child(
                                combo_box("combo", ["Apple", "Banana", "Cherry"])
                                    .text(self.combo.clone())
                                    .placeholder("Fruit")
                                    .w(px(170.))
                                    .on_change(on!(
                                        cx,
                                        TextChanged,
                                        "combo",
                                        |this, e| e.text,
                                        this.combo = e.text.clone()
                                    ))
                                    .on_select(on!(cx, SelectChanged, "pick", |this, e| e.index)),
                            )
                            .child(
                                text_field("nd")
                                    .text(self.nd.clone())
                                    .placeholder("nd focus")
                                    .focusable()
                                    .w(px(170.))
                                    .on_change(on!(cx, TextChanged, "nd", |this, e| e.text, this.nd = e.text.clone())),
                            ),
                    ),
            )
            .child(
                row()
                    .child(checkbox("check", "Check").checked(self.check).on_toggle(on!(
                        cx,
                        Toggled,
                        "check",
                        |this, e| e.checked,
                        this.check = e.checked
                    )))
                    .child(checkbox("mixed", "Mixed").checked(false).mixed(self.mixed).on_toggle(on!(
                        cx,
                        Toggled,
                        "mixed",
                        |this, e| e.checked,
                        this.mixed = false
                    )))
                    .child(
                        checkbox("off", "Disabled")
                            .disabled(true)
                            .on_toggle(on!(cx, Toggled, "off", |this, e| e.checked)),
                    )
                    .child(switch("switch").label("Wi-Fi").checked(self.on).on_toggle(on!(
                        cx,
                        Toggled,
                        "switch",
                        |this, e| e.checked,
                        this.on = e.checked
                    )))
                    .child(switch("switch-on").label("On").checked(true)),
            )
            .child(
                row()
                    .child(
                        select("select", ["Name", "Date", "Size"])
                            .selected(self.select)
                            .label("Sort")
                            .w(px(120.))
                            .on_change(on!(cx, SelectChanged, "select", |this, e| e.index, this.select = e.index)),
                    )
                    .child(segmented("seg", ["Day", "Week", "Month"]).selected(self.seg).label("Range").on_change(on!(
                        cx,
                        SelectChanged,
                        "seg",
                        |this, e| e.index,
                        this.seg = e.index
                    )))
                    .child(
                        radio_group("radio", ["One", "Two"])
                            .horizontal()
                            .selected(self.radio)
                            .label("Count")
                            .on_change(on!(cx, SelectChanged, "radio", |this, e| e.index, this.radio = e.index)),
                    ),
            )
            .child(
                row()
                    .child(
                        slider("slider", self.slider)
                            .range(0.0..=1.0)
                            .step(0.25)
                            .ticks(5)
                            .label("Volume")
                            .w(px(200.))
                            .on_change(on!(cx, SliderChanged, "slider", |this, e| e.value, this.slider = e.value)),
                    )
                    .child(stepper("stepper", self.stepper).range(0.0..=5.0).label("Copies").on_change(on!(
                        cx,
                        StepperChanged,
                        "stepper",
                        |this, e| e.value,
                        this.stepper = e.value
                    )))
                    .child(stepper("stepper-field", 2.5).with_field(1).label("Scale")),
            )
            .child(
                row()
                    .child(
                        date_picker("date", self.date)
                            .label("Due")
                            .presentation(DatePickerStyle::Stepper)
                            .w(px(150.))
                            .on_change(on!(cx, DateChanged, "date", |this, e| e.value, this.date = e.value)),
                    )
                    .child(color_well("color", self.color).label("Tint").on_change(on!(
                        cx,
                        ColorChanged,
                        "color",
                        |this, e| e.color,
                        this.color = e.color
                    )))
                    .child(link("link", "Read the docs").url("https://example.com").keep_internal().on_click(on!(
                        cx,
                        LinkActivated,
                        "link",
                        |this, e| e.url.clone().unwrap_or_default()
                    ))),
            )
            .child(
                row()
                    .child(
                        date_picker("day", self.day)
                            .label("Day")
                            .presentation(DatePickerStyle::Graphical)
                            .on_change(on!(cx, DateChanged, "day", |this, e| e.value, this.day = e.value)),
                    )
                    .child(
                        v_flex()
                            .gap(px(8.))
                            .child(label("Settings").text_style(TextStyle::Title2))
                            .child(label("Supporting text").secondary())
                            .child(label("Something went wrong").tone(LabelTone::Danger))
                            .child(progress(Some(0.4)).id("bar").label("Copying"))
                            .child(progress(Some(0.65)).circular().id("ring").label("Upload"))
                            .when(self.animated, |d| d.child(spinner().id("spinner")).child(progress(None).id("busy"))),
                    ),
            )
    }
}

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

struct Rig {
    cx: HeadlessAppContext,
    host: GpuiHost,
    window: WindowId,
    log: Log,
    view: Rc<RefCell<Option<nd_ui::Entity<Gallery>>>>,
}

const SIZE: (f32, f32) = (680., 760.);

fn open_with(appearance: WindowAppearance, animated: bool) -> Rig {
    let mut cx = headless();
    let log: Log = Rc::default();
    let view = Rc::new(RefCell::new(None));
    let (l, v) = (log.clone(), view.clone());
    let mut window = None;
    let host = cx.update(|cx| {
        GpuiHost::builder().quit_when_empty(false).start(cx, |app| {
            let handle = app.open_window(
                WindowOptions::main("forms").title("Forms").size(size(px(SIZE.0), px(SIZE.1))),
                |_, cx| {
                    let entity = cx.new(|_| Gallery { animated, ..Gallery::new(l) });
                    *v.borrow_mut() = Some(entity.clone());
                    entity
                },
            );
            window = Some(handle.id());
        })
    });
    // The windows paint from the theme registry; pin its scheme as the OS would report it.
    let mut registry = ThemeRegistry::new(Preset::current());
    let scheme = if matches!(appearance, WindowAppearance::Dark) { Scheme::Dark } else { Scheme::Light };
    registry.set_os(OsTheme { scheme: Some(scheme), ..registry.os().clone() });
    host.backend().set_theme(registry);
    settle(&mut cx, &host);
    Rig { cx, host, window: window.expect("window"), log, view }
}

fn open() -> Rig {
    open_with(WindowAppearance::Light, false)
}

fn find<'a>(node: &'a SemanticNode, f: &impl Fn(&SemanticNode) -> bool) -> Option<&'a SemanticNode> {
    if f(node) {
        return Some(node);
    }
    node.children.iter().find_map(|c| find(c, f))
}

impl Rig {
    fn node(&self, id: &str) -> SemanticNode {
        self.host.read(|app| {
            let tree = app.semantic_window(self.window).expect("window");
            find(&tree.root, &|n| n.test_id.as_deref() == Some(id)).cloned().unwrap_or_else(|| panic!("no {id}"))
        })
    }

    fn id(&self, id: &str) -> NodeId {
        self.node(id).id
    }

    fn settle(&mut self) {
        settle(&mut self.cx, &self.host);
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
        self.settle();
    }

    /// A left click at `at`, a fraction of the node's bounds.
    fn click_at(&mut self, id: &str, fx: f32, fy: f32) {
        let b = self.node(id).bounds.unwrap_or_else(|| panic!("{id} laid out"));
        let at = Point { x: b.origin.x + b.size.width * fx, y: b.origin.y + b.size.height * fy };
        let inputs = [PointerPhase::Down, PointerPhase::Up]
            .into_iter()
            .map(|phase| SyntheticInput::Pointer { phase, at, button: MouseButton::Left, click_count: 1 })
            .collect();
        self.input(inputs);
    }

    fn click(&mut self, id: &str) {
        self.click_at(id, 0.5, 0.5);
    }

    fn keys(&mut self, keys: &[&str]) {
        let inputs = keys.iter().map(|k| SyntheticInput::Key(Keystroke::parse(k).expect("keystroke"))).collect();
        self.input(inputs);
    }

    fn perform(&mut self, id: &str, action: SemanticAction) -> Result<(), ActionError> {
        let node = self.id(id);
        let result = self.cx.update(|cx| self.host.update(cx, |app| app.perform(node, &action)));
        self.settle();
        result
    }

    fn take(&self) -> Vec<String> {
        std::mem::take(&mut *self.log.borrow_mut())
    }

    fn read<R>(&mut self, f: impl FnOnce(&Gallery) -> R) -> R {
        let entity = self.view.borrow().clone().expect("view");
        self.host.read(|app| f(entity.read(app)))
    }
}

/// Settling again produces nothing: the value the app fed back is no new report.
fn quiet(rig: &mut Rig) {
    rig.settle();
    assert_eq!(rig.take(), Vec::<String>::new(), "no echo after the app applied the value");
}

mod forms {
    use super::*;

    pub fn checkbox_click_and_space_toggle() {
        let mut rig = open();
        rig.click("check");
        assert_eq!(rig.take(), ["check:true"]);
        assert!(rig.read(|g| g.check));
        assert_eq!(rig.node("check").checked, Some(true));
        quiet(&mut rig);
        rig.keys(&["space"]);
        assert_eq!(rig.take(), ["check:false"], "Space toggles the focused checkbox");
        quiet(&mut rig);
        rig.click("mixed");
        assert_eq!(rig.take(), ["mixed:true"], "a mixed checkbox checks");
    }

    pub fn switch_click_and_space_toggle() {
        let mut rig = open();
        rig.click("switch");
        assert_eq!(rig.take(), ["switch:true"]);
        quiet(&mut rig);
        rig.keys(&["space"]);
        assert_eq!(rig.take(), ["switch:false"]);
        quiet(&mut rig);
    }

    pub fn typing_arrives_as_text_changed() {
        let mut rig = open();
        rig.click("text");
        rig.keys(&["shift-h", "i"]);
        assert_eq!(rig.take(), ["text:H", "text:Hi"]);
        assert_eq!(rig.read(|g| g.text.clone()), "Hi");
        quiet(&mut rig);
        rig.keys(&["left", "shift-a"]);
        assert_eq!(rig.take(), ["text:HAi"], "the caret stayed where the user left it");
        rig.keys(&["backspace", "end", "!"]);
        assert_eq!(rig.take(), ["text:Hi", "text:Hi!"]);
        rig.keys(&["enter"]);
        assert_eq!(rig.take(), ["submit:Hi!"]);
        quiet(&mut rig);
        let select_all = if cfg!(target_os = "macos") { "cmd-a" } else { "ctrl-a" };
        rig.keys(&[select_all, "x"]);
        assert_eq!(rig.take(), ["text:x"], "select all, then typing replaces it");
    }

    pub fn an_app_write_replaces_the_text() {
        let mut rig = open();
        rig.click("text");
        rig.keys(&["a", "b", "c"]);
        rig.take();
        let entity = rig.view.borrow().clone().expect("view");
        rig.cx.update(|cx| {
            rig.host.update(cx, |app| {
                entity.update(app, |g, cx| {
                    g.text = "z".into();
                    cx.notify();
                })
            })
        });
        rig.settle();
        rig.keys(&["y"]);
        assert_eq!(rig.take(), ["text:zy"], "the caret was clamped to the new text's end");
    }

    pub fn secure_and_search_fields() {
        let mut rig = open();
        rig.click("secure");
        rig.keys(&["p", "w"]);
        assert_eq!(rig.take(), ["secure:p", "secure:pw"]);
        assert_eq!(rig.node("secure").value, None, "a secure field never reads back");
        rig.click("search");
        rig.keys(&["q"]);
        assert_eq!(rig.take(), ["search:q"]);
        // The clear button sits at the trailing edge.
        rig.click_at("search", 0.9, 0.5);
        assert_eq!(rig.take(), ["search:"]);
    }

    pub fn text_area_takes_newlines() {
        let mut rig = open();
        rig.perform("area", SemanticAction::Focus).expect("focus");
        let end = if cfg!(target_os = "macos") { "cmd-down" } else { "ctrl-end" };
        rig.keys(&[end, "enter", "x"]);
        assert_eq!(rig.take(), ["area:First line\nSecond line\n", "area:First line\nSecond line\nx"]);
        quiet(&mut rig);
    }

    pub fn combo_box_completes_and_picks() {
        let mut rig = open();
        rig.click_at("combo", 0.3, 0.5);
        rig.keys(&["b"]);
        assert_eq!(rig.take(), ["combo:Banana"], "typed text completes from the options");
        rig.keys(&["down", "down", "enter"]);
        assert_eq!(rig.take(), ["combo:Banana", "pick:1"]);
        quiet(&mut rig);
    }

    pub fn nd_focus_gives_the_field_gpui_focus() {
        let mut rig = open();
        let node = rig.id("nd");
        let window = rig.window;
        let moved = rig.cx.update(|cx| rig.host.update(cx, |app| app.focus(window, node)));
        assert!(moved, "the field is focusable in nd");
        rig.settle();
        rig.keys(&["k"]);
        assert_eq!(rig.take(), ["nd:k"]);
        // A click into another field moves nd focus off this one.
        rig.click("text");
        assert_ne!(rig.host.read(|app| app.focused(window)), Some(node));
    }

    pub fn slider_drag_reports_normalised_values() {
        let mut rig = open();
        let b = rig.node("slider").bounds.expect("slider");
        let y = b.origin.y + b.size.height / 4.;
        let drag = SyntheticInput::Drag {
            from: Point { x: b.origin.x + 4., y },
            to: Point { x: b.origin.x + b.size.width * 0.5, y },
            steps: 8,
        };
        rig.input(vec![drag]);
        let log = rig.take();
        assert!(!log.is_empty(), "the drag reported");
        assert_eq!(log.last().map(String::as_str), Some("slider:0.5"), "{log:?}");
        for entry in &log {
            let v: f32 = entry.trim_start_matches("slider:").parse().expect("number");
            assert_eq!((v * 4.).fract(), 0., "{v} is on the step");
        }
        let mut dedup = log.clone();
        dedup.dedup();
        assert_eq!(dedup, log, "no value reported twice in a row");
        quiet(&mut rig);
        rig.keys(&["right"]);
        assert_eq!(rig.take(), ["slider:0.75"], "an arrow key steps the focused slider");
        rig.keys(&["end"]);
        assert_eq!(rig.take(), ["slider:1"]);
        rig.keys(&["right"]);
        assert_eq!(rig.take(), Vec::<String>::new(), "no report past the end");
    }

    pub fn select_mappings() {
        let mut rig = open();
        rig.click("select");
        assert_eq!(rig.take(), Vec::<String>::new(), "a click opens the menu and picks nothing");
        rig.keys(&["down", "enter"]);
        assert_eq!(rig.take(), ["select:2"], "the open menu picks with the keyboard");
        assert_ne!(rig.node("select").expanded, Some(true), "picking with Return closes the menu");
        rig.click_at("seg", 0.83, 0.5);
        assert_eq!(rig.take(), ["seg:2"]);
        rig.keys(&["right"]);
        assert_eq!(rig.take(), ["seg:0"], "arrows wrap");
        rig.click_at("radio", 0.75, 0.5);
        assert_eq!(rig.take(), ["radio:1"]);
        rig.keys(&["left"]);
        assert_eq!(rig.take(), ["radio:0"]);
        quiet(&mut rig);
    }

    pub fn stepper_and_pickers() {
        let mut rig = open();
        rig.click_at("stepper", 0.75, 0.5);
        assert_eq!(rig.take(), ["stepper:4"]);
        rig.keys(&["up"]);
        assert_eq!(rig.take(), ["stepper:5"]);
        rig.keys(&["up"]);
        assert_eq!(rig.take(), Vec::<String>::new(), "no report past the maximum");
        rig.perform("date", SemanticAction::Focus).expect("focus");
        rig.keys(&["up"]);
        assert_eq!(rig.take(), ["date:2026-10-14T00:00:00"]);
        let select_all = if cfg!(target_os = "macos") { "cmd-a" } else { "ctrl-a" };
        let mut typed = vec![select_all];
        typed.extend(["2", "0", "2", "7", "-", "0", "1", "-", "0", "2", "enter"]);
        rig.keys(&typed);
        assert_eq!(rig.take(), ["date:2027-01-02T00:00:00"], "typed ISO text commits on Return");
        quiet(&mut rig);
        rig.perform("day", SemanticAction::Focus).expect("focus");
        rig.keys(&["right"]);
        rig.keys(&["down"]);
        assert_eq!(rig.take(), ["day:2026-10-14T00:00:00", "day:2026-10-21T00:00:00"]);
        rig.perform("color", SemanticAction::SetValue("#ff7800".into())).expect("set");
        assert_eq!(rig.take().len(), 1);
        assert_eq!(rig.node("color").value.as_deref(), Some("#ff7800"));
    }

    pub fn link_reports_activation() {
        let mut rig = open();
        rig.click("link");
        assert_eq!(rig.take(), ["link:https://example.com"]);
    }

    pub fn automation_set_value_reaches_the_app() {
        let mut rig = open();
        for (id, value, expected) in [
            ("text", "typed", "text:typed"),
            ("slider", "0.8", "slider:0.75"),
            ("select", "Size", "select:2"),
            ("stepper", "2", "stepper:2"),
            ("switch", "on", "switch:true"),
            ("date", "2026-12-24", "date:2026-12-24T00:00:00"),
        ] {
            rig.perform(id, SemanticAction::SetValue(value.into())).expect("set");
            assert_eq!(rig.take(), [expected]);
        }
        quiet(&mut rig);
        assert_eq!(rig.node("slider").value.as_deref(), Some("0.75"));
        assert_eq!(rig.node("text").value.as_deref(), Some("typed"));
    }

    pub fn disabled_controls_refuse() {
        let mut rig = open();
        rig.click("off");
        assert_eq!(rig.take(), Vec::<String>::new(), "a disabled checkbox ignores clicks");
        assert!(rig.perform("off", SemanticAction::Click).is_err(), "and automation");
        assert!(!rig.node("off").enabled);
        assert!(rig.perform("off", SemanticAction::Focus).is_err());
    }

    pub fn animated_progress_keeps_drawing() {
        let mut rig = open_with(WindowAppearance::Light, true);
        let spinner = rig.node("spinner").bounds.expect("spinner laid out");
        assert!(spinner.size.width > 0.);
        let busy = rig.node("busy");
        assert_eq!(busy.value, None, "indeterminate progress has no value");
        rig.settle();
    }

    pub fn goldens() {
        for (scheme, appearance) in [("light", WindowAppearance::Light), ("dark", WindowAppearance::Dark)] {
            let rig = open_with(appearance, false);
            let capture = rig.host.job(|app| app.capture(rig.window)).expect("capture");
            compare(scheme, &capture.png);
        }
    }

    /// Per channel, out of 255: antialiasing drifts by a few levels.
    const CHANNEL_TOLERANCE: i16 = 24;
    /// Share of pixels allowed past the channel tolerance.
    const PIXEL_TOLERANCE: f64 = 0.004;

    fn golden(scheme: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("tests/goldens/forms-{scheme}.png"))
    }

    fn compare(scheme: &str, png: &[u8]) {
        let path = golden(scheme);
        if std::env::var_os("ND_UPDATE_GOLDENS").is_some_and(|v| v == "1") {
            std::fs::create_dir_all(path.parent().expect("dir")).expect("goldens dir");
            std::fs::write(&path, png).expect("write golden");
            println!("recorded {}", path.display());
            return;
        }
        let expected = image::open(&path)
            .unwrap_or_else(|e| panic!("{}: {e} (record with ND_UPDATE_GOLDENS=1)", path.display()))
            .to_rgba8();
        let actual = image::load_from_memory(png).expect("png").to_rgba8();
        let fail = |why: String| {
            let out = path.with_extension("actual.png");
            std::fs::write(&out, png).expect("write actual");
            panic!("{scheme}: {why}; wrote {}", out.display());
        };
        if expected.dimensions() != actual.dimensions() {
            fail(format!("size {:?} != golden {:?}", actual.dimensions(), expected.dimensions()));
        }
        let off = expected
            .pixels()
            .zip(actual.pixels())
            .filter(|(a, b)| a.0.iter().zip(b.0.iter()).any(|(x, y)| (*x as i16 - *y as i16).abs() > CHANNEL_TOLERANCE))
            .count();
        let share = off as f64 / f64::from(expected.width() * expected.height());
        if share > PIXEL_TOLERANCE {
            fail(format!("{:.2}% of pixels differ", share * 100.));
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let tests: Vec<(&str, fn())> = vec![
        ("forms::checkbox_click_and_space_toggle", forms::checkbox_click_and_space_toggle),
        ("forms::switch_click_and_space_toggle", forms::switch_click_and_space_toggle),
        ("forms::typing_arrives_as_text_changed", forms::typing_arrives_as_text_changed),
        ("forms::an_app_write_replaces_the_text", forms::an_app_write_replaces_the_text),
        ("forms::secure_and_search_fields", forms::secure_and_search_fields),
        ("forms::text_area_takes_newlines", forms::text_area_takes_newlines),
        ("forms::combo_box_completes_and_picks", forms::combo_box_completes_and_picks),
        ("forms::nd_focus_gives_the_field_gpui_focus", forms::nd_focus_gives_the_field_gpui_focus),
        ("forms::slider_drag_reports_normalised_values", forms::slider_drag_reports_normalised_values),
        ("forms::select_mappings", forms::select_mappings),
        ("forms::stepper_and_pickers", forms::stepper_and_pickers),
        ("forms::link_reports_activation", forms::link_reports_activation),
        ("forms::automation_set_value_reaches_the_app", forms::automation_set_value_reaches_the_app),
        ("forms::disabled_controls_refuse", forms::disabled_controls_refuse),
        ("forms::animated_progress_keeps_drawing", forms::animated_progress_keeps_drawing),
        ("forms::goldens", forms::goldens),
    ];
    harness(&args, &tests);
}

/// The libtest command line, enough for `cargo test` and nextest (as `headless.rs`).
fn harness(args: &[String], tests: &[(&str, fn())]) {
    let has = |flag: &str| args.iter().any(|a| a == flag);
    if has("--list") {
        if !has("--ignored") {
            for (name, _) in tests {
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
    for (name, test) in tests {
        let selected =
            filters.is_empty() || filters.iter().any(|f| if exact { *f == name } else { name.contains(f.as_str()) });
        if !selected {
            continue;
        }
        match std::panic::catch_unwind(*test) {
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
