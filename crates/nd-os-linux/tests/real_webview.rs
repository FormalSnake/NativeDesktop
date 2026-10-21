//! M8 web view gates on Linux (ARCHITECTURE.md 6.3, 8, 16): `examples/browser` on the GPUI
//! backend with Chromium (nd-cef), each in a private headless Hyprland driven by the
//! compositor's virtual pointer and keyboard. Each runs with the page off-screen (`_cef`) and in
//! Chrome style (`_cef_chrome`, `ND_CEF_STYLE=chrome`), where the app's windows are on the
//! instance's XWayland and the page is Chromium's own X11 window inside the page's.
//!
//! - `real_parity_cef`: `BROWSER_SCENE=parity` runs every event and command of `nd::webview`
//!   against a local test page and must print `ND_WEBVIEW_PARITY_OK engine=cef`.
//! - `real_overlay_leak_cef`: a popover, a modal panel and a context menu over a native button
//!   and a live page that counts every mousedown, mouseup, mousemove and wheel it sees. Nothing
//!   inside an overlay reaches the page; a press that dismisses a light-dismiss overlay is
//!   consumed, release included. Positive controls show the same input reaching the page.
//! - `real_input_cef`: a click focuses a text field in the page, keys typed on the compositor's
//!   keyboard (shift included) land in it and the wheel scrolls the page.
//! - `real_resize_cef`: the resize gate with one flat page filling the window
//!   (`BROWSER_SCENE=resize`, `ND_DEBUG_EXPOSE=1`): in no presented frame of a border drag does
//!   the window background (magenta) or the compositor's show. Until Chromium's frame for a new
//!   size arrives, the page's edge colour fills the strip it has not drawn yet. In Chrome style
//!   only the window background counts: Hyprland resizes an XWayland window without waiting for
//!   its buffer, so the compositor shows along the dragged edge for a frame or two.
//! - `real_chrome_pages_cef_chrome`: `chrome://extensions` and `chrome://settings` load and lay
//!   out in a Chrome-style page.
//!
//! Needs `browser` built next to this test (`cargo build -p browser`) or `ND_BROWSER_BIN`.

#![cfg(target_os = "linux")]

mod common;

mod gate_m8_webview {
    use std::path::PathBuf;
    use std::process::Stdio;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use nd_core::semantic::SemanticNode;
    use nd_test::{Session, connect_app};
    use nd_ui::InputCounts;
    use ndshell::hyprland::{Client, Hyprland, Options};
    use ndshell::wayland::{Capture, Modifier, VirtualInput, keys};

    use crate::common::probe::Probe;
    use crate::common::resize::{analyze, edge_drags, near};

    /// `examples/browser`, built into the same target dir as this test.
    fn browser_bin() -> PathBuf {
        if let Some(bin) = std::env::var_os("ND_BROWSER_BIN") {
            return bin.into();
        }
        let exe = std::env::current_exe().unwrap();
        let bin = exe.parent().and_then(|deps| deps.parent()).unwrap().join("browser");
        assert!(bin.exists(), "build browser first (cargo build -p browser); looked at {bin:?}");
        bin
    }

    /// A headless Hyprland for one run, with XWayland for a Chrome-style page.
    struct Run {
        hyprland: Hyprland,
        display: Option<String>,
        style: &'static str,
    }

    impl Run {
        fn start(style: &'static str) -> Self {
            let chrome = style == "chrome";
            let extra_config = if chrome { "hl.config({ xwayland = { enabled = true } })".into() } else { String::new() };
            let hyprland = Hyprland::start(Options { extra_config, ..Options::default() })
                .unwrap_or_else(|e| panic!("headless Hyprland: {e}"));
            let display = chrome.then(|| hyprland.x11_display(Duration::from_secs(10)).unwrap());
            Self { hyprland, display, style }
        }

        /// The browser, as a client of this instance in this run's style.
        fn command(&self) -> std::process::Command {
            let mut command = self.hyprland.command(browser_bin());
            command.env("ND_CEF_STYLE", self.style);
            if let Some(display) = &self.display {
                command.env("DISPLAY", display);
            }
            command
        }

        fn marker(&self, what: &str) -> String {
            let style = if self.style == "chrome" { "_CHROME" } else { "" };
            format!("ND_M8_{what}_CEF{style}_LINUX_OK")
        }
    }

    #[test]
    fn real_parity_cef() {
        parity("osr");
    }

    #[test]
    fn real_parity_cef_chrome() {
        parity("chrome");
    }

