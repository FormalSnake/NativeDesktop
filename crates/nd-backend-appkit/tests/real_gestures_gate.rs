//! M4 gesture gate (ARCHITECTURE.md 6.1 and 16), tier 3: a pinch and a two-finger page swipe
//! reach an `nd-ui` app on [`AppHost`] through AppKit's own recognition. `ndshell` posts them at
//! the HID tap with the window server's gesture and continuous-scroll layouts, under the
//! machine-wide real-input lock. When the window server drops a synthesized gesture (macOS has no
//! public constructor for gesture events), the same CGEvent goes into the app's own queue as an
//! NSEvent instead, and the output says which path delivered it.
//!
//! AppKit's swipe tracker follows a synthesized swipe and springs it back, but never committed
//! one in testing (a 480 pt drag reads as about 12% progress, a flick with momentum events no
//! more), so the commit path is covered by the tier 1 `PageSwipe` tests, not here.
//!
//! "Swipe between pages" is the user's trackpad setting; the test turns it on for this process
//! only (the argument domain), never in the user's preferences. Prints `ND_M4_GESTURES_OK`.

mod harness;

fn main() {
    #[cfg(target_os = "macos")]
    harness::main_real("gate_m4::real_pinch_and_page_swipe", gate::run);
    #[cfg(not(target_os = "macos"))]
    harness::main_real("gate_m4::real_pinch_and_page_swipe", || {});
}

#[cfg(target_os = "macos")]
mod gate {
    use std::time::{Duration, Instant};

    use nd_backend_appkit::objc2::runtime::AnyObject;
    use nd_backend_appkit::objc2_app_kit::{NSEvent, NSScreen};
    use nd_backend_appkit::objc2_foundation::{
        MainThreadMarker, NSArgumentDomain, NSDictionary, NSNumber, NSString, NSUserDefaults,
    };
    use nd_backend_appkit::{AppHost, NdAppKitPlatform};
    use nd_ui::prelude::*;
    use nd_ui::{SwipePageEvent, WindowId};
    use ndshell::input::{self, GesturePhase as Hid};
    use objc2_core_foundation::CFRetained;
    use objc2_core_graphics::CGEvent;

    #[derive(Default)]
    struct Pad {
        pinches: Vec<(GesturePhase, f32)>,
        swipes: Vec<SwipePageEvent>,
    }

