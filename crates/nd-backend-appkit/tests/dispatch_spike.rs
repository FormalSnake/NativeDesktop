//! The M0 dispatch spike on a real window (3.12). Runs on the main thread (`harness = false`)
//! and drives the app through NSEvents posted into its own queue, which reach `NSApp.sendEvent`
//! exactly like hardware input. Prints `ND_DISPATCH_SPIKE_OK` and exits 0 when every check
//! passes. `ND_SPIKE_SHOT=<path>` also captures the window with `screencapture`.

#[cfg(not(target_os = "macos"))]
fn main() {}

#[cfg(target_os = "macos")]
fn main() {
    spike::run();
}

#[cfg(target_os = "macos")]
mod spike {
    use std::process::Command;
    use std::time::Duration;

    use nd_backend_appkit::objc2::rc::Retained;
    use nd_backend_appkit::objc2::{MainThreadOnly, Message};
    use nd_backend_appkit::objc2_app_kit::{
        NSEvent, NSEventModifierFlags, NSEventType, NSResponder, NSSearchToolbarItem, NSTextField, NSTextView, NSView,
        NSWindow,
    };
    use nd_backend_appkit::objc2_foundation::{MainThreadMarker, NSPoint, NSRect, NSSize, NSString};
    use nd_backend_appkit::{AppKitDispatch, NdAppKitPlatform, ToolbarHost};
    use nd_gpui::gpui;
    use nd_ui::dispatch::{DispatchTree, Focusable};
    use nd_ui::{KeyBinding, KeyContext, NodeId};

    gpui::actions!(spike, [Ping]);

    #[derive(Default)]
    struct Spike {
        pings: u32,
    }

    const ROOT: NodeId = NodeId(1);
    const SEARCH: NodeId = NodeId(2);
    const EDITOR: NodeId = NodeId(3);
    const TITLE: NodeId = NodeId(4);

    const KEY_A: u16 = 0x00;
    const KEY_K: u16 = 0x28;
    const KEY_TAB: u16 = 0x30;

    struct Checks {
        failed: u32,
    }

