//! Tier 2: the media family on the real AppKit backend. Images land as layer contents and
//! report once; the seek bar scrubs and commits once per drag and takes keys; lyrics follow,
//! hand over to a user scroll and resync; a shelf pages through its handle; bubbles and avatars
//! draw; the composer's text area submits on Return and turns pasted images into events. Each
//! widget also gets the echo check (feeding its reported value back emits nothing) and the
//! reentrancy check (props set from inside its own handler neither loop nor emit twice).
//! Prints `ND_MEDIA_MAPPING_OK`.

mod harness;

fn main() {
    #[cfg(target_os = "macos")]
    harness::main("gate_m6::media_mapping", || media::run());
    #[cfg(not(target_os = "macos"))]
    harness::main("gate_m6::media_mapping", || {});
}

#[cfg(target_os = "macos")]
#[path = "forms"]
mod media {
    mod rig;

    use std::io::Cursor;
    use std::time::{Duration, Instant};

    use nd_backend_appkit::objc2::rc::Retained;
    use nd_backend_appkit::objc2::{ClassType, sel};
    use nd_backend_appkit::objc2_app_kit::{
        NSButton, NSControl, NSEvent, NSEventModifierFlags, NSEventType, NSPasteboard, NSPasteboardTypePNG,
        NSScrollView, NSTextField, NSTextView, NSView,
    };
    use nd_backend_appkit::objc2_foundation::{NSArray, NSData, NSPoint, NSString};
    use nd_components::forms::{ImagesPasted, TextSubmitted, text_area};
    use nd_components::media::*;
    use nd_ui::prelude::*;
    use nd_ui::semantics::SemanticAction;

    pub use rig::{Checks, Rig};

    pub fn run() -> ! {
        let _lock = ndshell::lock::acquire("nd-backend-appkit media mapping").expect("machine lock");
        let rig = Rig::new();
        let mut c = Checks(0);
        images(&rig, &mut c);
        seek(&rig, &mut c);
        lyrics(&rig, &mut c);
        shelves(&rig, &mut c);
        chat(&rig, &mut c);
        composer_field(&rig, &mut c);
        if c.0 == 0 {
            println!("ND_MEDIA_MAPPING_OK");
            std::process::exit(0);
        }
        println!("ND_MEDIA_MAPPING_FAILED {} check(s)", c.0);
        std::process::exit(1);
    }

