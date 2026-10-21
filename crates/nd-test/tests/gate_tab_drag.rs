//! Tab drag and the edge-revealed sidebar on macOS, through the real cursor (the press, AppKit's
//! dragging session and the drop are the window server's):
//!
//! - `real_tab_drag`: `examples/browser` with `BROWSER_SCENE=tabdrag`, the cases of the browser's
//!   `scripts/mac-tabdrag.ts`. Two windows side by side; the counter page, its field filled, is
//!   dragged within its window, into the other window and back, and the other window's last tab
//!   is dragged into the first so the other window closes. A move between windows carries the
//!   live page: it keeps counting, keeps the field's text, and the server sees one load.
//! - `real_sidebar_reveal`: `BROWSER_SCENE=reveal` with `ND_REVEAL_TRACE=1`: the pointer at the
//!   leading edge slides the hidden sidebar in, leaving it slides it out, and on every frame of
//!   both slides the close button keeps its place on the panel.
//!
//! They move the user's cursor: nextest only, or `ND_REAL=1`.

#[cfg(target_os = "macos")]
mod gate_tab_drag {
    use std::time::{Duration, Instant};

    use nd_core::semantic::{SemanticNode, SemanticWindow};
    use nd_test::{AppHandle, LaunchOptions, launch_app};
    use ndshell::input;

    fn real_allowed(name: &str) -> bool {
        let allowed = std::env::var_os("NEXTEST").is_some() || std::env::var_os("ND_REAL").is_some_and(|v| v == "1");
        if !allowed {
            println!("gate_tab_drag::{name}: skipped, it moves the real cursor");
        }
        allowed
    }