    fn parity(style: &'static str) {
        let run = Run::start(style);
        let mut probe =
            Probe::launch(run.command(), &[("BROWSER_SCENE", "parity"), ("ND_WEBVIEW_ENGINE", "cef")]);
        let result = result_line(&mut probe, "ND_WEBVIEW_PARITY_", Duration::from_secs(240));
        assert!(result.starts_with("ND_WEBVIEW_PARITY_OK"), "{result}\n{}", probe.log());
        assert!(result.contains("engine=cef"), "ran on the wrong engine: {result}");
        assert!(result.contains(&format!("style={style}")), "ran in the wrong style: {result}");
        assert_chrome_style(&run, &probe);
        let steps = probe.lines.lock().unwrap().iter().filter(|l| l.starts_with("ND_PARITY ")).count();
        println!("{steps} parity steps; {}", run.marker("PARITY"));
    }

    /// The scene's result line, or a failure as soon as the browser exits without one.
    fn result_line(probe: &mut Probe, prefix: &str, timeout: Duration) -> String {
        let end = Instant::now() + timeout;
        loop {
            if let Some(line) = probe.last(prefix) {
                return line;
            }
            if let Ok(Some(status)) = probe.child.try_wait() {
                std::thread::sleep(Duration::from_millis(200));
                panic!("the browser exited ({status}) with no {prefix} line:\n{}", probe.log());
            }
            assert!(Instant::now() < end, "no {prefix} line:\n{}", probe.log());
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// A Chrome-style page that fell back to off-screen says so; this run must not have.
    fn assert_chrome_style(run: &Run, probe: &Probe) {
        if run.style == "chrome" {
            assert!(!probe.log().contains("Chrome style needs the window on X11"), "{}", probe.log());
        }
    }

    #[test]
    fn real_chrome_pages_cef_chrome() {
        let run = Run::start("chrome");
        let mut probe =
            Probe::launch(run.command(), &[("BROWSER_SCENE", "chrome-pages"), ("ND_WEBVIEW_ENGINE", "cef")]);
        let result = result_line(&mut probe, "ND_CHROME_PAGES_", Duration::from_secs(120));
        assert!(result.starts_with("ND_CHROME_PAGES_OK"), "{result}\n{}", probe.log());
        assert_chrome_style(&run, &probe);
        println!("{}", run.marker("CHROME_PAGES"));
    }

    fn find<'a>(node: &'a SemanticNode, test_id: &str) -> Option<&'a SemanticNode> {
        if node.test_id.as_deref() == Some(test_id) {
            return Some(node);
        }
        node.children.iter().find_map(|c| find(c, test_id))
    }

    struct App {
        probe: Probe,
        session: Session,
        origin: (f64, f64),
    }

    impl App {
        fn root(&self) -> SemanticNode {
            self.session.tree(None).expect("tree").windows.into_iter().next().expect("a window").root
        }

        /// A point of the node in output coordinates, at fractions of its box.
        fn at(&self, test_id: &str, fx: f32, fy: f32) -> Option<(f64, f64)> {
            let b = find(&self.root(), test_id)?.bounds?;
            let x = self.origin.0 + f64::from(b.origin.x + b.size.width * fx);
            let y = self.origin.1 + f64::from(b.origin.y + b.size.height * fy);
            Some((x, y))
        }

        fn status(&self) -> String {
            find(&self.root(), "status").map(|n| n.name.clone()).unwrap_or_default()
        }

        /// The page's title, which a web view reports as its name.
        fn title(&self) -> String {
            find(&self.root(), "web").map(|n| n.name.clone()).unwrap_or_default()
        }

        /// The counts the page wrote into its title, which a web view reports as its name.
        fn page(&self) -> Option<InputCounts> {
            let name = find(&self.root(), "web")?.name.clone();
            InputCounts::parse(&name).filter(|_| name.starts_with("down="))
        }

        fn counts(&self) -> InputCounts {
            std::thread::sleep(Duration::from_millis(300));
            self.page().expect("page counts")
        }

        fn wait(&self, what: &str, f: impl Fn(&Self) -> bool) {
            let start = Instant::now();
            while !f(self) {
                assert!(
                    start.elapsed() < Duration::from_secs(30),
                    "timed out waiting for {what}: status {:?}, page {:?}\n{}",
                    self.status(),
                    self.title(),
                    self.probe.log()
                );
                std::thread::sleep(Duration::from_millis(30));
            }
        }

        fn status_has(&self, s: &str) {
            self.wait(s, |a| a.status().contains(s));
        }

        fn click_node(&self, test_id: &str) {
            self.session.get_by_test_id(test_id).click().unwrap_or_else(|e| panic!("click {test_id}: {e}"));
        }
    }

