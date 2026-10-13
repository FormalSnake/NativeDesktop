//! Tier 2: an `nd-ui` app on [`AppHost`] with a real window. Key events are NSEvents posted into
//! the app's own queue, so they reach `NSApp.sendEvent` like hardware input and travel
//! `NdWindow` -> `App::dispatch_keystroke` -> the elements' `key_context`/`on_action`. Focus moves
//! by Tab over `focusable` elements and AppKit's first responder follows. Prints
//! `ND_APP_DISPATCH_OK`.

mod harness;

fn main() {
    #[cfg(target_os = "macos")]
    harness::main("gate_m1::appkit_key_dispatch", || app::run());
    #[cfg(not(target_os = "macos"))]
    harness::main("gate_m1::appkit_key_dispatch", || {});
}

#[cfg(target_os = "macos")]
mod app {
    use std::time::{Duration, Instant};

    use nd_backend_appkit::objc2::rc::Retained;
    use nd_backend_appkit::objc2_app_kit::{NSEvent, NSEventModifierFlags, NSEventType, NSView, NSWindow};
    use nd_backend_appkit::objc2_foundation::{NSPoint, NSString};
    use nd_backend_appkit::{AppHost, NdAppKitPlatform};
    use nd_gpui::gpui;
    use nd_ui::prelude::*;
    use nd_ui::{KeyBinding, NodeId, WindowId};

    gpui::actions!(app_dispatch, [Ping]);

    struct List {
        log: Vec<&'static str>,
    }

    impl Render for List {
        fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let row = |id: &'static str| div().id(id).focusable().w(px(200.)).h(px(32.)).bg(rgb(0xdddddd));
            v_flex()
                .size_full()
                .gap_2()
                .p(px(12.))
                .key_context("List")
                .on_action(cx.listener(|this, _: &Ping, _, _| this.log.push("list")))
                .child(row("first"))
                .child(
                    row("second")
                        .key_context("Row")
                        .on_action(cx.listener(|this, _: &Ping, _, _| this.log.push("row"))),
                )
                .child(text("not focusable"))
                .child(row("third"))
        }
    }

    const KEY_K: u16 = 0x28;
    const KEY_TAB: u16 = 0x30;

    struct Checks(u32);

    impl Checks {
        fn check(&mut self, ok: bool, what: &str) {
            println!("{} {what}", if ok { "ok  " } else { "FAIL" });
            if !ok {
                self.0 += 1;
            }
        }
    }

    fn post_key(platform: &NdAppKitPlatform, window: &NSWindow, chars: &str, code: u16, flags: NSEventModifierFlags) {
        let chars = NSString::from_str(chars);
        let event = NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
            NSEventType::KeyDown,
            NSPoint::new(0., 0.),
            flags,
            0.,
            window.windowNumber(),
            None,
            &chars,
            &chars,
            false,
            code,
        )
        .expect("key event");
        platform.app().postEvent_atStart(&event, false);
        platform.pump(Duration::from_millis(30), Duration::from_secs(2));
    }

    fn responder_is(window: &NSWindow, view: Option<&Retained<NSView>>) -> bool {
        let responder = window.firstResponder();
        match (responder, view) {
            (Some(r), Some(v)) => std::ptr::eq(Retained::as_ptr(&r).cast::<NSView>(), Retained::as_ptr(v)),
            _ => false,
        }
    }

    pub fn run() -> ! {
        let mut window_id = WindowId(0);
        let mut root = None;
        let host = AppHost::new(|cx| {
            cx.bind_keys([KeyBinding::new("cmd-k", Ping, None)]);
            let handle = cx.open_window(WindowOptions::main("dispatch").title("App dispatch"), |_, cx| {
                cx.new(|_| List { log: Vec::new() })
            });
            window_id = handle.id();
            root = Some(handle.root().clone());
        });
        let root = root.expect("root view");
        let platform = host.platform();
        let window = host.ns_window(window_id).expect("the host opened the window");
        let deadline = Instant::now() + Duration::from_secs(3);
        while !window.isKeyWindow() && Instant::now() < deadline {
            platform.pump(Duration::from_millis(50), Duration::from_millis(200));
        }
        // The first frame normally runs in AppKit's layout pass; run it now so nodes exist.
        host.update(|app| app.frame());

        let mut c = Checks(0);
        c.check(window.isKeyWindow(), "window is key (posted key events reach it)");
        let order: Vec<NodeId> = host.update(|app| app.focus_order(window_id));
        c.check(order.len() == 3, &format!("three focusable rows are Tab stops, the text is not ({order:?})"));
        let views: Vec<Option<Retained<NSView>>> = order.iter().map(|n| host.backend().view(*n)).collect();
        let log = |host: &AppHost| host.update(|app| root.read(app).log.clone());
        let focused = |host: &AppHost| host.update(|app| app.focused(window_id));

        post_key(platform, &window, "k", KEY_K, NSEventModifierFlags::Command);
        c.check(log(&host) == ["list"], "cmd-k reaches the element's on_action through App::dispatch_keystroke");

        post_key(platform, &window, "\t", KEY_TAB, NSEventModifierFlags::empty());
        c.check(focused(&host) == order.first().copied(), "tab focuses the first focusable element");
        c.check(responder_is(&window, views[0].as_ref()), "its NSView becomes AppKit's first responder");

        post_key(platform, &window, "\t", KEY_TAB, NSEventModifierFlags::empty());
        c.check(focused(&host) == order.get(1).copied(), "tab moves to the second row");
        c.check(responder_is(&window, views[1].as_ref()), "AppKit's first responder follows");

        post_key(platform, &window, "k", KEY_K, NSEventModifierFlags::Command);
        c.check(log(&host) == ["list", "row"], "with the row focused, its own Row-context listener takes cmd-k");

        post_key(platform, &window, "\t", KEY_TAB, NSEventModifierFlags::empty());
        post_key(platform, &window, "\t", KEY_TAB, NSEventModifierFlags::empty());
        c.check(focused(&host) == order.first().copied(), "tab skips the text and wraps to the first row");

        post_key(platform, &window, "\u{19}", KEY_TAB, NSEventModifierFlags::Shift);
        c.check(focused(&host) == order.get(2).copied(), "shift-tab wraps backwards to the last row");
        c.check(responder_is(&window, views[2].as_ref()), "AppKit's first responder follows shift-tab");

        if let Some(view) = &views[1] {
            window.makeFirstResponder(Some(view));
        }
        c.check(focused(&host) == order.get(1).copied(), "an AppKit-side focus change is mirrored into nd");

        window.close();
        if c.0 == 0 {
            println!("ND_APP_DISPATCH_OK");
            std::process::exit(0);
        }
        println!("ND_APP_DISPATCH_FAILED {} check(s)", c.0);
        std::process::exit(1);
    }
}