    fn find<'a>(node: &'a SemanticNode, test_id: &str) -> Option<&'a SemanticNode> {
        if node.test_id.as_deref() == Some(test_id) {
            return Some(node);
        }
        node.children.iter().find_map(|c| find(c, test_id))
    }

    struct Scene<'a> {
        app: &'a AppHandle,
    }

    impl Scene<'_> {
        fn windows(&self) -> Vec<SemanticWindow> {
            self.app.tree(None).map(|t| t.windows).unwrap_or_default()
        }

        fn window(&self, title: &str) -> Option<SemanticWindow> {
            self.windows().into_iter().find(|w| w.title == title)
        }

        /// What the window's `state` text says: `tabs=.. sel=.. loads=.. title=..`.
        fn state(&self, title: &str) -> String {
            self.window(title).and_then(|w| find(&w.root, "state").map(|n| n.name.clone())).unwrap_or_default()
        }

        fn field(&self, title: &str, key: &str) -> String {
            let state = self.state(title);
            let at = state.find(&format!("{key}=")).map(|i| i + key.len() + 1);
            at.map(|i| {
                let rest = &state[i..];
                if key == "title" { rest.to_owned() } else { rest.split(' ').next().unwrap_or("").to_owned() }
            })
            .unwrap_or_default()
        }

        /// The counter's count, from its title (`Counter c=<n> f=<field>`).
        fn count(&self, title: &str) -> Option<u32> {
            let t = self.field(title, "title");
            t.strip_prefix("Counter c=")?.split(' ').next()?.parse().ok()
        }

        fn wait(&self, what: &str, f: impl Fn(&Self) -> bool) {
            let start = Instant::now();
            while !f(self) {
                let states: Vec<String> =
                    self.windows().iter().map(|w| format!("{}: {}", w.title, self.state(&w.title))).collect();
                assert!(start.elapsed() < Duration::from_secs(30), "timed out waiting for {what}; {states:?}");
                std::thread::sleep(Duration::from_millis(50));
            }
        }

        /// A point of the window's content, `(x, y)` in its content coordinates, on screen.
        fn screen(&self, title: &str, x: f64, y: f64) -> (f64, f64) {
            let w = self.window(title).expect("window");
            let frame = w.frame.expect("window frame");
            let content = w.root.bounds.expect("content bounds");
            let bar = f64::from(frame.size.height - content.size.height);
            (f64::from(frame.origin.x) + x, f64::from(frame.origin.y) + bar + y)
        }

        /// The list's rows by id with their top and bottom in content coordinates, found by
        /// clicking down the list and reading which row each click selected. A row is where a
        /// click first selects it, one row pitch tall; the row selected before the sweep is only
        /// found when another row is clicked first.
        fn rows(&self, title: &str) -> Vec<(String, f64, f64)> {
            let w = self.window(title).expect("window");
            let list = find(&w.root, "tabs").and_then(|n| n.bounds).expect("list bounds");
            let x = f64::from(list.origin.x + list.size.width / 2.);
            let mut starts: Vec<(String, f64)> = Vec::new();
            let mut y = f64::from(list.origin.y) + 2.;
            let bottom = f64::from(list.origin.y + list.size.height).min(f64::from(list.origin.y) + 220.);
            while y < bottom {
                let before = self.field(title, "sel");
                input::click(self.screen(title, x, y));
                let start = Instant::now();
                while self.field(title, "sel") == before && start.elapsed() < Duration::from_millis(150) {
                    std::thread::sleep(Duration::from_millis(10));
                }
                let sel = self.field(title, "sel");
                if sel != before {
                    starts.push((sel, y));
                }
                y += 4.;
            }
            let pitch = starts.windows(2).map(|w| w[1].1 - w[0].1).fold(f64::MAX, f64::min);
            let pitch = if pitch == f64::MAX { 20. } else { pitch };
            starts.into_iter().map(|(id, top)| (id, top, top + pitch - 4.)).collect()
        }

        /// Below the list's rows, in content coordinates: a drop there appends.
        fn below_rows(&self, title: &str) -> f64 {
            let w = self.window(title).expect("window");
            let list = find(&w.root, "tabs").and_then(|n| n.bounds).expect("list bounds");
            f64::from(list.origin.y + list.size.height) - 20.
        }
    }

    fn row<'r>(rows: &'r [(String, f64, f64)], id: &str) -> &'r (String, f64, f64) {
        rows.iter().find(|r| r.0 == id).unwrap_or_else(|| panic!("no row {id} in {rows:?}"))
    }

    fn drag(from: (f64, f64), to: (f64, f64)) {
        input::drag(from, to.0 - from.0, to.1 - from.1, 48);
        std::thread::sleep(Duration::from_millis(400));
    }

    #[test]
    fn real_tab_drag() {
        if !real_allowed("real_tab_drag") {
            return;
        }
        if let Err(e) = ndshell::preflight::real_input() {
            panic!("{e}");
        }
        let mut options = LaunchOptions::example("browser").env("BROWSER_SCENE", "tabdrag");
        options.ready_timeout = Duration::from_secs(60);
        let app = launch_app(options).expect("launch browser");
        let sc = Scene { app: &app };
        sc.wait("both windows", |sc| sc.window("w1").is_some() && sc.window("w2").is_some());
        sc.wait("the counter filled", |sc| sc.field("w1", "title").contains("f=typed-by-drive"));
        input::activate(app.pid());
        assert_eq!(sc.field("w1", "tabs"), "*1,2,3");
        assert_eq!(sc.field("w1", "loads"), "1");

        let w1 = sc.rows("w1");
        let x = |title: &str| {
            let w = sc.window(title).expect("window");
            f64::from(find(&w.root, "tabs").and_then(|n| n.bounds).expect("list").size.width / 2.)
        };
        // Leg 1: the counter goes below page A in its own window.
        let (_, top, bottom) = row(&w1, "2").clone();
        let (_, _, a_bottom) = row(&w1, "3").clone();
        drag(sc.screen("w1", x("w1"), (top + bottom) / 2.), sc.screen("w1", x("w1"), a_bottom - 2.));
        sc.wait("the reorder", |sc| sc.field("w1", "tabs") == "*1,3,2");
        assert_eq!(sc.field("w1", "loads"), "1", "reordering reloaded the page");

        // Leg 2: into w2, below page B. The live page comes along.
        let w1 = sc.rows("w1");
        let (_, top, bottom) = row(&w1, "2").clone();
        drag(sc.screen("w1", x("w1"), (top + bottom) / 2.), sc.screen("w2", x("w2"), sc.below_rows("w2")));
        sc.wait("the move into w2", |sc| sc.field("w2", "tabs") == "4,2" && sc.field("w2", "sel") == "2");
        assert_eq!(sc.field("w1", "tabs"), "*1,3", "the counter left w1");
        sc.wait("the counter counting in w2", |sc| sc.count("w2").is_some());
        let first = sc.count("w2").expect("count");
        sc.wait("the counter still counting", |sc| sc.count("w2").is_some_and(|c| c > first));
        assert!(sc.field("w2", "title").contains("f=typed-by-drive"), "the field came across as {}", sc.state("w2"));
        assert_eq!(sc.field("w2", "loads"), "1", "the move into w2 reloaded the page");

        // Leg 3: back into w1, below page A.
        let w2 = sc.rows("w2");
        let w1 = sc.rows("w1");
        let (_, top, bottom) = row(&w2, "2").clone();
        let (_, _, a_bottom) = row(&w1, "3").clone();
        drag(sc.screen("w2", x("w2"), (top + bottom) / 2.), sc.screen("w1", x("w1"), a_bottom - 2.));
        sc.wait("the move back", |sc| sc.field("w1", "tabs") == "*1,3,2");
        assert!(sc.field("w1", "title").contains("f=typed-by-drive"), "came back as {}", sc.state("w1"));
        assert_eq!(sc.field("w1", "loads"), "1", "the move back reloaded the page");

        // Leg 4: w2's last tab into w1 closes w2. Page B kept its row from leg 3: with one row
        // a sweep has nothing else to select.
        let w1 = sc.rows("w1");
        let (_, top, bottom) = row(&w2, "4").clone();
        let (_, _, last) = row(&w1, "2").clone();
        drag(sc.screen("w2", x("w2"), (top + bottom) / 2.), sc.screen("w1", x("w1"), last - 2.));
        sc.wait("w2 closed", |sc| sc.window("w2").is_none());
        assert_eq!(sc.field("w1", "tabs"), "*1,3,2,4");
        println!("ND_TAB_DRAG_OK");
    }

    #[test]
    fn real_sidebar_reveal() {
        if !real_allowed("real_sidebar_reveal") {
            return;
        }
        if let Err(e) = ndshell::preflight::real_input() {
            panic!("{e}");
        }
        let mut options = LaunchOptions::example("browser")
            .env("BROWSER_SCENE", "reveal")
            .env("ND_REVEAL_TRACE", "1")
            .env("ND_ANIMATION_SLOWDOWN", "4");
        options.ready_timeout = Duration::from_secs(60);
        let app = launch_app(options).expect("launch browser");
        let sc = Scene { app: &app };
        sc.wait("the window", |sc| sc.window("Reveal").is_some());
        input::activate(app.pid());
        sc.wait("the window to be key", |sc| sc.window("Reveal").is_some_and(|w| w.key));
        let shown = |sc: &Scene| sc.state("Reveal");
        input::move_to(sc.screen("Reveal", 400., 300.));
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(shown(&sc), "revealed=false");
        input::move_to(sc.screen("Reveal", 2., 300.));
        sc.wait("the reveal", |sc| shown(sc) == "revealed=true");
        std::thread::sleep(Duration::from_millis(1200));
        input::move_to(sc.screen("Reveal", 120., 300.));
        std::thread::sleep(Duration::from_millis(600));
        assert_eq!(shown(&sc), "revealed=true", "the pointer over the panel keeps it");
        input::move_to(sc.screen("Reveal", 700., 300.));
        sc.wait("the conceal", |sc| shown(sc) == "revealed=false");
        std::thread::sleep(Duration::from_millis(1200));

        let frames: Vec<(String, f64, f64)> = app
            .stderr_tail(5000)
            .iter()
            .filter_map(|l| {
                let l = l.split("ND_REVEAL_FRAME ").nth(1)?;
                let value = |k: &str| l.split(' ').find_map(|p| p.strip_prefix(&format!("{k}=")).map(str::to_owned));
                Some((value("dir")?, value("panel")?.parse().ok()?, value("close")?.parse().ok()?))
            })
            .collect();
        for dir in ["in", "out"] {
            let gaps: Vec<f64> = frames.iter().filter(|f| f.0 == dir).map(|f| f.2 - f.1).collect();
            assert!(gaps.len() >= 8, "{dir}: only {} traced frames: {frames:?}", gaps.len());
            let (lo, hi) = gaps.iter().fold((f64::MAX, f64::MIN), |(lo, hi), g| (lo.min(*g), hi.max(*g)));
            let raw: Vec<(f64, f64)> = frames.iter().filter(|f| f.0 == dir).map(|f| (f.1, f.2)).collect();
            assert!(hi - lo <= 1., "{dir}: the close button drifted on the panel by {} pt: {raw:?}", hi - lo);
            let travel = frames
                .iter()
                .filter(|f| f.0 == dir)
                .map(|f| f.1)
                .fold((f64::MAX, f64::MIN), |(a, b), x| (a.min(x), b.max(x)));
            assert!(travel.1 - travel.0 > 100., "{dir}: the panel did not slide: {travel:?}");
        }
        println!("ND_SIDEBAR_REVEAL_OK");
    }
}
