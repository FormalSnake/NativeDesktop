//! Tier 2: what the messages port needed after the port gaps. A label clamps to a number of
//! lines; `icon` shows a symbol on its own; the composer's send button takes another icon and
//! fill; a video reports where it is and seeks; an image cuts a bubble tail out of its picture;
//! a panel fades in and out; a reaction landing on a shown bubble fades in; a transcript runs
//! up under the toolbar and rests clear of it; a press on an avatar or a picture reaches
//! the tap and double tap of the element around it. Prints
//! `ND_MSG_GAPS_OK`.

mod harness;

fn main() {
    #[cfg(target_os = "macos")]
    harness::main("msg_gaps::appkit_mapping", || gaps::run());
    #[cfg(not(target_os = "macos"))]
    harness::main("msg_gaps::appkit_mapping", || {});
}

#[cfg(target_os = "macos")]
#[path = "forms"]
mod gaps {
    mod rig;

    use std::time::{Duration, Instant};

    use nd_backend_appkit::objc2::rc::Retained;
    use nd_backend_appkit::objc2::{ClassType, DowncastTarget};
    use nd_backend_appkit::objc2_app_kit::{NSButton, NSImageView, NSView, NSWorkspace};
    use nd_backend_appkit::objc2_foundation::NSString;
    use nd_components::forms::label;
    use nd_components::media::{
        BubbleSide, Draft, ImageLoaded, ImageSource, Person, Reaction, VideoProgress, VideoReady, avatar, chat_bubble,
        composer, image, video,
    };
    use nd_components::{Icon, icon};
    use nd_ui::panel;
    use nd_ui::prelude::*;

    pub use rig::{Checks, Rig};

    const LONG: &str = "A preview long enough to need more than two lines at this width, so the clamp has \
                        something to cut, and then some more words after that for good measure.";

    pub fn run() -> ! {
        let rig = Rig::new();
        let mut c = Checks(0);
        lines_and_icons(&rig, &mut c);
        send_button(&rig, &mut c);
        video_progress(&rig, &mut c);
        tails(&rig, &mut c);
        panel_fade(&rig, &mut c);
        reaction_fade(&rig, &mut c);
        titlebar(&rig, &mut c);
        glass_shapes(&rig, &mut c);
        under_toolbar(&rig, &mut c);
        presses_through_media(&rig, &mut c);
        if c.0 == 0 {
            println!("ND_MSG_GAPS_OK");
            std::process::exit(0);
        }
        println!("ND_MSG_GAPS_FAILED {} check(s)", c.0);
        std::process::exit(1);
    }

    fn reduce_motion() -> bool {
        NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion()
    }

    fn until(rig: &Rig, limit: Duration, done: impl Fn() -> bool) -> bool {
        let start = Instant::now();
        while start.elapsed() < limit {
            rig.pump();
            if done() {
                return true;
            }
        }
        done()
    }

    fn find<T: ClassType + DowncastTarget>(view: &NSView) -> Option<Retained<T>> {
        if let Some(t) = view.downcast_ref::<T>() {
            return Some(nd_backend_appkit::objc2::Message::retain(t));
        }
        view.subviews().iter().find_map(|v| find::<T>(&v))
    }

    struct Lines;

    impl Render for Lines {
        fn render(&mut self, _w: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            v_flex()
                .p(px(12.))
                .gap(px(8.))
                .items_start()
                .child(label("One line").id("one").w(px(160.)))
                .child(label(LONG).lines(2).id("two").w(px(160.)))
                .child(label(LONG).wraps().id("all").w(px(160.)))
                .child(icon(Icon::Moon, 12.).id("moon"))
                .child(nd_components::icon_button("round", Icon::Plus).bezel(nd_ui::Bezel::Circle).size(px(28.)))
        }
    }

