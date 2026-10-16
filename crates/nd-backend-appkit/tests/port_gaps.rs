//! Tier 2: what the messages port needed from the AppKit backend. Rich text reaches a label as
//! attributed runs; a bubble's fill role picks its fill; a canvas draws its display list as
//! layers and redraws when it changes; an SVG data URL decodes into an image; hover reports
//! enter and exit; `focus_id` hands the keyboard to a field. Prints `ND_PORT_GAPS_OK`.

mod harness;

fn main() {
    #[cfg(target_os = "macos")]
    harness::main("port_gaps::appkit_mapping", || gaps::run());
    #[cfg(not(target_os = "macos"))]
    harness::main("port_gaps::appkit_mapping", || {});
}

#[cfg(target_os = "macos")]
#[path = "forms"]
mod gaps {
    mod rig;

    use std::cell::RefCell;
    use std::rc::Rc;
    use std::time::{Duration, Instant};

    use nd_backend_appkit::objc2::rc::Retained;
    use nd_backend_appkit::objc2::{ClassType, msg_send};
    use nd_backend_appkit::objc2_app_kit::{NSEvent, NSFontAttributeName, NSTextField, NSTextView, NSView};
    use nd_backend_appkit::objc2_foundation::{NSRange, NSString};
    use nd_components::forms::{label, text_field};
    use nd_components::media::{BubbleSide, ImageLoaded, ImageSource, chat_bubble, image};
    use nd_theme::FillRole;
    use objc2_core_graphics::CGColor;
    use objc2_quartz_core::CAShapeLayer;
    use nd_ui::prelude::*;
    use nd_ui::{RichText, canvas, span};

    pub use rig::{Checks, Rig};

