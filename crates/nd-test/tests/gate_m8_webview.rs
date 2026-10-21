//! M8/M9 web view gates on macOS (ARCHITECTURE.md 6.3, 8, 16), each on WKWebView and on CEF in
//! both styles (off-screen, and Chrome style: `ND_CEF_STYLE=chrome`):
//!
//! - `real_parity_*`: `examples/browser` with `BROWSER_SCENE=parity` runs every event and command of
//!   `nd::webview` against a local test page and must print `ND_WEBVIEW_PARITY_OK`.
//! - `real_overlay_leak_*`: the M4 overlay leak scenario with the page in a `webview()`: a
//!   popover, a modal panel and a context menu over a native button and a live page that counts
//!   every mousedown, mouseup, mousemove and wheel it sees, driven with real HID input. Nothing
//!   inside an overlay reaches the page; a press that dismisses a light-dismiss overlay is
//!   consumed, release included.
//! - `real_chrome_pages_cef_chrome`: `chrome://extensions` and `chrome://settings` load and lay
//!   out in a Chrome-style page.
//! - `real_adblock_cef*`: uBlock Origin's default lists on the request filter block and redirect
//!   the 0.5.11 adblock fixture's requests as that build did (CEF only; WebKit has no filter).
//! - `real_private_*`: two tabs of a private window share a cookie, a normal window does not
//!   see it, and a private window opened after the first closed starts empty.
//!
//! Needs `browser` built with the `cef` feature for the CEF runs (`cargo build -p browser
//! --features cef`); a build without it fails the CEF tests with the engine's own message.
//! The `real_` tests need Accessibility for the runner and run only under nextest or with
//! `ND_REAL=1`.

#[cfg(target_os = "macos")]
mod gate_m8_webview {
    use std::time::{Duration, Instant};

    use nd_core::semantic::{SemanticNode, SemanticWindow};
    use nd_test::{AppHandle, LaunchOptions, launch_app};
    use nd_ui::InputCounts;
    use ndshell::input;

    /// `engine` is `system`, `cef` or `cef-chrome` (CEF in Chrome style).
    fn browser(engine: &str, scene: &str) -> LaunchOptions {
        let (engine, style) = engine.split_once('-').unwrap_or((engine, "osr"));
        LaunchOptions::example("browser")
            .env("ND_WEBVIEW_ENGINE", engine)
            .env("ND_CEF_STYLE", style)
            .env("BROWSER_SCENE", scene)
    }

    fn parity(engine: &str) {
        let mut options = browser(engine, "parity");
        options.ready_timeout = Duration::from_secs(60);
        let app = launch_app(options).expect("launch browser");
        let result = app.wait_for_stderr("ND_WEBVIEW_PARITY_", Duration::from_secs(240));
        let tail = app.stderr_tail(40).join("\n");
        match result {
            Ok(line) if line.contains("ND_WEBVIEW_PARITY_OK") => {
                let (name, style) = engine.split_once('-').unwrap_or((engine, "osr"));
                assert!(line.contains(&format!("engine={name}")), "ran on the wrong engine: {line}");
                if name == "cef" {
                    assert!(line.contains(&format!("style={style}")), "ran in the wrong style: {line}");
                }
                println!("ND_M8_PARITY_{}_OK", engine.to_uppercase().replace('-', "_"));
            }
            Ok(line) => panic!("{line}\n{tail}"),
            Err(e) => panic!("no parity result: {e}\n{tail}"),
        }
    }

    #[test]
    fn real_parity_wkwebview() {
        parity("system");
    }

    #[test]
    fn real_parity_cef() {
        parity("cef");
    }

    #[test]
    fn real_parity_cef_chrome() {
        parity("cef-chrome");
    }