    fn lines_and_icons(rig: &Rig, c: &mut Checks) {
        let opened = rig.open("lines", |_| Lines);
        let win = opened.id();
        rig.pump();
        let height = |id: &str| rig.view(win, id).frame().size.height;
        let (one, two, all) = (height("one"), height("two"), height("all"));
        c.check(two > one * 1.5 && two < one * 2.5, &format!("lines: two lines tall ({two} against {one})"));
        c.check(all > two, &format!("lines: the unclamped label is taller ({all} > {two})"));
        let moon = rig.view(win, "moon");
        c.check(moon.frame().size.width == 12. && moon.frame().size.height == 12., "icon: sized by its square");
        let image_view = find::<NSImageView>(&moon);
        c.check(image_view.is_some_and(|v| v.image().is_some()), "icon: shows its symbol");
        let round = find::<NSButton>(&rig.view(win, "round")).expect("round button");
        let glass = nd_backend_appkit::objc2::runtime::AnyClass::get(c"NSGlassEffectView").is_some();
        let want = if glass {
            nd_backend_appkit::objc2_app_kit::NSBezelStyle::Glass
        } else {
            nd_backend_appkit::objc2_app_kit::NSBezelStyle::Circular
        };
        c.check(round.bezelStyle() == want, &format!("circle: the system's round button ({:?})", round.bezelStyle()));
        if glass {
            let shape: isize = unsafe { nd_backend_appkit::objc2::msg_send![&*round, borderShape] };
            c.check(shape == 3, &format!("circle: a glass circle ({shape})"));
        }
        rig.close(opened);
    }

    struct Send {
        editing: bool,
    }

    impl Render for Send {
        fn render(&mut self, _w: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            let green = nd_ui::prelude::hsla(0.36, 0.6, 0.42, 1.);
            let mut draft = Draft::default();
            draft.text = "hi".into();
            v_flex().p(px(12.)).w(px(400.)).child(
                composer("c", &draft).when(self.editing, |c| c.send_icon(Icon::Check, "Save Edit").send_fill(green)),
            )
        }
    }

    fn send_button(rig: &Rig, c: &mut Checks) {
        let opened = rig.open("send", |_| Send { editing: false });
        let win = opened.id();
        rig.pump();
        let button = || find::<NSButton>(&rig.view(win, "c.send")).expect("send button");
        let tip = |b: &NSButton| b.toolTip().map(|t| t.to_string());
        c.check(tip(&button()).as_deref() == Some("Send"), "send: the default tooltip");
        let before = button().bezelColor();
        rig.update(&opened, |s, cx| {
            s.editing = true;
            cx.notify();
        });
        rig.pump();
        c.check(tip(&button()).as_deref() == Some("Save Edit"), "send: the edit tooltip");
        let after = button().bezelColor();
        c.check(after.is_some() && after != before, "send: the fill replaces the accent");
        rig.close(opened);
    }

    #[derive(Default)]
    struct Clip {
        source: String,
        ready: bool,
        seek: Option<(u64, f64)>,
        progress: Vec<VideoProgress>,
    }

    impl Render for Clip {
        fn render(&mut self, _w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let mut v = video(self.source.clone())
                .id("clip")
                .muted(true)
                .w(px(160.))
                .h(px(90.))
                .on_ready(cx.listener(|this, _: &VideoReady, _, cx| {
                    this.ready = true;
                    cx.notify();
                }))
                .on_progress(cx.listener(|this, e: &VideoProgress, _, cx| {
                    this.progress.push(*e);
                    cx.notify();
                }));
            if let Some((request, at)) = self.seek {
                v = v.seek(request, at);
            }
            v_flex().p(px(12.)).child(v)
        }
    }

    fn video_progress(rig: &Rig, c: &mut Checks) {
        let clip = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/clip.mp4");
        let opened = rig.open("clip", |_| Clip { source: clip.into(), ..Clip::default() });
        let ready = until(rig, Duration::from_secs(10), || rig.read(&opened, |s| s.ready));
        c.check(ready, "video: the clip opens");
        let reported = until(rig, Duration::from_secs(5), || rig.read(&opened, |s| !s.progress.is_empty()));
        c.check(reported, "video: reports where it is once ready");
        rig.update(&opened, |s, cx| {
            s.progress.clear();
            s.seek = Some((1, 0.25));
            cx.notify();
        });
        let sought = until(rig, Duration::from_secs(5), || {
            rig.read(&opened, |s| s.progress.iter().any(|p| (p.position - 0.25).abs() < 0.05))
        });
        let got = rig.read(&opened, |s| s.progress.clone());
        c.check(sought, &format!("video: a seek moves it and is reported ({got:?})"));
        rig.close(opened);
    }

