//! Tier 2: the AppKit terminal view in a real window. A click focuses it (first responder and
//! the session's focus), and keys posted through the app's event queue go nd dispatch, AppKit's
//! first responder, `interpretKeyEvents`, the key encoder and the pty to `cat`, whose echo comes
//! back into the grid. The view draws through a `CAMetalLayer` sized to its backing pixels.
//! Takes the ndshell machine lock: a key window is needed. Prints `ND_TERMINAL_WINDOW_OK`.

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let name = "gate_m9_terminal::appkit_window_keys_reach_cat";
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
    use nd::appkit::objc2::{Message, msg_send};
    use nd::appkit::objc2_app_kit::{NSResponder, NSView};
    use nd::appkit::objc2_foundation::{NSNotFound, NSRange, NSString};
    use nd::terminal::{Command, SessionOptions, TerminalSession};
    use nd_ui::semantics::{MouseButton, PointerPhase, SyntheticInput};
    use nd_ui::{Keystroke, Point};

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

    fn find_terminal(view: &NSView) -> Option<Retained<NSView>> {
        if view.class().name().to_str().is_ok_and(|n| n.contains("NdTerminalView")) {
            return Some(view.retain());
        }
        view.subviews().iter().find_map(|v| find_terminal(&v))
    }

    pub fn run() {
        let _lock = ndshell::lock::acquire("terminal window").expect("machine lock");
        let dir = std::env::temp_dir().join(format!("nd-terminal-window-{}", std::process::id()));
        unsafe {
            std::env::set_var("ND_APP_DATA_DIR", &dir);
            std::env::set_var("ND_THEME_TOML", "");
            std::env::set_var("ND_THEME_JSON", "");
        }
        let session = TerminalSession::spawn(&Command::new("cat"), SessionOptions::default()).expect("spawn cat");
        let mut main = None;
        let s = session.clone();
        let host = terminal::app().host(|cx| {
            terminal::bind_keys(cx);
            main = Some(terminal::open_with(cx, Command::new("cat"), vec![s]));
        });
        let main = main.expect("main window");
        pump(&host, Duration::from_millis(800));
        let window = host.ns_window(main.id()).expect("window");
        let content = window.contentView().expect("content view");
        let frame = unsafe { content.superview() }.unwrap_or(content.clone());
        let view = find_terminal(&frame).expect("the terminal is an NdTerminalView in the window");
        let layer = view.layer().expect("layer-backed");
        assert!(layer.class().name().to_str().is_ok_and(|n| n.contains("CAMetalLayer")), "drawn by Metal");
        let terminal_view =
            view.downcast_ref::<nd::terminal::appkit::NdTerminalView>().expect("the terminal's own view class");
        assert!(terminal_view.frames_presented() > 0, "the view presented a frame");

        // Click the middle of the terminal, in the content view's top-left coordinates.
        let r = view.convertRect_toView(view.bounds(), Some(&content));
        let flipped = content.isFlipped();
        let y = if flipped { r.origin.y } else { content.bounds().size.height - r.origin.y - r.size.height };
        let at = Point { x: (r.origin.x + r.size.width / 2.) as f32, y: (y + r.size.height / 2.) as f32 };
        host.update(|app| {
            for phase in [PointerPhase::Down, PointerPhase::Up] {
                app.synthesize(
                    main.id(),
                    &SyntheticInput::Pointer { phase, at, button: MouseButton::Left, click_count: 1 },
                )
                .expect("posted");
            }
        });
        pump_until(&host, "the click to focus the terminal", || session.is_focused());
        let responder: Option<Retained<NSResponder>> = window.firstResponder();
        assert!(
            responder.is_some_and(|r| std::ptr::eq(&*r as *const NSResponder as *const NSView, &*view)),
            "the terminal view is first responder"
        );

        let before = terminal_view.frames_presented();
        for key in ["h", "i", "enter"] {
            let keystroke = Keystroke::parse(key).expect("keystroke");
            host.update(|app| app.synthesize(main.id(), &SyntheticInput::Key(keystroke))).expect("posted");
            pump(&host, Duration::from_millis(30));
        }
        pump_until(&host, "cat's echo", || session.screen_text() == "hi\nhi");

        // Tab is nd's focus traversal everywhere else; the focused terminal keeps it.
        for key in ["a", "tab", "b", "enter"] {
            let keystroke = Keystroke::parse(key).expect("keystroke");
            host.update(|app| app.synthesize(main.id(), &SyntheticInput::Key(keystroke))).expect("posted");
            pump(&host, Duration::from_millis(30));
        }
        pump_until(&host, "the tab's echo", || session.screen_text() == "hi\nhi\na       b\na       b");

        // Output redraws the view.
        pump_until(&host, "a frame for the echo", || terminal_view.frames_presented() > before);

        // An input method composes, then commits: marked text stays out of the program until
        // `insertText:` arrives.
        let marked = NSString::from_str("にほ");
        let committed = NSString::from_str("日本");
        let none = NSRange::new(NSNotFound as usize, 0);
        let _: () = unsafe {
            msg_send![&*view, setMarkedText: &*marked, selectedRange: NSRange::new(2, 0), replacementRange: none]
        };
        let has_marked: bool = unsafe { msg_send![&*view, hasMarkedText] };
        assert!(has_marked, "composing text is marked");
        pump(&host, Duration::from_millis(100));
        assert_eq!(
            session.screen_text(),
            "hi\nhi\na       b\na       b",
            "nothing reaches the program while composing"
        );
        let _: () = unsafe { msg_send![&*view, insertText: &*committed, replacementRange: none] };
        let has_marked: bool = unsafe { msg_send![&*view, hasMarkedText] };
        assert!(!has_marked, "the commit ends the composition");
        let enter = Keystroke::parse("enter").expect("keystroke");
        host.update(|app| app.synthesize(main.id(), &SyntheticInput::Key(enter))).expect("posted");
        pump_until(&host, "the committed text's echo", || {
            session.screen_text() == "hi\nhi\na       b\na       b\n日本\n日本"
        });

        // A drag selects; the selection is the text under it. A cell joins it once the pointer
        // passes its middle.
        let (cell_w, cell_h) = {
            let size = session.size();
            let scale = window.backingScaleFactor();
            (f64::from(size.cell_width) / scale, f64::from(size.cell_height) / scale)
        };
        let top = if flipped { r.origin.y } else { content.bounds().size.height - r.origin.y - r.size.height };
        let row_y = |row: f64| (top + 6. + cell_h * (row + 0.5)) as f32;
        let col_x = |col: f64| (r.origin.x + 6. + cell_w * (col + 0.5)) as f32;
        host.update(|app| {
            let drag = SyntheticInput::Drag {
                from: Point { x: col_x(0.), y: row_y(0.) },
                to: Point { x: col_x(1.4), y: row_y(1.) },
                steps: 4,
            };
            app.synthesize(main.id(), &drag).expect("posted");
        });
        pump_until(&host, "the drag's selection", || session.selection_text().as_deref() == Some("hi\nhi"));

        // A smaller window is a narrower grid.
        let cols = session.size().cols;
        let mut frame = window.frame();
        frame.size.width -= 200.;
        window.setFrame_display(frame, true);
        pump_until(&host, "the grid to follow the window", || session.size().cols < cols);
        // `ND_TERMINAL_HOLD=<seconds>` keeps the window up after the checks, printing its screen
        // frame, for a look or a region capture.
        if let Some(secs) = std::env::var("ND_TERMINAL_HOLD").ok().and_then(|v| v.parse::<u64>().ok()) {
            let f = window.frame();
            let screen = window.screen().map_or(0., |s| s.frame().size.height);
            println!(
                "ND_TERMINAL_FRAME {} {} {} {}",
                f.origin.x,
                screen - f.origin.y - f.size.height,
                f.size.width,
                f.size.height
            );
            pump(&host, Duration::from_secs(secs));
        }
        let _ = std::fs::remove_dir_all(&dir);
        println!("ND_TERMINAL_WINDOW_OK");
        std::process::exit(0);
    }
}
