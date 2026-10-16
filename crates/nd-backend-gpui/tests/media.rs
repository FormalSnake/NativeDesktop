//! Tier 2: the drawn media kinds on the GPUI backend inside GPUI's headless app. Pointer and key
//! input go through the window's own input path (`App::synthesize`), wheels through GPUI's
//! dispatch; checks read the events the app heard and what the backend probes. Goldens compare
//! one board in light and dark with `tests/goldens/<os>/media-{light,dark}.png` (`ND_UPDATE_GOLDENS=1`
//! records them; a mismatch writes `<golden>.actual.png`). Image content is PNG bytes made here
//! and every clock is paused, so the board is the same on every run.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use nd_backend_gpui::GpuiHost;
use nd_components::forms::{PastedImage, TextChanged, TextSubmitted, text_area};
use nd_components::{
    Attachment, BubbleSide, ChatMessage, Draft, ImageLoaded, ImageSource, LyricClicked, LyricLine, Lyrics,
    LyricsFollowChanged, NowPlaying, Person, PlaybackClock, Reaction, RunPosition, SeekPhase, Seeked, ShelfArrows,
    ShelfHandle, ShelfScrolled, avatar, avatar_group, chat_bubble, chat_row, composer, image, image_placeholder,
    lyrics_view, player_bar, seek_bar, shelf,
};
use nd_gpui::gpui::{HeadlessAppContext, Modifiers, PlatformInput, ScrollDelta, ScrollWheelEvent, TouchPhase};
use nd_gpui::gpui::{WindowAppearance, point as gpoint, px as gpx};
use nd_theme::{OsTheme, Preset, Scheme, ThemeRegistry};
use nd_ui::prelude::*;
use nd_ui::semantics::{ActionError, MouseButton, PointerPhase, SemanticAction, SyntheticInput};
use nd_ui::{Keystroke, NodeId, Point, SemanticNode, WindowId};

type Log = Rc<RefCell<Vec<String>>>;

/// A PNG of `w` x `h` with a two-colour gradient, so goldens show orientation and fit.
fn png(w: u32, h: u32, a: [u8; 3], b: [u8; 3]) -> Arc<[u8]> {
    let img = image::RgbaImage::from_fn(w, h, |x, y| {
        let t = (x + y) as f32 / (w + h) as f32;
        let mix = |i: usize| (a[i] as f32 + (b[i] as f32 - a[i] as f32) * t) as u8;
        image::Rgba([mix(0), mix(1), mix(2), 255])
    });
    let mut out = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).expect("png");
    out.into()
}

fn picture(key: &str, a: [u8; 3], b: [u8; 3]) -> ImageSource {
    ImageSource::bytes(key.to_owned(), png(96, 64, a, b))
}

fn secs(s: f64) -> Duration {
    Duration::from_secs_f64(s)
}

fn song() -> Lyrics {
    Lyrics::synced([
        LyricLine::new(secs(4.), "Under the paper moon"),
        LyricLine::new(secs(8.), "We kept the lights down low"),
        LyricLine::new(secs(12.), "Every word a little slow").words([
            (secs(12.), "Every"),
            (secs(12.6), "word"),
            (secs(13.2), "a"),
            (secs(13.5), "little"),
            (secs(14.4), "slow"),
        ]),
        LyricLine::new(secs(16.), "And the city sang along"),
        LyricLine::new(secs(20.), "Until the morning came"),
        LyricLine::new(secs(24.), "Under the paper moon"),
    ])
}

struct Board {
    log: Log,
    seek: PlaybackClock,
    area: SharedString,
    shelf: ShelfHandle,
    draft: Draft,
}

impl Board {
    fn new(log: Log) -> Self {
        let mut draft = Draft::default();
        draft.text = "Two photos from today".into();
        for (i, (a, b)) in [([210, 120, 60], [250, 200, 90]), ([40, 110, 190], [120, 200, 230])].into_iter().enumerate()
        {
            let pasted = PastedImage { bytes: png(64, 64, a, b), mime: "image/png".into() };
            draft.attachments.push(Attachment::image(format!("att-{i}"), &pasted));
        }
        Self { log, seek: PlaybackClock::paused(Duration::ZERO), area: "".into(), shelf: ShelfHandle::new(), draft }
    }
}

macro_rules! on {
    ($cx:ident, $ev:ty, |$this:ident, $e:ident| $value:expr $(, $apply:expr)?) => {
        $cx.listener(move |$this: &mut Board, $e: &$ev, _window, cx| {
            let shown = $value;
            $this.log.borrow_mut().push(shown);
            $($apply;)?
            cx.notify();
        })
    };
}