    impl Checks {
        fn check(&mut self, ok: bool, what: &str) {
            println!("{} {what}", if ok { "ok  " } else { "FAIL" });
            if !ok {
                self.failed += 1;
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

    /// The view that owns AppKit's first responder: the field behind a field editor, or the
    /// responder itself.
    fn responder_view(window: &NSWindow) -> Option<Retained<NSView>> {
        let responder: Retained<NSResponder> = window.firstResponder()?;
        if let Some(text) = responder.downcast_ref::<NSTextView>()
            && text.isFieldEditor()
        {
            let delegate = text.delegate()?;
            let object: &nd_backend_appkit::objc2::runtime::AnyObject = delegate.as_ref();
            return object.downcast_ref::<NSView>().map(|v| v.retain());
        }
        responder.downcast::<NSView>().ok()
    }

    fn is_responder(window: &NSWindow, view: &NSView) -> bool {
        responder_view(window).is_some_and(|r| std::ptr::eq(&*r, view))
    }

    fn chain_matches(editor: &NSView, title: &NSView, search: &NSView) -> bool {
        [(editor, title), (title, search), (search, editor)]
            .iter()
            .all(|(view, next)| unsafe { view.nextKeyView() }.is_some_and(|n| std::ptr::eq(&*n, *next)))
    }

    fn text_field(mtm: MainThreadMarker, placeholder: &str, y: f64) -> Retained<NSTextField> {
        let field = NSTextField::textFieldWithString(&NSString::from_str(""), mtm);
        field.setPlaceholderString(Some(&NSString::from_str(placeholder)));
        field.setFrame(NSRect::new(NSPoint::new(20., y), NSSize::new(400., 24.)));
        field
    }

    pub fn run() -> ! {
        let mtm = MainThreadMarker::new().expect("the spike runs on the main thread");
        let platform = NdAppKitPlatform::new(mtm);
        let window = platform.open_window("Dispatch spike", 640., 300.);

        let search = NSSearchToolbarItem::initWithItemIdentifier(
            NSSearchToolbarItem::alloc(mtm),
            &NSString::from_str("nd.search"),
        );
        let _toolbar = ToolbarHost::attach(mtm, &window, "nd.spike", vec![Retained::into_super(search.clone())]);
        let search_field = search.searchField();

        let content = window.contentView().expect("content view");
        let editor = text_field(mtm, "Editor", 160.);
        let title = text_field(mtm, "Title", 110.);
        content.addSubview(&editor);
        content.addSubview(&title);

        let mut tree: DispatchTree<Spike> = DispatchTree::new([KeyBinding::new("secondary-k", Ping, Some("Spike"))]);
        tree.set_root(ROOT);
        tree.set_key_context(ROOT, Some(KeyContext::parse("Spike").unwrap()));
        // Tree order puts the toolbar first, like `scaffold().toolbar(..).content(..)`.
        for node in [SEARCH, EDITOR, TITLE] {
            tree.insert(ROOT, node, None);
            tree.set_focusable(node, Some(Focusable::default()));
        }
        tree.on_action(ROOT, |_: &Ping, cx| cx.cx.pings += 1);

        let dispatch = AppKitDispatch::install(&window, tree, Spike::default());
        dispatch.register_view(SEARCH, &search_field);
        dispatch.register_view(EDITOR, &editor);
        dispatch.register_view(TITLE, &title);
        dispatch.sync_key_view_loop();

        platform.show(&window);
        // Activation and key status arrive as AppKit-defined events after `show` returns.
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while !window.isKeyWindow() && std::time::Instant::now() < deadline {
            platform.pump(Duration::from_millis(50), Duration::from_millis(200));
        }

        let mut c = Checks { failed: 0 };
        c.check(window.isKeyWindow(), "window is key (posted key events reach the key window)");
        let before_resync = chain_matches(&editor, &title, &search_field);
        println!(
            "     chain right after show, before re-sync: {}",
            if before_resync { "intact" } else { "rewritten by AppKit" }
        );
        // The toolbar re-links its search field's nextKeyView into the field's own hosting
        // views when it installs the item, so the loop is synced again once the window is up.
        dispatch.sync_key_view_loop();
        c.check(
            chain_matches(&editor, &title, &search_field),
            "AppKit's nextKeyView chain matches nd's focus order, toolbar included",
        );
        let search_in_toolbar = search_field.window().is_some_and(|w| std::ptr::eq(&*w, &**window))
            && !search_field.isDescendantOf(&content);
        c.check(search_in_toolbar, "search field lives in the window's toolbar, outside the content view");

        dispatch.focus(EDITOR);
        c.check(is_responder(&window, &editor), "nd focus on the editor makes its NSTextField first responder");

        post_key(&platform, &window, "k", KEY_K, NSEventModifierFlags::Command);
        c.check(dispatch.cx().pings == 1, "cmd-k fires the binding while the native NSTextField has focus");
        c.check(editor.stringValue().to_string().is_empty(), "the bound keystroke never reaches the field");

        post_key(&platform, &window, "a", KEY_A, NSEventModifierFlags::empty());
        c.check(editor.stringValue().to_string() == "a", "an unbound key reaches the field as text");
        c.check(dispatch.cx().pings == 1, "typing does not fire the binding");

        post_key(&platform, &window, "\t", KEY_TAB, NSEventModifierFlags::empty());
        c.check(dispatch.tree().focused() == Some(TITLE), "tab moves nd focus to the next field");
        c.check(is_responder(&window, &title), "AppKit first responder follows nd focus");

        post_key(&platform, &window, "\t", KEY_TAB, NSEventModifierFlags::empty());
        c.check(dispatch.tree().focused() == Some(SEARCH), "tab wraps into the toolbar search item");
        c.check(is_responder(&window, &search_field), "the toolbar search field becomes first responder");

        post_key(&platform, &window, "k", KEY_K, NSEventModifierFlags::Command);
        c.check(dispatch.cx().pings == 2, "the binding fires with the toolbar item focused");

        post_key(&platform, &window, "\u{19}", KEY_TAB, NSEventModifierFlags::Shift);
        c.check(dispatch.tree().focused() == Some(TITLE), "shift-tab leaves the toolbar backwards");

        window.makeFirstResponder(Some(&editor));
        c.check(dispatch.tree().focused() == Some(EDITOR), "an AppKit-side focus change (a click) is mirrored into nd");

        c.check(
            chain_matches(&editor, &title, &search_field),
            "the nextKeyView chain still matches after focus moved through the toolbar",
        );

        if let Ok(path) = std::env::var("ND_SPIKE_SHOT") {
            dispatch.focus(SEARCH);
            platform.pump(Duration::from_millis(200), Duration::from_secs(1));
            let status = Command::new("/usr/sbin/screencapture")
                .args(["-x", "-o", &format!("-l{}", window.windowNumber()), &path])
                .status();
            let captured = status.is_ok_and(|s| s.success()) && std::fs::metadata(&path).is_ok_and(|m| m.len() > 0);
            c.check(captured, &format!("window captured to {path}"));
        }

        window.close();
        if c.failed == 0 {
            println!("ND_DISPATCH_SPIKE_OK");
            std::process::exit(0);
        }
        println!("ND_DISPATCH_SPIKE_FAILED {} check(s)", c.failed);
        std::process::exit(1);
    }
}