    struct Tails {
        loaded: usize,
    }

    impl Render for Tails {
        fn render(&mut self, _w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let svg = "data:image/svg+xml;utf8,<svg xmlns='http://www.w3.org/2000/svg' width='40' height='30'><rect width='40' height='30' fill='blue'/></svg>";
            let loaded = |this: &mut Self, _: &ImageLoaded, _: &mut Window, cx: &mut Context<Self>| {
                this.loaded += 1;
                cx.notify();
            };
            v_flex()
                .p(px(12.))
                .gap(px(8.))
                .child(
                    image(ImageSource::from(svg))
                        .id("tailed")
                        .bubble_tail(BubbleSide::Sent)
                        .w(px(126.))
                        .h(px(90.))
                        .rounded_lg()
                        .on_load(cx.listener(loaded)),
                )
                .child(
                    image(ImageSource::from(svg))
                        .id("plain")
                        .w(px(120.))
                        .h(px(90.))
                        .rounded_lg()
                        .on_load(cx.listener(loaded)),
                )
        }
    }

    fn tails(rig: &Rig, c: &mut Checks) {
        let opened = rig.open("tails", |_| Tails { loaded: 0 });
        let win = opened.id();
        until(rig, Duration::from_secs(5), || rig.read(&opened, |s| s.loaded == 2));
        let mask = |id: &str| {
            let view = rig.view(win, id);
            let layer = view.layer().and_then(|l| unsafe { l.sublayers() }).and_then(|s| s.firstObject());
            layer.is_some_and(|l| l.mask().is_some())
        };
        c.check(mask("tailed"), "tail: the picture is cut to a bubble with its tail");
        c.check(!mask("plain"), "tail: a picture without one keeps its rounded corners");
        rig.close(opened);
    }

    struct Panel {
        open: bool,
    }

    impl Render for Panel {
        fn render(&mut self, _w: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            v_flex().p(px(12.)).child(div().id("anchor").w(px(40.)).h(px(20.))).when(self.open, |col| {
                col.child(panel("p").w(px(200.)).h(px(100.)).child(div().id("inside").w(px(40.)).h(px(20.))))
            })
        }
    }

    fn panel_fade(rig: &Rig, c: &mut Checks) {
        let opened = rig.open("panel", |_| Panel { open: false });
        let win = opened.id();
        rig.pump();
        rig.update(&opened, |s, cx| {
            s.open = true;
            cx.notify();
        });
        let window = || rig.view(win, "inside").window();
        let start = window().map(|w| w.alphaValue());
        if !reduce_motion() {
            c.check(start.is_some_and(|a| a < 1.), &format!("panel: opens transparent and fades in ({start:?})"));
        }
        let shown = until(rig, Duration::from_secs(2), || window().is_some_and(|w| w.alphaValue() >= 1.));
        c.check(shown, "panel: fully shown after the fade");
        let panel = window().expect("the panel window");
        rig.update(&opened, |s, cx| {
            s.open = false;
            cx.notify();
        });
        rig.pump();
        let gone = until(rig, Duration::from_secs(2), || !panel.isVisible());
        c.check(gone, "panel: ordered out once it has faded");
        rig.close(opened);
    }

    struct Bubble {
        reactions: Vec<Reaction>,
    }

    impl Render for Bubble {
        fn render(&mut self, _w: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            v_flex()
                .p(px(24.))
                .child(chat_bubble("Hello", BubbleSide::Received).id("b").reactions(self.reactions.clone()))
        }
    }