impl Render for Board {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let alice = Person::new("Alice Moreno").image(picture("alice", [200, 90, 120], [240, 180, 140]));
        let bob = Person::new("Bob Stone");
        let now = NowPlaying::new("Paper Moon")
            .subtitle("The Night Shift")
            .artwork(picture("art", [60, 40, 140], [200, 80, 160]))
            .clock(PlaybackClock::paused(secs(72.)))
            .duration(secs(214.))
            .buffered(secs(130.))
            .volume(0.6, false);
        let card = |i: usize| {
            v_flex()
                .gap(px(4.))
                .flex_none()
                .child(image_placeholder().id(format!("card-{i}")).w(px(120.)).h(px(80.)).rounded_md())
                .child(text(format!("Album {i}")))
        };
        let left = v_flex()
            .w(px(420.))
            .gap(px(12.))
            .child(
                h_flex()
                    .gap(px(12.))
                    .items_center()
                    .child(
                        image(picture("photo", [30, 140, 120], [230, 220, 120]))
                            .id("img-test")
                            .size_hint(200, 100)
                            .w(px(160.))
                            .rounded_lg()
                            .on_load(on!(cx, ImageLoaded, |this, e| format!("loaded:{}x{}", e.width, e.height))),
                    )
                    .child(
                        image(picture("face", [90, 60, 200], [250, 160, 80])).id("circle").size(px(64.)).rounded_full(),
                    )
                    .child(image_placeholder().id("placeholder").w(px(96.)).h(px(64.)).rounded_lg()),
            )
            .child(
                seek_bar("seek", self.seek).duration(Some(secs(100.))).buffered(Some(secs(70.))).show_times().on_seek(
                    on!(
                        cx,
                        Seeked,
                        |this, e| format!("seek:{:?}:{:.0}", e.phase, e.position.as_secs_f64()),
                        if e.phase == SeekPhase::Committed {
                            this.seek = PlaybackClock::paused(e.position)
                        }
                    ),
                ),
            )
            .child(player_bar("player", now.clone()))
            .child(player_bar("mini", now).compact().w(px(300.)))
            .child(
                lyrics_view("lyrics", song(), PlaybackClock::paused(secs(13.3)))
                    .duration(Some(secs(30.)))
                    .w_full()
                    .h(px(240.))
                    .on_follow_change(on!(cx, LyricsFollowChanged, |this, e| format!("follow:{}", e.following)))
                    .on_line_click(on!(cx, LyricClicked, |this, e| format!("line:{:?}", e.line))),
            );
        let right = v_flex()
            .w(px(420.))
            .gap(px(8.))
            .child(
                shelf("shelf", &self.shelf)
                    .arrows(ShelfArrows::Always)
                    .gap(px(12.))
                    .children((0..6).map(card))
                    .on_scroll(on!(cx, ShelfScrolled, |this, e| format!(
                        "shelf:back={}:forward={}",
                        e.can_page_back, e.can_page_forward
                    ))),
            )
            .child(chat_row(
                "m1",
                ChatMessage::received("Are you coming tonight?").position(RunPosition::First).sender(alice.clone()),
            ))
            .child(chat_row(
                "m2",
                ChatMessage::received("The show starts at nine").position(RunPosition::Last).sender(alice.clone()),
            ))
            .child(chat_row("m3", ChatMessage::sent("Wouldn't miss it").position(RunPosition::First)))
            .child(chat_row(
                "m4",
                ChatMessage::sent("Saving you a seat")
                    .position(RunPosition::Last)
                    .reactions([
                        Reaction { glyph: "\u{2764}\u{fe0f}".into(), count: 2, mine: false },
                        Reaction { glyph: "\u{1f44d}".into(), count: 1, mine: true },
                    ])
                    .receipt("Read 8:14 PM"),
            ))
            .child(
                h_flex().w_full().justify_end().child(chat_bubble("\u{1f389}\u{1f389}", BubbleSide::Sent).id("emoji")),
            )
            .child(
                h_flex()
                    .gap(px(8.))
                    .items_center()
                    .child(avatar(alice.clone(), 40.).id("avatar"))
                    .child(avatar(bob.clone(), 40.).id("monogram"))
                    .child(avatar(Person::new("+1 555 0100"), 40.).id("number"))
                    .child(avatar_group([alice, bob, Person::new("Cleo Park")], 40.).id("group")),
            )
            .child(composer("composer", &self.draft))
            .child(
                text_area("area")
                    .text(self.area.clone())
                    .placeholder("Message")
                    .min_lines(1)
                    .max_lines(4)
                    .submit_on_return()
                    .w(px(300.))
                    .rounded_xl()
                    .on_change(on!(cx, TextChanged, |this, e| format!("area:{}", e.text), this.area = e.text.clone()))
                    .on_submit(on!(cx, TextSubmitted, |this, e| format!("submit:{}", e.text))),
            )
            .child(
                h_flex()
                    .gap(px(8.))
                    .child(button("off", "Disabled").disabled(true).on_click(cx.listener(
                        |this: &mut Board, _: &ClickEvent, _, _| this.log.borrow_mut().push("off:click".into()),
                    )))
                    .child(button("on", "Enabled").on_click(cx.listener(|this: &mut Board, _: &ClickEvent, _, _| {
                        this.log.borrow_mut().push("on:click".into())
                    }))),
            );
        h_flex().size_full().p(px(16.)).gap(px(24.)).items_start().child(left).child(right)
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
    view: Rc<RefCell<Option<nd_ui::Entity<Board>>>>,
}