    pub fn run() -> ! {
        let rig = Rig::new();
        let mut c = Checks(0);
        rich_and_roles(&rig, &mut c);
        canvases(&rig, &mut c);
        svg(&rig, &mut c);
        hover_and_focus(&rig, &mut c);
        if c.0 == 0 {
            println!("ND_PORT_GAPS_OK");
            std::process::exit(0);
        }
        println!("ND_PORT_GAPS_FAILED {} check(s)", c.0);
        std::process::exit(1);
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

    fn find<T: ClassType + nd_backend_appkit::objc2::DowncastTarget>(view: &NSView) -> Option<Retained<T>> {
        if let Some(t) = view.downcast_ref::<T>() {
            return Some(nd_backend_appkit::objc2::Message::retain(t));
        }
        for v in view.subviews().iter() {
            if let Some(t) = find::<T>(&v) {
                return Some(t);
            }
        }
        None
    }

    fn sublayer_count(view: &NSView) -> usize {
        view.layer().and_then(|l| unsafe { l.sublayers() }).map_or(0, |s| s.count())
    }

    struct Rich;

    impl Render for Rich {
        fn render(&mut self, _w: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            v_flex()
                .p(px(12.))
                .gap(px(8.))
                .child(
                    label("plain")
                        .id("rich")
                        .rich(RichText::new([span("Hello "), span("bold").bold(), span(" site").link("https://example.com")])),
                )
                .child(chat_bubble("Sent", BubbleSide::Sent).id("sent"))
                .child(chat_bubble("SMS", BubbleSide::Received).role(FillRole::Success).id("sms"))
                .child(chat_bubble("Received", BubbleSide::Received).role(FillRole::Neutral).id("received"))
        }
    }

    /// The bubble's fill as components rounded to thousandths.
    fn bubble_fill(view: &NSView) -> Option<Vec<i64>> {
        let layer = view.layer()?;
        let shape = unsafe { layer.sublayers() }?.firstObject()?;
        let color = shape.downcast_ref::<CAShapeLayer>()?.fillColor()?;
        let n = CGColor::number_of_components(Some(&color));
        let p = CGColor::components(Some(&color));
        // SAFETY: CoreGraphics returns `n` components for the colour, alive while it is.
        let parts = unsafe { std::slice::from_raw_parts(p, n) };
        Some(parts.iter().map(|v| (v * 1000.).round() as i64).collect())
    }

    fn rich_and_roles(rig: &Rig, c: &mut Checks) {
        let opened = rig.open("rich", |_| Rich);
        let win = opened.id();
        rig.pump();
        let field = find::<NSTextField>(&rig.view(win, "rich")).expect("label field");
        let text = field.attributedStringValue();
        c.check(text.string().to_string() == "Hello bold site", &format!("rich: the runs make the text ({})", text.string()));
        let at = |i: usize, key: &NSString| unsafe {
            let mut range = NSRange::new(0, 0);
            text.attribute_atIndex_effectiveRange(key, i, &mut range)
        };
        let font_name = |i: usize| {
            at(i, unsafe { NSFontAttributeName })
                .and_then(|f| f.downcast::<nd_backend_appkit::objc2_app_kit::NSFont>().ok())
                .map(|f| f.fontDescriptor().symbolicTraits())
        };
        let plain = font_name(0);
        let bold = font_name(7);
        c.check(
            bold.is_some_and(|t| t.contains(nd_backend_appkit::objc2_app_kit::NSFontDescriptorSymbolicTraits::TraitBold)),
            "rich: the bold run is bold",
        );
        c.check(plain != bold, "rich: the plain run is not");
        c.check(at(12, &NSString::from_str("NDLink")).is_some(), "rich: the link run carries its target");
        c.check(at(0, &NSString::from_str("NDLink")).is_none(), "rich: plain text carries none");

        let sent = bubble_fill(&rig.view(win, "sent"));
        let sms = bubble_fill(&rig.view(win, "sms"));
        let received = bubble_fill(&rig.view(win, "received"));
        c.check(sent.is_some() && sms.is_some() && received.is_some(), "roles: each bubble has a fill");
        c.check(sms != sent && sms != received, &format!("roles: the success fill is its own ({sms:?})"));
        rig.close(opened);
    }

    struct Drawing {
        dots: usize,
    }

    impl Render for Drawing {
        fn render(&mut self, _w: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            let dots = self.dots;
            v_flex().p(px(12.)).child(
                canvas(move |bounds, p, _| {
                    p.fill_rounded_rect(bounds, px(6.), nd_ui::prelude::hsla(0.6, 0.5, 0.5, 1.));
                    for i in 0..dots {
                        p.fill_circle(point(px(10. + 20. * i as f32), px(10.)), px(4.), nd_ui::prelude::hsla(0., 1., 0.5, 1.));
                    }
                    p.stroke_arc(point(px(50.), px(50.)), px(20.), 0., 0.75, px(3.), nd_ui::prelude::hsla(0.3, 1., 0.4, 1.));
                })
                .id("cv")
                .w(px(120.))
                .h(px(80.)),
            )
        }
    }

    fn canvases(rig: &Rig, c: &mut Checks) {
        let opened = rig.open("canvas", |_| Drawing { dots: 2 });
        let win = opened.id();
        rig.pump();
        let view = rig.view(win, "cv");
        c.check(view.frame().size.width == 120. && view.frame().size.height == 80., "canvas: framed by layout");
        c.check(sublayer_count(&view) == 4, &format!("canvas: one layer per op ({})", sublayer_count(&view)));
        rig.update(&opened, |d, cx| {
            d.dots = 3;
            cx.notify();
        });
        rig.pump();
        c.check(sublayer_count(&view) == 5, &format!("canvas: redrawn when the list changes ({})", sublayer_count(&view)));
        rig.close(opened);
    }

    struct Svg {
        loaded: Option<(u32, u32)>,
    }

    impl Render for Svg {
        fn render(&mut self, _w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let svg = "data:image/svg+xml;utf8,<svg xmlns='http://www.w3.org/2000/svg' width='10' height='20'><rect width='10' height='20' fill='red'/></svg>";
            v_flex().p(px(12.)).child(
                image(ImageSource::from(svg)).id("svg").size(px(40.)).on_load(cx.listener(
                    |this, e: &ImageLoaded, _, cx| {
                        this.loaded = Some((e.width, e.height));
                        cx.notify();
                    },
                )),
            )
        }
    }

    fn svg(rig: &Rig, c: &mut Checks) {
        let opened = rig.open("svg", |_| Svg { loaded: None });
        let ok = until(rig, Duration::from_secs(5), || rig.read(&opened, |s| s.loaded.is_some()));
        c.check(ok, "svg: a data URL loads");
        let size = rig.read(&opened, |s| s.loaded);
        c.check(size == Some((10, 20)), &format!("svg: reports its own size ({size:?})"));
        rig.close(opened);
    }

    struct Hover {
        log: Rc<RefCell<Vec<bool>>>,
    }

    impl Render for Hover {
        fn render(&mut self, _w: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            let log = self.log.clone();
            v_flex()
                .p(px(12.))
                .gap(px(8.))
                .child(div().id("row").w(px(100.)).h(px(30.)).on_hover(move |h, _, _| log.borrow_mut().push(*h)))
                .child(text_field("first").w(px(200.)))
                .child(div().id("group").child(text_field("second").w(px(200.))))
        }
    }

    fn hover_and_focus(rig: &Rig, c: &mut Checks) {
        let log = Rc::new(RefCell::new(Vec::new()));
        let l = log.clone();
        let opened = rig.open("hover", move |_| Hover { log: l });
        let win = opened.id();
        rig.pump();
        let row = rig.view(win, "row");
        let event = NSEvent::new();
        let _: () = unsafe { msg_send![&*row, mouseEntered: &*event] };
        let _: () = unsafe { msg_send![&*row, mouseExited: &*event] };
        rig.pump();
        c.check(*log.borrow() == [true, false], &format!("hover: enter then exit ({:?})", log.borrow()));

        let window = row.window().expect("in a window");
        let second = rig.view(win, "second");
        c.check(rig.focus_id(win, "group"), "focus: an id resolves to its first focusable descendant");
        rig.pump();
        let target = Retained::as_ptr(&second).cast::<()>();
        // A focused field hands the keyboard to the window's field editor, whose delegate it is.
        let in_second = window.firstResponder().is_some_and(|r| {
            Retained::as_ptr(&r).cast::<()>() == target
                || r.downcast_ref::<NSTextView>()
                    .and_then(|tv| tv.delegate())
                    .is_some_and(|d| Retained::as_ptr(&d).cast::<()>() == target)
        });
        c.check(in_second, "focus: the second field has the keyboard");
        rig.close(opened);
    }
}
