//! Tier 2: browser-style window tabs on AppKit, the port of the old `scripts/tabs-drive.ts`.
//! Every tab is a real `NSWindow` in one `NSWindowTabGroup`. Cmd-T goes through the File menu's
//! own key equivalent, the tab bar's `+` through `newWindowForTab:` in the responder chain,
//! the close button through `performClose:` (held until the app drops the page), and Cmd-W
//! through the window's key path. Takes the ndshell machine lock: a key window is needed.
//! Prints `ND_TABS_OK`.

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let name = "gate_m6::appkit_window_tabs_example";
    if args.iter().any(|a| a == "--list") {
        if !args.iter().any(|a| a == "--ignored") && cfg!(target_os = "macos") {
            println!("{name}: test");
        }
        return;
    }
    let filters: Vec<&String> = args.iter().skip(1).filter(|a| !a.starts_with('-')).collect();
    if args.iter().any(|a| a == "--ignored")
        || !(filters.is_empty() || filters.iter().any(|f| name.contains(f.as_str())))
    {
        return;
    }
    #[cfg(target_os = "macos")]
    window::run();
}

#[cfg(target_os = "macos")]
mod window {
    use std::time::{Duration, Instant};

    use nd::appkit::AppHost;
    use nd::appkit::objc2::rc::Retained;
    use nd::appkit::objc2::sel;
    use nd::appkit::objc2_app_kit::{
        NSApplication, NSEvent, NSEventModifierFlags, NSEventType, NSMenu, NSMenuItem, NSWindow,
    };
    use nd::appkit::objc2_foundation::{MainThreadMarker, NSPoint, NSString};
    use nd_ui::semantics::SyntheticInput;
    use nd_ui::{Keystroke, WindowId};

    fn pump(host: &AppHost, for_: Duration) {
        let end = Instant::now() + for_;
        while Instant::now() < end {
            host.platform().pump(Duration::from_millis(10), Duration::from_millis(50));
            host.update(|app| {
                if app.needs_frame() {
                    app.frame();
                }
            });
        }
    }

    fn pump_until(host: &AppHost, what: &str, mut done: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            pump(host, Duration::from_millis(30));
        }
    }

    fn windows(host: &AppHost) -> Vec<WindowId> {
        host.update(|app| app.window_ids().collect())
    }

    fn tabs_of(window: &NSWindow) -> usize {
        window.tabGroup().map_or(1, |g| g.windows().count())
    }

    fn menu_item(mtm: MainThreadMarker, menu: &str, item: &str) -> Retained<NSMenuItem> {
        let bar = NSApplication::sharedApplication(mtm).mainMenu().expect("a main menu");
        let sub: Retained<NSMenu> =
            bar.itemWithTitle(&NSString::from_str(menu)).and_then(|i| i.submenu()).expect("the menu");
        sub.itemWithTitle(&NSString::from_str(item)).unwrap_or_else(|| panic!("{menu} has {item}"))
    }

    fn command_key(window: &NSWindow, key: &str, code: u16) -> Retained<NSEvent> {
        let key = NSString::from_str(key);
        NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
            NSEventType::KeyDown,
            NSPoint::new(0., 0.),
            NSEventModifierFlags::Command,
            0.,
            window.windowNumber(),
            None,
            &key,
            &key,
            false,
            code,
        )
        .expect("key event")
    }

    pub fn run() {
        let _lock = ndshell::lock::acquire("tabs window").expect("machine lock");
        let mtm = MainThreadMarker::new().expect("main thread");
        unsafe {
            std::env::set_var("ND_THEME_TOML", "");
            std::env::set_var("ND_THEME_JSON", "");
        }
        let mut browser = None;
        let host = tabs::app().host(|cx| browser = Some(tabs::init(cx)));
        let browser = browser.expect("browser");
        pump(&host, Duration::from_millis(600));
        let first = windows(&host)[0];
        let ns_first = host.ns_window(first).expect("window");
        assert_eq!(ns_first.tabbingIdentifier().to_string(), "nd.browser");

        // The menus carry the shortcuts, from the same bindings nd dispatch matches.
        let new_tab = menu_item(mtm, "File", "New Tab");
        assert_eq!(new_tab.keyEquivalent().to_string(), "t");
        assert!(new_tab.keyEquivalentModifierMask().contains(NSEventModifierFlags::Command));
        let close_tab = menu_item(mtm, "File", "Close Tab");
        assert_eq!(close_tab.keyEquivalent().to_string(), "w");

        // Cmd-T through the menu's own key equivalent.
        let bar = NSApplication::sharedApplication(mtm).mainMenu().expect("main menu");
        assert!(bar.performKeyEquivalent(&command_key(&ns_first, "t", 17)), "the menu takes Cmd-T");
        pump_until(&host, "the second tab", || windows(&host).len() == 2 && tabs_of(&ns_first) == 2);

        // The tab bar's `+`: `newWindowForTab:` up the key window's responder chain.
        let app = NSApplication::sharedApplication(mtm);
        pump_until(&host, "a key window", || app.keyWindow().is_some());
        let sent = unsafe { app.sendAction_to_from(sel!(newWindowForTab:), None, None) };
        assert!(sent, "a responder implements newWindowForTab:");
        pump_until(&host, "the third tab", || windows(&host).len() == 3 && tabs_of(&ns_first) == 3);
        assert_eq!(browser_tabs(&host, &browser), 3);

        // The close button (performClose:) is held until the app drops the page, then closes.
        let third = windows(&host)[2];
        let ns_third = host.ns_window(third).expect("third window");
        ns_third.performClose(None);
        pump_until(&host, "the third tab to close", || windows(&host).len() == 2 && tabs_of(&ns_first) == 2);
        assert!(!ns_third.isVisible(), "the closed tab's window is gone");
        assert_eq!(host.update(|app| browser.read(app).closed.clone()), vec![3]);

        // Cmd-W through the window's key path closes the key tab.
        let key = host.update(|app| app.window_ids().find(|id| *id != first)).expect("second tab");
        let keystroke = Keystroke::parse("cmd-w").expect("keystroke");
        host.update(|app| app.synthesize(key, &SyntheticInput::Key(keystroke))).expect("posted");
        pump_until(&host, "Cmd-W to close a tab", || windows(&host).len() == 1);
        assert_eq!(browser_tabs(&host, &browser), 1);
        assert_eq!(tabs_of(&ns_first), 1);
        println!("ND_TABS_OK");
    }

    fn browser_tabs(host: &AppHost, browser: &nd::prelude::Entity<tabs::Browser>) -> usize {
        host.update(|app| browser.read(app).tabs.len())
    }
}