    #[test]
    fn real_chrome_pages_cef_chrome() {
        let mut options = browser("cef-chrome", "chrome-pages");
        options.ready_timeout = Duration::from_secs(60);
        let app = launch_app(options).expect("launch browser");
        let result = app.wait_for_stderr("ND_CHROME_PAGES_", Duration::from_secs(120));
        let tail = app.stderr_tail(40).join("\n");
        match result {
            Ok(line) if line.contains("ND_CHROME_PAGES_OK") => println!("ND_M8_CHROME_PAGES_OK"),
            Ok(line) => panic!("{line}\n{tail}"),
            Err(e) => panic!("no chrome pages result: {e}\n{tail}"),
        }
    }

    /// A scene that prints `<prefix>OK` or `<prefix>FAILED`.
    fn scene(engine: &str, scene: &str, prefix: &str) -> String {
        let mut options = browser(engine, scene);
        options.ready_timeout = Duration::from_secs(60);
        let app = launch_app(options).expect("launch browser");
        let result = app.wait_for_stderr(prefix, Duration::from_secs(150));
        let tail = app.stderr_tail(40).join("\n");
        match result {
            Ok(line) if line.starts_with(&format!("{prefix}OK")) => line,
            Ok(line) => panic!("{line}\n{tail}"),
            Err(e) => panic!("no {prefix} result: {e}\n{tail}"),
        }
    }

    #[test]
    fn real_adblock_cef() {
        let line = scene("cef", "adblock", "ND_ADBLOCK_");
        println!("{line}; ND_M8_ADBLOCK_CEF_OK");
    }

    #[test]
    fn real_adblock_cef_chrome() {
        let line = scene("cef-chrome", "adblock", "ND_ADBLOCK_");
        println!("{line}; ND_M8_ADBLOCK_CEF_CHROME_OK");
    }

    #[test]
    fn real_private_wkwebview() {
        let line = scene("system", "private", "ND_PRIVATE_");
        println!("{line}; ND_M8_PRIVATE_WKWEBVIEW_OK");
    }

    #[test]
    fn real_private_cef() {
        let line = scene("cef", "private", "ND_PRIVATE_");
        println!("{line}; ND_M8_PRIVATE_CEF_OK");
    }

    #[test]
    fn real_private_cef_chrome() {
        let line = scene("cef-chrome", "private", "ND_PRIVATE_");
        println!("{line}; ND_M8_PRIVATE_CEF_CHROME_OK");
    }

    fn real_allowed(name: &str) -> bool {
        let allowed = std::env::var_os("NEXTEST").is_some() || std::env::var_os("ND_REAL").is_some_and(|v| v == "1");
        if !allowed {
            println!("gate_m8_webview::{name}: skipped, it moves the real cursor");
        }
        allowed
    }