    fn wiggle(input: &mut VirtualInput, at: (f64, f64)) {
        for i in 0..6 {
            input.move_to(at.0 + f64::from(i) * 4., at.1 + f64::from(i % 2) * 3.);
            let _ = input.sync();
            std::thread::sleep(Duration::from_millis(30));
        }
    }

    fn click(input: &mut VirtualInput, at: (f64, f64)) {
        input.click(at.0, at.1).unwrap();
        std::thread::sleep(Duration::from_millis(150));
    }

    fn launch(run: &Run, scene: &str, dir: &std::path::Path) -> App {
        let hyprland = &run.hyprland;
        let socket = dir.join("a.sock");
        let mut command = run.command();
        command
            .env("BROWSER_SCENE", scene)
            .env("NATIVE_AUTOMATION", "1")
            .env("ND_AUTOMATION_SOCKET", &socket)
            .env("ND_CEF_CACHE", dir.join("cef"))
            .stdout(Stdio::null());
        let probe = Probe::launch(command, &[]);
        let start = Instant::now();
        let session = loop {
            if let Ok(session) = connect_app(&socket) {
                break session;
            }
            assert!(start.elapsed() < Duration::from_secs(30), "no automation socket:\n{}", probe.log());
            std::thread::sleep(Duration::from_millis(50));
        };
        let client =
            hyprland.wait_for_client(probe.child.id(), Duration::from_secs(10)).expect("browser maps a window");
        App { probe, session, origin: (f64::from(client.at.0), f64::from(client.at.1)) }
    }

    #[test]
    fn real_overlay_leak_cef() {
        overlay_leak("osr");
    }

    #[test]
    fn real_overlay_leak_cef_chrome() {
        overlay_leak("chrome");
    }