    fn png(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(w, h, image::Rgba([20, 120, 220, 255]));
        let mut out = Vec::new();
        img.write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png).expect("png");
        out
    }

    /// Pumps until `done` holds or `limit` passes.
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

    fn mouse(view: &NSView, kind: NSEventType, x: f64, y: f64) -> Retained<NSEvent> {
        let window = view.window().expect("in a window");
        let at = view.convertPoint_toView(NSPoint::new(x, y), None);
        NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
            kind,
            at,
            NSEventModifierFlags::empty(),
            0.,
            window.windowNumber(),
            None,
            0,
            1,
            1.,
        )
        .expect("mouse event")
    }

    fn fire(control: &NSControl) {
        let mtm = nd_backend_appkit::objc2_foundation::MainThreadMarker::new().expect("main thread");
        let action = control.action().expect("action");
        let sender: &nd_backend_appkit::objc2::runtime::AnyObject = control;
        unsafe {
            nd_backend_appkit::objc2_app_kit::NSApplication::sharedApplication(mtm).sendAction_to_from(
                action,
                control.target().as_deref(),
                Some(sender),
            );
        }
    }

    fn find<T: ClassType + nd_backend_appkit::objc2::DowncastTarget>(view: &NSView) -> Option<Retained<T>> {
        for v in view.subviews().iter() {
            if let Some(t) = v.downcast_ref::<T>() {
                return Some(nd_backend_appkit::objc2::Message::retain(t));
            }
            if let Some(t) = find::<T>(&v) {
                return Some(t);
            }
        }
        None
    }


    struct Pics {
        loads: u32,
        tip: bool,
    }

    impl Render for Pics {
        fn render(&mut self, _w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let tip = if self.tip { "a" } else { "b" };
            v_flex().p(px(12.)).child(
                image(ImageSource::bytes("tier2-blue", png(128, 64)))
                    .id("img")
                    .alt("Blue")
                    .tooltip(tip)
                    .size(px(64.))
                    .rounded_full()
                    .on_load(cx.listener(|this, _: &ImageLoaded, _, cx| {
                        this.loads += 1;
                        // Reentrancy: a prop change from inside the handler.
                        this.tip = !this.tip;
                        cx.notify();
                    })),
            )
        }
    }

    fn images(rig: &Rig, c: &mut Checks) {
        let opened = rig.open("images", |_| Pics { loads: 0, tip: false });
        let win = opened.id();
        let loaded = until(rig, Duration::from_secs(5), || rig.read(&opened, |p| p.loads) >= 1);
        c.check(loaded, "image: decodes off the main thread and reports ImageLoaded");
        let view = rig.view(win, "img");
        let layer = view.layer().and_then(|l| unsafe { l.sublayers() }).and_then(|s| s.firstObject());
        let has = layer.as_ref().is_some_and(|l| unsafe { l.contents() }.is_some());
        c.check(has, "image: the picture is the layer's contents");
        let radius = layer.as_ref().map_or(0., |l| l.cornerRadius());
        c.check((radius - 32.).abs() < 0.5, &format!("image: rounded_full clips to a circle ({radius})"));
        for _ in 0..5 {
            rig.pump();
        }
        c.check(rig.read(&opened, |p| p.loads) == 1, "image: one ImageLoaded, the handler's own update sets nothing more");
        let node = rig.node(win, "img").expect("img node");
        c.check(node.role == nd_ui::Role::Image && node.name == "Blue", "image: role and alt");
        rig.close(opened);
    }

    struct Scrub {
        clock: PlaybackClock,
        log: Vec<Seeked>,
    }

    impl Render for Scrub {
        fn render(&mut self, _w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            v_flex().p(px(12.)).child(
                seek_bar("seek", self.clock).duration(Some(Duration::from_secs(100))).buffered(Some(Duration::from_secs(60))).show_times().w(px(400.)).on_seek(
                    cx.listener(|this, e: &Seeked, _, cx| {
                        this.log.push(*e);
                        // Echo and reentrancy: the app applies a commit at once.
                        if e.phase == SeekPhase::Committed {
                            this.clock = PlaybackClock::paused(e.position);
                        }
                        cx.notify();
                    }),
                ),
            )
        }
    }

    fn seek(rig: &Rig, c: &mut Checks) {
        let opened = rig.open("seek", |_| Scrub { clock: PlaybackClock::paused(Duration::from_secs(10)), log: Vec::new() });
        let win = opened.id();
        let view = rig.view(win, "seek");
        let w = view.bounds().size.width;
        view.mouseDown(&mouse(&view, NSEventType::LeftMouseDown, w * 0.5, 8.));
        view.mouseDragged(&mouse(&view, NSEventType::LeftMouseDragged, w * 0.25, 8.));
        view.mouseUp(&mouse(&view, NSEventType::LeftMouseUp, w * 0.25, 8.));
        rig.pump();
        rig.pump();
        let log = rig.read(&opened, |s| s.log.clone());
        let commits: Vec<&Seeked> = log.iter().filter(|e| e.phase == SeekPhase::Committed).collect();
        c.check(log.first().is_some_and(|e| e.phase == SeekPhase::Scrubbing), "seek: a press scrubs");
        c.check(commits.len() == 1, &format!("seek: one commit per drag ({log:?})"));
        c.check(commits.first().is_some_and(|e| e.position.as_secs() == 25), "seek: the commit lands where it was released");
        let before = rig.read(&opened, |s| s.log.len());
        for _ in 0..4 {
            rig.pump();
        }
        c.check(rig.read(&opened, |s| s.log.len()) == before, "seek: feeding the commit back emits nothing");
        let window = view.window().expect("window");
        window.makeFirstResponder(Some(&view));
        let key = NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
            NSEventType::KeyDown,
            NSPoint::ZERO,
            NSEventModifierFlags::empty(),
            0.,
            window.windowNumber(),
            None,
            &NSString::from_str("\u{f703}"),
            &NSString::from_str("\u{f703}"),
            false,
            124,
        )
        .expect("key event");
        view.keyDown(&key);
        rig.pump();
        let last = rig.read(&opened, |s| s.log.last().copied());
        c.check(last.is_some_and(|e| e.position.as_secs() == 30), &format!("seek: Right steps 5 s ({last:?})"));
        c.check(rig.perform(win, "seek", SemanticAction::SetValue("1:00".into())).is_ok(), "seek: setValue");
        let last = rig.read(&opened, |s| s.log.last().copied());
        c.check(last.is_some_and(|e| e.position.as_secs() == 60 && e.phase == SeekPhase::Committed), "seek: setValue commits");
        rig.close(opened);
    }

    struct Sing {
        lyrics: Lyrics,
        follow: Vec<bool>,
        clicks: u32,
    }

    impl Render for Sing {
        fn render(&mut self, _w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            v_flex().size_full().child(
                lyrics_view("lyrics", self.lyrics.clone(), PlaybackClock::paused(Duration::from_secs(40)))
                    .on_follow_change(cx.listener(|this, e: &LyricsFollowChanged, _, cx| {
                        this.follow.push(e.following);
                        cx.notify();
                    }))
                    .on_line_click(cx.listener(|this, _: &LyricClicked, _, cx| {
                        this.clicks += 1;
                        cx.notify();
                    }))
                    .size_full(),
            )
        }
    }

    fn song() -> Lyrics {
        Lyrics::synced((0..30).map(|i| {
            LyricLine::new(Duration::from_secs(2 * i + 1), format!("Line number {i} of the song"))
                .end(Duration::from_secs(2 * i + 2))
        }))
    }

    fn lyrics(rig: &Rig, c: &mut Checks) {
        let opened = rig.open("lyrics", |_| Sing { lyrics: song(), follow: Vec::new(), clicks: 0 });
        let win = opened.id();
        let view = rig.view(win, "lyrics");
        let scroll = find::<NSScrollView>(&view).expect("lyrics scroll view");
        for _ in 0..4 {
            rig.pump();
        }
        let clip = scroll.contentView();
        let followed = clip.bounds().origin.y;
        c.check(followed > 100., &format!("lyrics: the view follows to the line at 40 s ({followed})"));
        clip.setBoundsOrigin(NSPoint::new(0., followed - 120.));
        rig.pump();
        clip.setBoundsOrigin(NSPoint::new(0., followed - 160.));
        rig.pump();
        c.check(rig.read(&opened, |s| s.follow.clone()) == [false], "lyrics: a user scroll stops following once");
        let resync = find::<NSButton>(&view).expect("resync button");
        c.check(!resync.isHidden(), "lyrics: the resync button shows");
        fire(&resync);
        let back = until(rig, Duration::from_secs(2), || (clip.bounds().origin.y - followed).abs() < 2.);
        c.check(rig.read(&opened, |s| s.follow.clone()) == [false, true] && back, "lyrics: resync follows again");
        c.check(resync.isHidden(), "lyrics: the resync button hides");
        rig.close(opened);
    }

    struct Shelves {
        handle: ShelfHandle,
        scrolled: Vec<ShelfScrolled>,
    }

    impl Render for Shelves {
        fn render(&mut self, _w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            v_flex().w_full().child(
                shelf("shelf", &self.handle)
                    .gap_4()
                    .px_4()
                    .h(px(140.))
                    .children((0..8).map(|i| div().id(format!("card{i}")).w(px(120.)).h(px(120.)).flex_none()))
                    .on_scroll(cx.listener(|this, e: &ShelfScrolled, _, cx| {
                        this.scrolled.push(*e);
                        cx.notify();
                    })),
            )
        }
    }

    fn shelves(rig: &Rig, c: &mut Checks) {
        let opened = rig.open("shelf", |_| Shelves { handle: ShelfHandle::new(), scrolled: Vec::new() });
        rig.pump();
        let handle = rig.read(&opened, |s| s.handle.clone());
        c.check(!handle.can_page_back() && handle.can_page_forward(), "shelf: at the start it pages forward only");
        rig.update(&opened, |s, cx| {
            s.handle.page(1);
            cx.notify();
        });
        let last = std::cell::Cell::new(-1f32);
        let settled = until(rig, Duration::from_secs(3), || {
            let o = handle.offset();
            let still = (o - last.replace(o)).abs() < 0.1;
            o > 100. && still
        });
        let offset = handle.offset();
        c.check(settled, &format!("shelf: a page request scrolls ({offset})"));
        // Cards sit 136 apart after a 16 inset; a page of 480 less a card lands on the third edge.
        c.check((offset - 272.).abs() < 1., &format!("shelf: lands on a card edge ({offset})"));
        let events = rig.read(&opened, |s| s.scrolled.clone());
        c.check(events.last().is_some_and(|e| e.can_page_back), "shelf: ShelfScrolled reports it can page back");
        let count = events.len();
        rig.update(&opened, |_, cx| cx.notify());
        rig.pump();
        c.check(rig.read(&opened, |s| s.scrolled.len()) == count, "shelf: a re-render emits nothing");
        rig.close(opened);
    }

    struct Thread;

    impl Render for Thread {
        fn render(&mut self, _w: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            v_flex()
                .w_full()
                .p(px(12.))
                .child(chat_row("m1", ChatMessage::received("Hello there, how are you doing today?").sender(Person::new("Ada Lovelace"))))
                .child(chat_row("m2", ChatMessage::sent("Fine")))
                .child(avatar_group([Person::new("Ada L"), Person::new("Bo"), Person::new("Cy")], 40.).id("group"))
        }
    }

    fn chat(rig: &Rig, c: &mut Checks) {
        let opened = rig.open("chat", |_| Thread);
        let win = opened.id();
        rig.pump();
        let bubble = rig.view(win, "m1");
        let label = find::<NSTextField>(&bubble).expect("bubble text");
        c.check(label.stringValue().to_string().starts_with("Hello"), "bubble: the text is a label");
        c.check(bubble.frame().size.width <= f64::from(BUBBLE.max_width + BUBBLE.tail_width) + 0.5, "bubble: capped width");
        let short = rig.view(win, "m2");
        c.check(short.frame().size.width < 100., &format!("bubble: hugs short text ({})", short.frame().size.width));
        let sent_x = short.frame().origin.x + short.frame().size.width;
        c.check(sent_x > 400., &format!("bubble: sent sits trailing ({sent_x})"));
        let group = rig.view(win, "group");
        let discs = group.layer().and_then(|l| unsafe { l.sublayers() }).and_then(|s| s.firstObject()).and_then(|g| unsafe { g.sublayers() });
        c.check(discs.is_some_and(|d| d.count() == 3), "avatar group: one disc per person");
        rig.close(opened);
    }

    struct Field {
        text: SharedString,
        submits: u32,
        images: u32,
    }

    impl Render for Field {
        fn render(&mut self, _w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            v_flex().p(px(12.)).child(
                text_area("field")
                    .text(self.text.clone())
                    .min_lines(1)
                    .max_lines(8)
                    .submit_on_return()
                    .accepts_media()
                    .rounded_xl()
                    .w(px(300.))
                    .on_change(cx.listener(|this, e: &nd_components::forms::TextChanged, _, cx| {
                        this.text = e.text.clone();
                        cx.notify();
                    }))
                    .on_submit(cx.listener(|this, _: &TextSubmitted, _, cx| {
                        this.submits += 1;
                        this.text = SharedString::default();
                        cx.notify();
                    }))
                    .on_images(cx.listener(|this, _: &ImagesPasted, _, cx| {
                        this.images += 1;
                        cx.notify();
                    })),
            )
        }
    }

    /// Text in through the input system, as a keyboard delivers it.
    fn type_into(tv: &NSTextView, text: &NSString) {
        let anywhere = nd_backend_appkit::objc2_foundation::NSRange::new(nd_backend_appkit::objc2_foundation::NSNotFound as usize, 0);
        unsafe { nd_backend_appkit::objc2_app_kit::NSTextInputClient::insertText_replacementRange(tv, text, anywhere) };
    }

    fn composer_field(rig: &Rig, c: &mut Checks) {
        let opened = rig.open("composer", |_| Field { text: "".into(), submits: 0, images: 0 });
        let win = opened.id();
        let scroll = rig.view(win, "field").downcast::<NSScrollView>().expect("scroll view");
        let tv = scroll.documentView().and_then(|v| v.downcast::<NSTextView>().ok()).expect("text view");
        let one_line = scroll.frame().size.height;
        let window = tv.window().expect("window");
        window.makeFirstResponder(Some(&tv));
        type_into(&tv, &NSString::from_str("hello"));
        rig.pump();
        c.check(rig.read(&opened, |f| f.text.to_string()) == "hello", "composer field: typing reaches the app");
        unsafe { nd_backend_appkit::objc2_app_kit::NSTextInputClient::doCommandBySelector(&*tv, sel!(insertNewline:)) };
        rig.pump();
        c.check(rig.read(&opened, |f| f.submits) == 1, "composer field: Return submits");
        c.check(!tv.string().to_string().contains('\n'), "composer field: Return inserts no line break");
        c.check(tv.string().to_string().is_empty(), "composer field: the app's cleared text lands");
        type_into(&tv, &NSString::from_str("a\nb\nc\nd"));
        rig.pump();
        rig.pump();
        let grown = scroll.frame().size.height;
        c.check(grown > one_line * 2., &format!("composer field: grows with its lines ({one_line} to {grown})"));
        let pb = NSPasteboard::pasteboardWithUniqueName();
        pb.clearContents();
        let data = NSData::with_bytes(&png(4, 4));
        let types = NSArray::from_slice(&[unsafe { NSPasteboardTypePNG }]);
        unsafe { pb.declareTypes_owner(&types, None) };
        pb.setData_forType(Some(&data), unsafe { NSPasteboardTypePNG });
        let took = tv.readSelectionFromPasteboard(&pb);
        rig.pump();
        c.check(took && rig.read(&opened, |f| f.images) == 1, "composer field: a pasted image becomes ImagesPasted");
        c.check(!tv.string().to_string().contains('\u{fffc}'), "composer field: no attachment glyph in the text");
        rig.close(opened);
    }
}