    impl Render for Pad {
        fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            v_flex()
                .size_full()
                .bg(rgb(0xeeeeee))
                .on_pinch(cx.listener(|this: &mut Pad, e: &PinchEvent, _, cx| {
                    this.pinches.push((e.phase, e.scale));
                    if e.phase == GesturePhase::Ended {
                        cx.haptic(Haptic::Alignment);
                    }
                }))
                .on_swipe_page(cx.listener(|this: &mut Pad, e: &SwipePageEvent, _, _| this.swipes.push(*e)))
                .child(text("pinch or swipe"))
        }
    }

    struct Checks(u32);

    impl Checks {
        fn check(&mut self, ok: bool, what: &str) {
            println!("{} {what}", if ok { "ok  " } else { "FAIL" });
            if !ok {
                self.0 += 1;
            }
        }
    }

    /// Lets this process see "Swipe between pages" as on, whatever the user chose.
    fn enable_swipe_navigation() {
        let defaults = NSUserDefaults::standardUserDefaults();
        let key = NSString::from_str("AppleEnableSwipeNavigateWithScrolls");
        let on = NSNumber::new_bool(true);
        let value: &AnyObject = &on;
        let domain = NSDictionary::from_slices(&[&*key], &[value]);
        // SAFETY: a string-keyed dictionary of property-list values, as the domain requires.
        unsafe { defaults.setVolatileDomain_forName(&domain, NSArgumentDomain) };
    }

    fn pump(platform: &NdAppKitPlatform, ms: u64) {
        let end = Instant::now() + Duration::from_millis(ms);
        while Instant::now() < end {
            platform.pump(Duration::from_millis(10), Duration::from_millis(50));
        }
    }

    /// Posts `events` into the app's own queue, as AppKit would have read them from the window
    /// server.
    fn post_to_queue(platform: &NdAppKitPlatform, events: Vec<Option<CFRetained<CGEvent>>>) {
        for event in events.into_iter().flatten() {
            if let Some(ns) = NSEvent::eventWithCGEvent(&event) {
                platform.app().postEvent_atStart(&ns, false);
                pump(platform, 16);
            }
        }
    }

    pub fn run() {
        let lock = match ndshell::preflight::real_input_locked("gate_m4::real_pinch_and_page_swipe") {
            Ok(lock) => lock,
            Err(e) => {
                println!("ND_M4_GESTURES_FAILED preflight: {e}");
                std::process::exit(1);
            }
        };
        enable_swipe_navigation();
        let mut window_id = WindowId(0);
        let mut root = None;
        let host = AppHost::new(|cx| {
            let options = WindowOptions::main("gestures").title("Gestures").size(size(px(480.), px(320.)));
            let handle = cx.open_window(options, |_, cx| cx.new(|_| Pad::default()));
            window_id = handle.id();
            root = Some(handle.root().clone());
        });
        let root = root.expect("root view");
        let platform = host.platform();
        let window = host.ns_window(window_id).expect("window");
        input::activate(std::process::id());
        let deadline = Instant::now() + Duration::from_secs(3);
        while !window.isKeyWindow() && Instant::now() < deadline {
            pump(platform, 50);
        }
        host.update(|app| app.frame());

        let mtm = MainThreadMarker::new().expect("main thread");
        let primary = NSScreen::screens(mtm).firstObject().expect("a screen").frame();
        let frame = window.frame();
        // HID points are global with a top-left origin on the primary display.
        let center =
            (frame.origin.x + frame.size.width / 2., primary.size.height - (frame.origin.y + frame.size.height / 2.));

        let mut c = Checks(0);
        c.check(window.isKeyWindow(), "window is key");
        c.check(NSEvent::isSwipeTrackingFromScrollEventsEnabled(), "swipe tracking is on for this process");

        // Pinch: four steps of +5%.
        input::pinch(center, 0.05, 4);
        pump(platform, 300);
        let mut pinch_path = "HID";
        if host.update(|app| root.read(app).pinches.is_empty()) {
            pinch_path = "the app's queue (the window server dropped the synthesized gesture)";
            let mut events = vec![input::magnify_event(center, 0., Hid::Began)];
            events.extend((0..4).map(|_| input::magnify_event(center, 0.05, Hid::Changed)));
            events.push(input::magnify_event(center, 0., Hid::Ended));
            post_to_queue(platform, events);
            pump(platform, 300);
        }
        let pinches = host.update(|app| root.read(app).pinches.clone());
        let zoom: f32 = pinches.iter().map(|(_, s)| *s).product();
        println!("pinch via {pinch_path}: {pinches:?}");
        c.check(pinches.first().map(|p| p.0) == Some(GesturePhase::Began), "the pinch begins");
        c.check(pinches.last().map(|p| p.0) == Some(GesturePhase::Ended), "and ends");
        c.check((zoom - 1.05f32.powi(4)).abs() < 0.02, &format!("scales multiply to the pinch's zoom ({zoom:.3})"));

        // Page swipe: fingers right, back.
        input::two_finger_swipe(center, 40, 12);
        let settle = Instant::now() + Duration::from_secs(4);
        while Instant::now() < settle
            && !host.update(|app| {
                root.read(app)
                    .swipes
                    .last()
                    .is_some_and(|s| s.phase != GesturePhase::Changed && s.phase != GesturePhase::Began)
            })
        {
            pump(platform, 50);
        }
        let mut swipe_path = "HID";
        if host.update(|app| root.read(app).swipes.is_empty()) {
            swipe_path = "the app's queue (the window server dropped the synthesized scroll)";
            let mut events = vec![input::scroll_event(center, 40, 0, Hid::Began)];
            events.extend((0..12).map(|_| input::scroll_event(center, 40, 0, Hid::Changed)));
            events.push(input::scroll_event(center, 0, 0, Hid::Ended));
            post_to_queue(platform, events);
            pump(platform, 800);
        }
        let swipes = host.update(|app| root.read(app).swipes.clone());
        println!(
            "page swipe via {swipe_path}: {} events, max progress {:.2}, last {:?}",
            swipes.len(),
            swipes.iter().map(|s| s.progress).fold(0., f32::max),
            swipes.last()
        );
        c.check(swipes.first().map(|s| s.phase) == Some(GesturePhase::Began), "the page swipe begins");
        c.check(swipes.iter().all(|s| s.direction == SwipeDirection::Back), "fingers moving right swipe back");
        c.check(
            swipes.last().is_some_and(|s| matches!(s.phase, GesturePhase::Ended | GesturePhase::Cancelled)),
            "and settles, committed or not",
        );
        c.check(swipes.iter().any(|s| s.progress > 0.), "progress follows the fingers");

        window.close();
        drop(lock);
        if c.0 == 0 {
            println!("ND_M4_GESTURES_OK");
            std::process::exit(0);
        }
        println!("ND_M4_GESTURES_FAILED {} check(s)", c.0);
        std::process::exit(1);
    }
}