    fn overlay_leak(style: &'static str) {
        let run = Run::start(style);
        let dir = std::env::temp_dir().join(format!("nd-webview-overlays-{style}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let app = launch(&run, "overlays", &dir);
        app.wait("the page's counting script", |a| a.page().is_some());
        assert_chrome_style(&run, &app.probe);
        let mut input = VirtualInput::connect(&run.hyprland).unwrap();

        // Controls: with nothing over them the page counts moves, presses and wheel, and the
        // button clicks.
        let free = app.at("web", 0.7, 0.85).unwrap();
        let under = app.at("under", 0.5, 0.5).unwrap();
        let before = app.counts();
        wiggle(&mut input, free);
        click(&mut input, free);
        input.scroll(free.0, free.1, 3).unwrap();
        let after = app.counts();
        assert!(after.moved > before.moved, "control: the page counts moves ({before} -> {after})");
        assert!(after.down > before.down && after.up > before.up, "control: the page counts presses ({after})");
        assert!(after.wheel > before.wheel, "control: the page counts wheel ({after})");
        click(&mut input, under);
        app.status_has("under=1");

        // Popover over the button and the page: everything inside stays inside.
        app.click_node("open-popover");
        app.status_has("popover=open");
        app.wait("the popover laid out", |a| a.at("pop", 0.5, 0.5).is_some());
        std::thread::sleep(Duration::from_millis(300));
        let pop = |fx, fy| app.at("pop", fx, fy).unwrap();
        let (top_left, bottom_right) = (pop(0., 0.), pop(1., 1.));
        let inside =
            |p: (f64, f64)| p.0 > top_left.0 && p.0 < bottom_right.0 && p.1 > top_left.1 && p.1 < bottom_right.1;
        let over_button = (under.0, under.1.max(top_left.1 + 10.));
        let web_left = app.at("web", 0., 0.5).unwrap().0;
        let over_web = (web_left + 20., pop(0.5, 0.6).1);
        assert!(inside(over_button) && inside(over_web), "the popover covers both: {top_left:?} {bottom_right:?}");
        input.move_to(over_web.0, over_web.1);
        let _ = input.sync();
        let base = app.counts();
        for at in [over_web, over_button] {
            wiggle(&mut input, at);
            click(&mut input, at);
            input.right_click(at.0, at.1).unwrap();
            input.scroll(at.0, at.1, 3).unwrap();
        }
        input.move_to(over_web.0, over_web.1);
        let _ = input.sync();
        let inner = app.counts();
        assert!(app.status().contains("popover=open"), "a press inside closed the popover: {}", app.status());
        assert_eq!(inner, base, "input inside the popover reached the page under it");
        assert!(app.status().contains("under=1 "), "the button under the popover was clicked: {}", app.status());

        // Outside, on the page below the popover: closes it, consumed with its release.
        let outside = (web_left + 40., bottom_right.1 + 30.);
        assert!(!inside(outside));
        input.move_to(outside.0, outside.1);
        let _ = input.sync();
        let base = app.counts();
        click(&mut input, outside);
        app.status_has("popover=closed");
        let dismissed = app.counts();
        assert_eq!((dismissed.down, dismissed.up), (base.down, base.up), "the dismissing press reached the page");
        click(&mut input, outside);
        let live = app.counts();
        assert_eq!((live.down, live.up), (base.down + 1, base.up + 1), "with the popover gone the page takes presses");

        // Modal panel: presses outside are blocked, nothing inside leaks, Esc closes it.
        app.click_node("open-panel");
        app.status_has("panel=open");
        app.wait("the panel laid out", |a| a.at("panel", 0.5, 0.5).is_some());
        std::thread::sleep(Duration::from_millis(300));
        let base = app.counts();
        let in_panel = app.at("panel", 0.5, 0.5).unwrap();
        wiggle(&mut input, in_panel);
        click(&mut input, in_panel);
        input.scroll(in_panel.0, in_panel.1, 3).unwrap();
        click(&mut input, under);
        click(&mut input, outside);
        let blocked = app.counts();
        assert_eq!(
            (blocked.down, blocked.up, blocked.wheel),
            (base.down, base.up, base.wheel),
            "the modal panel let input through: {blocked}"
        );
        assert!(app.status().contains("under=1 ") && app.status().contains("panel=open"), "{}", app.status());
        input.key(keys::ESC, &[]).unwrap();
        app.status_has("panel=closed");

        // Context menu over the page: choosing an item works, the page sees nothing, a press
        // outside closes it and is consumed.
        let open_at = app.at("menu-zone", 0.8, 0.2).unwrap();
        input.right_click(open_at.0, open_at.1).unwrap();
        std::thread::sleep(Duration::from_millis(500));
        let item = (open_at.0 + 40., open_at.1 + 14.);
        let web = |p: (f64, f64)| {
            let (a, b) = (app.at("web", 0., 0.).unwrap(), app.at("web", 1., 1.).unwrap());
            p.0 > a.0 && p.0 < b.0 && p.1 > a.1 && p.1 < b.1
        };
        assert!(web((open_at.0 + 80., item.1)), "the menu opens over the page");
        let base = app.counts();
        wiggle(&mut input, (open_at.0 + 60., item.1));
        click(&mut input, item);
        app.status_has("chosen=1");
        input.right_click(open_at.0, open_at.1).unwrap();
        std::thread::sleep(Duration::from_millis(500));
        click(&mut input, under);
        std::thread::sleep(Duration::from_millis(400));
        assert!(
            app.status().contains("under=1 "),
            "the press that closed the menu clicked the button: {}",
            app.status()
        );
        let after_menu = app.counts();
        assert_eq!(
            (after_menu.down, after_menu.up, after_menu.wheel),
            (base.down, base.up, base.wheel),
            "menu: {after_menu}"
        );
        click(&mut input, under);
        app.status_has("under=2");
        drop(app);
        let _ = std::fs::remove_dir_all(&dir);
        println!("{}", run.marker("OVERLAY_LEAK"));
    }

    #[test]
    fn real_input_cef() {
        input("osr");
    }

    #[test]
    fn real_input_cef_chrome() {
        input("chrome");
    }

    fn input(style: &'static str) {
        let run = Run::start(style);
        let dir = std::env::temp_dir().join(format!("nd-webview-input-{style}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let app = launch(&run, "input", &dir);
        app.wait("the page's script", |a| a.title().starts_with("typed="));
        assert_chrome_style(&run, &app.probe);
        let mut input = VirtualInput::connect(&run.hyprland).unwrap();
        let (left, top) = app.at("web", 0., 0.).unwrap();
        // The field sits 24 px in from the page's top left and is about 24 px tall.
        // The pointer enters the window before it presses, as a hand's does: a press in the same
        // batch as the motion that enters the surface can reach the app before its pointer focus.
        let field = (left + 60., top + 36.);
        wiggle(&mut input, (field.0 - 20., field.1));
        click(&mut input, field);
        app.wait("the click to focus the field", |a| a.title().contains(" f=1"));
        assert!(app.title().ends_with(" at=60,36"), "the press lands where it was made: {}", app.title());
        for (key, mods) in [(keys::A, &[][..]), (keys::S, &[]), (keys::SPACE, &[]), (keys::A, &[Modifier::Shift])] {
            input.key(key, mods).unwrap();
            std::thread::sleep(Duration::from_millis(60));
        }
        app.wait("the typed text in the field", |a| a.title().starts_with("typed=as A y="));
        let inside = app.at("web", 0.5, 0.6).unwrap();
        input.scroll(inside.0, inside.1, 5).unwrap();
        let scrolled = |a: &App| {
            a.title().split_whitespace().find_map(|w| w.strip_prefix("y=")?.parse::<u32>().ok()).is_some_and(|y| y > 0)
        };
        app.wait("the page scrolled", scrolled);
        println!("{}", app.title());
        drop(app);
        let _ = std::fs::remove_dir_all(&dir);
        println!("{}", run.marker("INPUT"));
    }

    /// The page's colour, `#2f8f5b`.
    const PAGE: [u8; 3] = [0x2f, 0x8f, 0x5b];

    #[derive(Clone, Copy, Debug, Default)]
    struct Shot {
        magenta: u32,
        background: u32,
    }

    #[test]
    fn real_resize_cef() {
        resize("osr");
    }

    #[test]
    fn real_resize_cef_chrome() {
        resize("chrome");
    }

    fn resize(style: &'static str) {
        let run = Run::start(style);
        let hyprland = &run.hyprland;
        let dir = std::env::temp_dir().join(format!("nd-webview-resize-{style}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let env = [("BROWSER_SCENE", "resize"), ("ND_DEBUG_EXPOSE", "1")];
        let cache = dir.join("cef");
        let mut command = run.command();
        command.env("ND_CEF_CACHE", &cache);
        let probe = Probe::launch(command, &env);
        let client: Client = {
            let end = Instant::now() + Duration::from_secs(30);
            loop {
                if let Ok(client) = hyprland.wait_for_client(probe.child.id(), Duration::from_secs(1)) {
                    break client;
                }
                assert!(Instant::now() < end, "browser maps no window:\n{}", probe.log());
            }
        };
        // The page has painted once the window's inside is its colour.
        let end = Instant::now() + Duration::from_secs(30);
        loop {
            let shot = ndshell::wayland::screenshot(hyprland).unwrap();
            let (x, y) = (client.at.0 as u32 + 40, client.at.1 as u32 + 40);
            if near(shot.rgb(x, y), PAGE) {
                break;
            }
            assert!(Instant::now() < end, "the page never painted:\n{}", probe.log());
            std::thread::sleep(Duration::from_millis(100));
        }
        assert_chrome_style(&run, &probe);
        let mut input = VirtualInput::connect(hyprland).unwrap();
        let shots = Arc::new(Mutex::new(Vec::<Shot>::new()));
        let recording = Arc::new(Mutex::new(false));
        let (sink, on) = (shots.clone(), recording.clone());
        let capture = Capture::start(hyprland, move |captured| {
            if !*on.lock().unwrap() {
                return;
            }
            let Some(frame) = analyze(captured) else { return };
            sink.lock().unwrap().push(Shot { magenta: frame.magenta, background: frame.background });
        })
        .unwrap_or_else(|e| panic!("capture: {e}"));
        input.move_to(5., 5.);
        input.sync().unwrap();
        std::thread::sleep(Duration::from_millis(150));
        *recording.lock().unwrap() = true;
        edge_drags(&mut input, &client, 240.);
        *recording.lock().unwrap() = false;
        let count = capture.stop().unwrap();
        let after = hyprland.wait_for_client(probe.child.id(), Duration::from_secs(1)).unwrap();
        let shots = shots.lock().unwrap().clone();
        let magenta = shots.iter().filter(|s| s.magenta > 0).count();
        let background = shots.iter().filter(|s| s.background > 0).count();
        println!(
            "{count} frames captured; drag: {} frames, {magenta} with magenta, {background} with background; \
             window {:?} -> {:?}",
            shots.len(),
            client.size,
            after.size
        );
        assert!(after.size.0 >= client.size.0 + 200, "the border drags resized the window: {:?}", after.size);
        assert!(shots.len() >= 30, "{} frames", shots.len());
        assert_eq!(magenta, 0, "the page covers the window in every presented frame");
        // Hyprland's XWayland window manager has no `_NET_WM_SYNC_REQUEST`: it draws an X11 window
        // at its new size before the client's buffer for it arrives, so the compositor shows in
        // a strip along the dragged edge whatever the app does.
        if style != "chrome" {
            assert_eq!(background, 0, "the buffer covers the window in every presented frame");
        }
        drop(probe);
        let _ = std::fs::remove_dir_all(&dir);
        println!("{}", run.marker("RESIZE"));
    }
}