    fn find<'a>(node: &'a SemanticNode, test_id: &str) -> Option<&'a SemanticNode> {
        if node.test_id.as_deref() == Some(test_id) {
            return Some(node);
        }
        node.children.iter().find_map(|c| find(c, test_id))
    }

    #[derive(Clone, Copy, Debug)]
    struct Area {
        x: f64,
        y: f64,
        w: f64,
        h: f64,
    }

    impl Area {
        fn at(&self, fx: f64, fy: f64) -> (f64, f64) {
            (self.x + self.w * fx, self.y + self.h * fy)
        }

        fn contains(&self, p: (f64, f64)) -> bool {
            p.0 > self.x && p.0 < self.x + self.w && p.1 > self.y && p.1 < self.y + self.h
        }
    }

    struct Scene<'a> {
        app: &'a AppHandle,
    }

    impl Scene<'_> {
        fn window(&self) -> SemanticWindow {
            self.app.tree(None).expect("tree").windows.into_iter().next().expect("a window")
        }

        /// The node's rect in global top-left screen points.
        fn area(&self, test_id: &str) -> Option<Area> {
            let w = self.window();
            let frame = w.frame.expect("window frame");
            let content = w.root.bounds.expect("content bounds");
            let title = f64::from(frame.size.height - content.size.height);
            let b = find(&w.root, test_id)?.bounds?;
            Some(Area {
                x: f64::from(frame.origin.x + b.origin.x),
                y: f64::from(frame.origin.y) + title + f64::from(b.origin.y),
                w: f64::from(b.size.width),
                h: f64::from(b.size.height),
            })
        }

        fn status(&self) -> String {
            find(&self.window().root, "status").map(|n| n.name.clone()).unwrap_or_default()
        }

        /// The counts the page wrote into its title, which a web view reports as its name.
        fn page(&self) -> Option<InputCounts> {
            let window = self.window();
            let name = &find(&window.root, "web")?.name;
            InputCounts::parse(name).filter(|_| name.starts_with("down="))
        }

        fn wait(&self, what: &str, f: impl Fn(&Self) -> bool) {
            let start = Instant::now();
            while !f(self) {
                assert!(
                    start.elapsed() < Duration::from_secs(30),
                    "timed out waiting for {what}; status {}",
                    self.status()
                );
                std::thread::sleep(Duration::from_millis(30));
            }
        }

        fn status_has(&self, s: &str) {
            self.wait(s, |sc| sc.status().contains(s));
        }

        fn settled_page(&self) -> InputCounts {
            std::thread::sleep(Duration::from_millis(300));
            self.page().expect("page counts")
        }

        fn click(&self, test_id: &str) {
            self.app.get_by_test_id(test_id).click().expect("semantic click");
        }
    }

    fn wiggle(at: (f64, f64)) {
        for i in 0..6 {
            input::move_to((at.0 + f64::from(i) * 4., at.1 + f64::from(i % 2) * 3.));
            std::thread::sleep(Duration::from_millis(30));
        }
    }

    fn overlay_leak(engine: &str) {
        if let Err(e) = ndshell::preflight::real_input() {
            panic!("{e}");
        }
        let mut options = browser(engine, "overlays");
        options.ready_timeout = Duration::from_secs(60);
        let app = launch_app(options).expect("launch browser");
        let sc = Scene { app: &app };
        sc.wait("the page's counting script", |sc| sc.page().is_some());
        input::activate(app.pid());
        sc.wait("the window to be key", |sc| sc.window().key);
        let web = sc.area("web").expect("web bounds");
        let under = sc.area("under").expect("under bounds");

        // Controls: with nothing over them the page counts moves, presses and wheel.
        let before = sc.settled_page();
        let free = web.at(0.7, 0.85);
        wiggle(free);
        input::click(free);
        input::scroll(free, -60, 3);
        let after = sc.settled_page();
        assert!(after.moved > before.moved, "control: the page counts moves ({before} -> {after})");
        assert!(after.down > before.down && after.up > before.up, "control: the page counts presses ({after})");
        assert!(after.wheel > before.wheel, "control: the page counts wheel ({after})");
        input::click(under.at(0.5, 0.5));
        sc.status_has("under=1");

        // Popover: everything inside stays inside.
        sc.click("open-popover");
        sc.status_has("popover=open");
        sc.wait("the popover on screen", |sc| sc.area("pop").is_some_and(|p| p.w > 200.));
        std::thread::sleep(Duration::from_millis(400));
        let pop = sc.area("pop").expect("popover bounds");
        let over_button = (under.at(0.5, 0.5).0, under.at(0.5, 0.5).1.max(pop.y + 20.));
        let over_web = (web.x + 30., pop.at(0.5, 0.6).1);
        assert!(pop.contains(over_button) && under.contains(over_button), "popover {pop:?} covers the button");
        assert!(pop.contains(over_web) && web.contains(over_web), "popover {pop:?} covers the page {web:?}");
        input::move_to(over_web);
        let base = sc.settled_page();
        for at in [over_web, over_button] {
            wiggle(at);
            input::click(at);
            input::right_click(at);
            input::scroll(at, -80, 4);
        }
        input::move_to(over_web);
        let inside = sc.settled_page();
        assert!(sc.status().contains("popover=open"), "the popover closed during the run: {}", sc.status());
        assert_eq!(inside, base, "input inside the popover reached the page under it");
        assert!(sc.status().contains("under=1 "), "{}", sc.status());

        // Outside, on the page below the popover: closes it and is consumed, release included.
        let outside = (web.x + 40., (pop.y + pop.h + 30.).min(web.y + web.h - 10.));
        assert!(!pop.contains(outside) && web.contains(outside));
        input::move_to(outside);
        let base = sc.settled_page();
        input::click(outside);
        sc.status_has("popover=closed");
        let dismissed = sc.settled_page();
        assert_eq!((dismissed.down, dismissed.up), (base.down, base.up), "the dismissing press reached the page");
        input::click(outside);
        let live = sc.settled_page();
        assert_eq!((live.down, live.up), (base.down + 1, base.up + 1), "with the popover gone the page takes presses");

        // Modal panel: blocks presses outside, nothing inside leaks, Esc closes it.
        sc.click("open-panel");
        sc.status_has("panel=open");
        sc.wait("the panel on screen", |sc| sc.area("panel").is_some_and(|p| p.w > 200.));
        std::thread::sleep(Duration::from_millis(300));
        let panel = sc.area("panel").expect("panel bounds");
        let base = sc.settled_page();
        let in_panel = panel.at(0.5, 0.5);
        wiggle(in_panel);
        input::click(in_panel);
        input::scroll(in_panel, -60, 3);
        input::click(under.at(0.5, 0.5));
        input::click(outside);
        let blocked = sc.settled_page();
        assert_eq!(
            (blocked.down, blocked.up, blocked.wheel),
            (base.down, base.up, base.wheel),
            "modal panel: {blocked}"
        );
        assert!(sc.status().contains("under=1 ") && sc.status().contains("panel=open"), "{}", sc.status());
        input::key(53, &[]);
        sc.status_has("panel=closed");

        // Context menu over the page: choosing an item works, the page sees nothing, a press
        // outside closes it and is consumed.
        let zone = sc.area("menu-zone").expect("menu zone bounds");
        let open_at = zone.at(0.8, 0.2);
        input::right_click(open_at);
        std::thread::sleep(Duration::from_millis(500));
        let item = (open_at.0 + 40., open_at.1 + 12.);
        assert!(web.contains((open_at.0 + 80., item.1)), "the menu opens over the page");
        let base = sc.settled_page();
        wiggle((open_at.0 + 60., item.1));
        input::click(item);
        sc.status_has("chosen=1");
        input::right_click(open_at);
        std::thread::sleep(Duration::from_millis(500));
        input::click(under.at(0.5, 0.5));
        std::thread::sleep(Duration::from_millis(400));
        assert!(sc.status().contains("under=1 "), "the press that closed the menu clicked the button: {}", sc.status());
        let after_menu = sc.settled_page();
        assert_eq!(
            (after_menu.down, after_menu.up, after_menu.wheel),
            (base.down, base.up, base.wheel),
            "menu: {after_menu}"
        );
        input::click(under.at(0.5, 0.5));
        sc.status_has("under=2");
        println!("ND_M8_OVERLAY_LEAK_{}_OK", engine.to_uppercase().replace('-', "_"));
    }

    #[test]
    fn real_overlay_leak_wkwebview() {
        if real_allowed("real_overlay_leak_wkwebview") {
            overlay_leak("system");
        }
    }

    #[test]
    fn real_overlay_leak_cef() {
        if real_allowed("real_overlay_leak_cef") {
            overlay_leak("cef");
        }
    }

    #[test]
    fn real_overlay_leak_cef_chrome() {
        if real_allowed("real_overlay_leak_cef_chrome") {
            overlay_leak("cef-chrome");
        }
    }
}