    fn reaction_fade(rig: &Rig, c: &mut Checks) {
        let opened = rig.open("bubble", |_| Bubble { reactions: Vec::new() });
        let win = opened.id();
        rig.pump();
        rig.update(&opened, |s, cx| {
            s.reactions = vec![Reaction { glyph: "\u{2764}\u{fe0f}".into(), count: 1, mine: false }];
            cx.notify();
        });
        let view = rig.view(win, "b");
        let layers = view.layer().and_then(|l| unsafe { l.sublayers() }).expect("bubble layers");
        let pill = layers.lastObject().expect("a reaction pill");
        let fading = unsafe { pill.animationForKey(&NSString::from_str("nd-enter")) }.is_some();
        if !reduce_motion() {
            c.check(fading, "reaction: one that lands on a shown bubble fades in");
        }
        rig.close(opened);
    }

    fn titlebar(rig: &Rig, c: &mut Checks) {
        let window = rig.platform.open_window("titlebar", 480., 320.);
        let room = |w: &nd_backend_appkit::NdWindow| {
            use nd_backend_appkit::objc2_app_kit::NSUserInterfaceItemIdentification;
            w.titlebarAccessoryViewControllers()
                .iter()
                .filter(|a| a.view().identifier().is_some_and(|i| i.to_string() == "nd.toolbar.room"))
                .map(|a| a.view().frame().size.height)
                .sum::<f64>()
        };
        // AppKit gives a bottom accessory at least a standard bar's height (36 pt), so a small
        // overflow grows the band by that much.
        nd_backend_appkit::titlebar_room(&window, 46.);
        let first = room(&window);
        c.check(first >= 8., &format!("titlebar: a 46 pt item grows the band ({first})"));
        nd_backend_appkit::titlebar_room(&window, 30.);
        c.check(room(&window) == first, "titlebar: a shorter item leaves it");
        nd_backend_appkit::titlebar_room(&window, 100.);
        c.check(room(&window) == 62., &format!("titlebar: a taller one grows the same accessory ({})", room(&window)));
        window.close();
    }

    struct Glasses;

    impl Render for Glasses {
        fn render(&mut self, _w: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            v_flex()
                .p(px(12.))
                .gap(px(8.))
                .child(nd_components::glass().id("pill").w(px(120.)).h(px(36.)))
                .child(nd_components::glass().id("dot").w(px(36.)).h(px(36.)))
                .child(nd_components::glass().id("panel").w(px(200.)).h(px(200.)))
                .child(div().id("ring").size(px(60.)).rounded_full().border_2().child(div().id("inside").size(px(20.))))
        }
    }

    /// The HIG's shapes for Liquid Glass: a control-sized element is a capsule or a circle, a
    /// large surface square unless the app rounds it.
    fn glass_shapes(rig: &Rig, c: &mut Checks) {
        let glass = nd_backend_appkit::objc2::runtime::AnyClass::get(c"NSGlassEffectView").is_some();
        if !glass {
            return;
        }
        let opened = rig.open("glass", |_| Glasses);
        let win = opened.id();
        rig.pump();
        let radius =
            |id: &str| -> f64 { unsafe { nd_backend_appkit::objc2::msg_send![&*rig.view(win, id), cornerRadius] } };
        c.check(radius("pill") == 18., &format!("glass: a control-sized one is a capsule ({})", radius("pill")));
        c.check(radius("dot") == 18., &format!("glass: a square one is a circle ({})", radius("dot")));
        c.check(radius("panel") == 0., &format!("glass: a large surface stays square ({})", radius("panel")));
        let ring = rig.view(win, "ring").layer().map(|l| l.cornerRadius());
        c.check(ring == Some(30.), &format!("rounded_full(): the radius is half the size, not unbounded ({ring:?})"));
        rig.close(opened);
    }

    struct Transcript {
        rows: nd_virtual::ListState<u64>,
    }

    impl Render for Transcript {
        fn render(&mut self, _w: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            use nd_components::{scaffold, sidebar, toolbar};
            scaffold()
                .toolbar(toolbar().hide_title().navigation(icon(Icon::SquarePen, 16.)))
                .sidebar(sidebar().child(div().size_full()))
                .content_under_toolbar()
                .toolbar_extension(28.)
                .content_edge_on_hover()
                .content(
                    div()
                        .size_full()
                        .child(nd_virtual::list("transcript", &self.rows, |_key: u64, _, _| div().w_full().h(px(44.)))),
                )
        }
    }