const SIZE: (f32, f32) = (900., 690.);

fn open_with(appearance: WindowAppearance) -> Rig {
    let mut cx = headless();
    let log: Log = Rc::default();
    let view = Rc::new(RefCell::new(None));
    let (l, v) = (log.clone(), view.clone());
    let mut window = None;
    let host = cx.update(|cx| {
        GpuiHost::builder().quit_when_empty(false).start(cx, |app| {
            let handle = app.open_window(
                WindowOptions::main("media").title("Media").size(size(px(SIZE.0), px(SIZE.1))),
                |_, cx| {
                    let entity = cx.new(|_| Board::new(l));
                    *v.borrow_mut() = Some(entity.clone());
                    entity
                },
            );
            window = Some(handle.id());
        })
    });
    let mut registry = ThemeRegistry::new(Preset::current());
    let scheme = if matches!(appearance, WindowAppearance::Dark) { Scheme::Dark } else { Scheme::Light };
    registry.set_os(OsTheme { scheme: Some(scheme), ..registry.os().clone() });
    host.backend().set_theme(registry);
    settle(&mut cx, &host);
    Rig { cx, host, window: window.expect("window"), log, view }
}

fn open() -> Rig {
    open_with(WindowAppearance::Light)
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

    /// Settles until `done`, giving the image workers and wall-clock tweens real time.
    fn settle_until(&mut self, what: &str, done: impl Fn(&mut Self) -> bool) {
        for _ in 0..150 {
            self.settle();
            if done(self) {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("timed out waiting for {what}");
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

    fn at(&self, id: &str, fx: f32, fy: f32) -> Point {
        let b = self.node(id).bounds.unwrap_or_else(|| panic!("{id} laid out"));
        Point { x: b.origin.x + b.size.width * fx, y: b.origin.y + b.size.height * fy }
    }

    fn pointer(&mut self, phase: PointerPhase, at: Point) {
        self.input(vec![SyntheticInput::Pointer { phase, at, button: MouseButton::Left, click_count: 1 }]);
    }

    fn click_point(&mut self, at: Point) {
        let inputs = [PointerPhase::Down, PointerPhase::Up]
            .into_iter()
            .map(|phase| SyntheticInput::Pointer { phase, at, button: MouseButton::Left, click_count: 1 })
            .collect();
        self.input(inputs);
    }

    fn keys(&mut self, keys: &[&str]) {
        let inputs = keys.iter().map(|k| SyntheticInput::Key(Keystroke::parse(k).expect("keystroke"))).collect();
        self.input(inputs);
    }

    fn wheel(&mut self, at: Point, dx: f32, dy: f32) {
        let handle = self.host.window_handle(self.window).expect("gpui window");
        let event = ScrollWheelEvent {
            position: gpoint(gpx(at.x), gpx(at.y)),
            delta: ScrollDelta::Pixels(gpoint(gpx(dx), gpx(dy))),
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

    fn perform(&mut self, id: &str, action: SemanticAction) -> Result<(), ActionError> {
        let node = self.id(id);
        let result = self.cx.update(|cx| self.host.update(cx, |app| app.perform(node, &action)));
        self.settle();
        result
    }

    fn take(&self) -> Vec<String> {
        std::mem::take(&mut *self.log.borrow_mut())
    }

    fn read<R>(&mut self, f: impl FnOnce(&Board) -> R) -> R {
        let entity = self.view.borrow().clone().expect("view");
        self.host.read(|app| f(entity.read(app)))
    }
}

/// Settling again produces nothing: what the app fed back is no new report.
fn quiet(rig: &mut Rig) {
    rig.settle();
    assert_eq!(rig.take(), Vec::<String>::new(), "no echo after the app applied the value");
}

/// Drops the reports the board makes on its own while it loads (images, the shelf's arrows).
fn opened() -> Rig {
    let mut rig = open();
    rig.settle_until("the test image", |r| r.log.borrow().iter().any(|l| l.starts_with("loaded:")));
    rig.take();
    rig
}

mod media {
    use super::*;

    pub fn seek_bar_drag_scrubs_then_commits_once() {
        let mut rig = opened();
        let (from, to) = (rig.at("seek", 0.1, 0.25), rig.at("seek", 0.5, 0.25));
        rig.pointer(PointerPhase::Down, from);
        rig.pointer(PointerPhase::Move, to);
        assert_eq!(rig.node("seek").value.as_deref(), Some("0:50 of 1:40"), "the bar shows the drag");
        rig.pointer(PointerPhase::Up, to);
        assert_eq!(rig.take(), ["seek:Scrubbing:10", "seek:Scrubbing:50", "seek:Committed:50"]);
        assert_eq!(rig.read(|b| b.seek.position), secs(50.));
        quiet(&mut rig);
        rig.keys(&["right"]);
        assert_eq!(rig.take(), ["seek:Committed:55"], "an arrow key steps from what is shown");
        rig.keys(&["left", "left"]);
        assert_eq!(rig.take(), ["seek:Committed:50", "seek:Committed:45"]);
        quiet(&mut rig);
    }

    pub fn image_placeholder_has_its_final_size_and_loads_once() {
        let mut rig = open();
        let b = rig.node("img-test").bounds.expect("laid out");
        assert_eq!((b.size.width, b.size.height), (160., 80.), "the size hint shapes the placeholder");
        rig.settle_until("the test image", |r| r.log.borrow().iter().any(|l| l.starts_with("loaded:")));
        let b = rig.node("img-test").bounds.expect("laid out");
        assert_eq!((b.size.width, b.size.height), (160., 80.), "the decoded image keeps the placeholder's box");
        for _ in 0..5 {
            rig.settle();
        }
        let loaded: Vec<String> = rig.take().into_iter().filter(|l| l.starts_with("loaded:")).collect();
        assert_eq!(loaded, ["loaded:96x64"], "one report per load");
        let p = rig.node("placeholder").bounds.expect("laid out");
        assert_eq!((p.size.width, p.size.height), (96., 64.));
    }

    pub fn lyrics_wheel_stops_following_and_resync_resumes() {
        let mut rig = opened();
        let centre = rig.at("lyrics", 0.5, 0.5);
        rig.wheel(centre, 0., -40.);
        rig.wheel(centre, 0., -40.);
        assert_eq!(rig.take(), ["follow:false"], "one report however far the user scrolls");
        let b = rig.node("lyrics").bounds.expect("laid out");
        let button = Point { x: b.origin.x + b.size.width - 12. - 22., y: b.origin.y + b.size.height - 12. - 16. };
        rig.click_point(button);
        assert_eq!(rig.take(), ["follow:true"]);
        quiet(&mut rig);
    }

    pub fn shelf_arrow_pages() {
        let mut rig = opened();
        let handle = rig.read(|b| b.shelf.clone());
        assert_eq!(handle.offset(), 0.);
        assert!(handle.can_page_forward(), "six cards overflow the shelf");
        let b = rig.node("shelf").bounds.expect("laid out");
        let forward = Point { x: b.origin.x + b.size.width - 8. - 16., y: b.origin.y + b.size.height / 2. };
        rig.click_point(forward);
        let glide = handle.clone();
        rig.settle_until("the page glide", move |_| glide.offset() == 264.);
        assert!(handle.can_page_back());
        assert_eq!(handle.offset(), 264., "a page is the viewport less one card, snapped to a card edge");
        assert_eq!(rig.take(), ["shelf:back=true:forward=true"], "reported once, when it could page back");
    }

    pub fn text_area_return_submits_and_shift_return_breaks() {
        let mut rig = opened();
        rig.click_point(rig.at("area", 0.5, 0.5));
        rig.keys(&["h", "i"]);
        assert_eq!(rig.take(), ["area:h", "area:hi"]);
        let before = rig.node("area").bounds.expect("laid out").size.height;
        rig.keys(&["enter"]);
        assert_eq!(rig.take(), ["submit:hi"], "Return sends and inserts nothing");
        rig.keys(&["shift-enter", "x"]);
        assert_eq!(rig.take(), ["area:hi\n", "area:hi\nx"]);
        rig.settle();
        let after = rig.node("area").bounds.expect("laid out").size.height;
        assert!(after > before, "the area grew with its second line ({before} -> {after})");
        quiet(&mut rig);
    }

    pub fn disabled_button_takes_no_click() {
        let mut rig = opened();
        rig.click_point(rig.at("off", 0.5, 0.5));
        assert_eq!(rig.take(), Vec::<String>::new());
        assert!(rig.perform("off", SemanticAction::Click).is_err(), "and automation is refused");
        assert!(!rig.node("off").enabled);
        rig.click_point(rig.at("on", 0.5, 0.5));
        assert_eq!(rig.take(), ["on:click"]);
    }

    /// Props set back from a handler with an equal value report nothing more.
    pub fn echo() {
        let mut rig = opened();
        rig.perform("seek", SemanticAction::SetValue("0:30".into())).expect("set");
        assert_eq!(rig.take(), ["seek:Committed:30"]);
        let entity = rig.view.borrow().clone().expect("view");
        rig.cx.update(|cx| {
            rig.host.update(cx, |app| {
                entity.update(app, |b, cx| {
                    b.seek = PlaybackClock::paused(secs(30.));
                    cx.notify();
                })
            })
        });
        quiet(&mut rig);
        assert_eq!(rig.node("seek").value.as_deref(), Some("0:30 of 1:40"));
    }

    /// Handlers that change their widget's props (the seek bar's clock, the area's text) do
    /// not make it report again or loop.
    pub fn reentrancy() {
        let mut rig = opened();
        let (from, to) = (rig.at("seek", 0.2, 0.25), rig.at("seek", 0.8, 0.25));
        rig.input(vec![SyntheticInput::Drag { from, to, steps: 6 }]);
        let log = rig.take();
        assert_eq!(log.iter().filter(|l| l.starts_with("seek:Committed")).count(), 1, "{log:?}");
        assert_eq!(log.last().map(String::as_str), Some("seek:Committed:80"));
        quiet(&mut rig);
        rig.click_point(rig.at("area", 0.5, 0.5));
        rig.keys(&["a", "b"]);
        assert_eq!(rig.take(), ["area:a", "area:ab"]);
        quiet(&mut rig);
    }

    pub fn goldens() {
        // GPUI renders headless only on macOS (Metal) and Linux (wgpu); Windows captures arrive
        // with the Fluent goldens (M3).
        if nd_gpui::gpui_platform::current_headless_renderer().ok().flatten().is_none() {
            println!("skipped: GPUI has no headless renderer on this OS");
            return;
        }
        for (scheme, appearance) in [("light", WindowAppearance::Light), ("dark", WindowAppearance::Dark)] {
            let mut rig = open_with(appearance);
            rig.settle_until("the board's images", |r| r.log.borrow().iter().any(|l| l.starts_with("loaded:")));
            // The other pictures decode on the same workers; give them frames to land.
            for _ in 0..20 {
                std::thread::sleep(Duration::from_millis(10));
                rig.settle();
            }
            let capture = rig.host.job(|app| app.capture(rig.window)).expect("capture");
            compare(scheme, &capture.png);
        }
    }

    /// Per channel, out of 255: antialiasing drifts by a few levels.
    const CHANNEL_TOLERANCE: i16 = 24;
    /// Share of pixels allowed past the channel tolerance.
    const PIXEL_TOLERANCE: f64 = 0.004;

    fn golden(scheme: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(format!("tests/goldens/{}/media-{scheme}.png", std::env::consts::OS))
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
        ("media::seek_bar_drag_scrubs_then_commits_once", media::seek_bar_drag_scrubs_then_commits_once),
        (
            "media::image_placeholder_has_its_final_size_and_loads_once",
            media::image_placeholder_has_its_final_size_and_loads_once,
        ),
        (
            "media::lyrics_wheel_stops_following_and_resync_resumes",
            media::lyrics_wheel_stops_following_and_resync_resumes,
        ),
        ("media::shelf_arrow_pages", media::shelf_arrow_pages),
        (
            "media::text_area_return_submits_and_shift_return_breaks",
            media::text_area_return_submits_and_shift_return_breaks,
        ),
        ("media::disabled_button_takes_no_click", media::disabled_button_takes_no_click),
        ("media::echo", media::echo),
        ("media::reentrancy", media::reentrancy),
        ("media::goldens", media::goldens),
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