    /// Messages' transcript: the list runs up under the toolbar, which blurs what scrolls under
    /// it, and its first row rests below the bar. The title stays in the toolbar, empty, and the
    /// compose button leads the content's part of it.
    fn under_toolbar(rig: &Rig, c: &mut Checks) {
        use nd_backend_appkit::objc2_app_kit::{NSScrollView, NSWindowTitleVisibility};
        let opened = rig.open("transcript", |cx| {
            let mut rows = nd_virtual::ListState::new(cx).estimated_row_height(px(44.));
            rows.set_items(0..60, cx);
            Transcript { rows }
        });
        rig.pump();
        rig.pump();
        let window = &opened.window;
        let content = window.contentView().expect("content view");
        let mut lists = Vec::new();
        fn walk(v: &NSView, out: &mut Vec<Retained<NSScrollView>>) {
            if let Some(s) = v.downcast_ref::<NSScrollView>() {
                out.push(Retained::from(s));
            }
            for sub in v.subviews().iter() {
                walk(&sub, out);
            }
        }
        walk(&content, &mut lists);
        let list = lists.iter().max_by(|a, b| a.frame().size.height.total_cmp(&b.frame().size.height)).expect("a list");
        let bars = list.safeAreaInsets().top;
        let in_window = list.convertRect_toView(list.bounds(), None);
        let top = in_window.origin.y + in_window.size.height;
        let height = content.frame().size.height;
        c.check(bars > 20., &format!("under toolbar: the list sits under the bars ({bars})"));
        c.check(
            (top - height).abs() < 1.,
            &format!("under toolbar: the list reaches the window's top ({top} of {height})"),
        );
        let inset = list.contentInsets().top;
        c.check((inset - bars).abs() < 1., &format!("under toolbar: the list is inset by the bars ({inset})"));
        c.check(
            (bars - 80.).abs() < 1.,
            &format!("under toolbar: toolbar_extension(28) adds to the 52 pt bars ({bars})"),
        );
        let pockets: Vec<f64> = list
            .subviews()
            .iter()
            .filter(|v| v.class().name().to_string_lossy().ends_with("NSScrollPocket"))
            .map(|v| v.alphaValue())
            .collect();
        c.check(
            pockets.iter().all(|a| *a == 0.),
            &format!("under toolbar: at rest the edge effect is off ({pockets:?})"),
        );
        let rest = list.contentView().bounds().origin.y;
        c.check((rest + bars).abs() < 1., &format!("under toolbar: the first row rests below the bar ({rest})"));
        if nd_backend_appkit::objc2::runtime::AnyClass::get(c"NSGlassEffectView").is_some() {
            c.check(
                window.titleVisibility() == NSWindowTitleVisibility::Visible && window.title().to_string().is_empty(),
                "under toolbar: a hidden title stays in the toolbar with no text",
            );
            use nd_backend_appkit::objc2_app_kit::NSAccessibility;
            let name = window.accessibilityTitle().map(|t| t.to_string());
            c.check(
                name.as_deref() == Some("transcript"),
                &format!("under toolbar: the window keeps its name ({name:?})"),
            );
        }
        let toolbar = window.toolbar().expect("toolbar");
        c.check(
            toolbar.items().iter().any(|i| i.isNavigational()),
            "under toolbar: navigation() is a navigational item",
        );
        rig.close(opened);
    }

    struct Presses {
        log: Vec<&'static str>,
    }

    impl Render for Presses {
        fn render(&mut self, _w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            v_flex()
                .p(px(12.))
                .gap(px(8.))
                .child(
                    v_flex()
                        .id("tile")
                        .p(px(6.))
                        .on_tap(record::<TapEvent>("tile", cx))
                        .child(avatar(Person::new("Ana Ruiz"), 60.).id("tile.avatar"))
                        .child(label("Ana")),
                )
                .child(
                    div()
                        .id("picture")
                        .on_tap(record::<TapEvent>("picture tap", cx))
                        .on_double_tap(record::<DoubleTapEvent>("picture double", cx))
                        .child(
                            image(ImageSource::path("/nonexistent/picture.png"))
                                .id("picture.image")
                                .w(px(120.))
                                .h(px(80.)),
                        ),
                )
                .child(div().id("plain").size(px(60.)).bg(hsla(0.6, 0.5, 0.5, 1.)).on_double_tap(record::<
                    DoubleTapEvent,
                >(
                    "plain double", cx
                )))
        }
    }

    fn record<E: 'static>(
        what: &'static str,
        cx: &mut Context<Presses>,
    ) -> impl Fn(&E, &mut Window, &mut App) + 'static {
        cx.listener(move |this: &mut Presses, _: &E, _, cx| {
            this.log.push(what);
            cx.notify();
        })
    }

    /// Posts a click at the middle of `test_id` the way AppKit delivers one: to the view the
    /// window's hit test finds, with the click count and the time of a real press.
    fn click(rig: &Rig, opened: &rig::Opened<Presses>, test_id: &str, count: isize) -> String {
        use nd_backend_appkit::objc2_app_kit::{NSEvent, NSEventModifierFlags, NSEventType};
        use nd_backend_appkit::objc2_foundation::{NSPoint, NSProcessInfo};
        let target = rig.view(opened.id(), test_id);
        let b = target.bounds();
        let in_window = target.convertPoint_toView(NSPoint::new(b.size.width / 2., b.size.height / 2.), None);
        let content = opened.window.contentView().expect("content view");
        // SAFETY: main thread, a view in a live window.
        let at = unsafe { content.superview() }.map_or(in_window, |s| s.convertPoint_fromView(in_window, None));
        let Some(hit) = content.hitTest(at) else { return "nothing".into() };
        let hit_class = hit.class().name().to_string_lossy().into_owned();
        let event = |kind| {
            NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
                kind,
                in_window,
                NSEventModifierFlags::empty(),
                NSProcessInfo::processInfo().systemUptime(),
                opened.window.windowNumber(),
                None,
                0,
                count,
                1.,
            )
            .expect("mouse event")
        };
        hit.mouseDown(&event(NSEventType::LeftMouseDown));
        hit.mouseUp(&event(NSEventType::LeftMouseUp));
        hit_class
    }

    /// A press on a media view (an avatar, a picture) is the element's around it: a
    /// pinned tile's picture selects it, a double click on a photo opens it. A double click on
    /// a plain element fires its double tap too.
    fn presses_through_media(rig: &Rig, c: &mut Checks) {
        let opened = rig.open("presses", |_| Presses { log: Vec::new() });
        rig.pump();
        rig.pump();
        let settle = |rig: &Rig| {
            let start = Instant::now();
            while start.elapsed() < Duration::from_millis(700) {
                rig.pump();
            }
        };
        let log = |rig: &Rig| rig.read(&opened, |p| p.log.clone());

        let hit = click(rig, &opened, "tile.avatar", 1);
        settle(rig);
        c.check(
            log(rig) == ["tile"],
            &format!("press: a click on the avatar taps its tile (hit {hit}, {:?})", log(rig)),
        );
        rig.update(&opened, |p, _| p.log.clear());

        click(rig, &opened, "picture.image", 1);
        click(rig, &opened, "picture.image", 2);
        settle(rig);
        c.check(
            log(rig) == ["picture double"],
            &format!("press: a double click on a picture is its element's double tap, not a tap ({:?})", log(rig)),
        );
        rig.update(&opened, |p, _| p.log.clear());

        click(rig, &opened, "picture.image", 1);
        settle(rig);
        c.check(log(rig) == ["picture tap"], &format!("press: one click on a picture is one tap ({:?})", log(rig)));
        rig.update(&opened, |p, _| p.log.clear());

        click(rig, &opened, "plain", 1);
        click(rig, &opened, "plain", 2);
        settle(rig);
        c.check(log(rig) == ["plain double"], &format!("press: a double click on a plain element ({:?})", log(rig)));
        rig.close(opened);
    }
}
