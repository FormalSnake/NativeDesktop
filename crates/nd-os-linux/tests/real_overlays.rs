//! M4 overlay leak gate, tier 3 on Linux (ARCHITECTURE.md 6.3, 16): `examples/overlays` on the
//! GPUI backend in a private headless Hyprland (`ndshell::hyprland`), driven by the compositor's
//! virtual pointer and keyboard, read back over the automation socket. A popover and a modal
//! panel open over a native button and a GPU surface that counts every press, release and move it
//! receives. Inside an overlay neither sees anything; a press outside the popover closes it and
//! is consumed; the panel blocks presses outside it and Esc closes it. Positive controls show the
//! same input reaching the surface and the button with nothing over them. Needs `overlays` built
//! next to this test (`cargo build -p overlays`) or `ND_OVERLAYS_BIN`.

#![cfg(target_os = "linux")]

mod gate_m4_overlays {
    use std::path::PathBuf;
    use std::process::{Child, Stdio};
    use std::time::{Duration, Instant};

    use nd_core::semantic::SemanticNode;
    use nd_test::{Session, connect_app};
    use nd_ui::InputCounts;
    use ndshell::hyprland::{Hyprland, Options};
    use ndshell::wayland::{VirtualInput, keys};

    fn overlays_bin() -> PathBuf {
        if let Some(bin) = std::env::var_os("ND_OVERLAYS_BIN") {
            return bin.into();
        }
        let exe = std::env::current_exe().unwrap();
        let bin = exe.parent().and_then(|deps| deps.parent()).unwrap().join("overlays");
        assert!(bin.exists(), "build overlays first (cargo build -p overlays); looked at {bin:?}");
        bin
    }

    fn find<'a>(node: &'a SemanticNode, test_id: &str) -> Option<&'a SemanticNode> {
        if node.test_id.as_deref() == Some(test_id) {
            return Some(node);
        }
        node.children.iter().find_map(|c| find(c, test_id))
    }

    struct App {
        child: Child,
        session: Session,
        origin: (f64, f64),
    }

    impl Drop for App {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
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

        fn counts(&self) -> InputCounts {
            std::thread::sleep(Duration::from_millis(200));
            let value = find(&self.root(), "web").and_then(|n| n.value.clone()).expect("surface counts");
            InputCounts::parse(&value).expect("counts")
        }

        fn wait(&self, what: &str, f: impl Fn(&Self) -> bool) {
            let start = Instant::now();
            while !f(self) {
                assert!(start.elapsed() < Duration::from_secs(10), "timed out waiting for {what}: {}", self.status());
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
            input.move_to(at.0 + f64::from(i) * 4., at.1);
            let _ = input.sync();
            std::thread::sleep(Duration::from_millis(16));
        }
    }

    fn click(input: &mut VirtualInput, at: (f64, f64)) {
        input.click(at.0, at.1).unwrap();
        std::thread::sleep(Duration::from_millis(150));
    }

    #[test]
    fn real_overlay_leak_hyprland() {
        let hyprland = Hyprland::start(Options::default()).unwrap_or_else(|e| panic!("headless Hyprland: {e}"));
        let dir = std::env::temp_dir().join(format!("nd-overlays-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let socket = dir.join("a.sock");
        let child = hyprland
            .command(overlays_bin())
            .env("NATIVE_AUTOMATION", "1")
            .env("ND_AUTOMATION_SOCKET", &socket)
            .env("ND_APP_DATA_DIR", dir.join("data"))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("overlays starts");
        let start = Instant::now();
        let session = loop {
            if let Ok(session) = connect_app(&socket) {
                break session;
            }
            assert!(start.elapsed() < Duration::from_secs(20), "no automation socket at {}", socket.display());
            std::thread::sleep(Duration::from_millis(50));
        };
        let client = hyprland.wait_for_client(child.id(), Duration::from_secs(10)).expect("overlays maps a window");
        let app = App { child, session, origin: (f64::from(client.at.0), f64::from(client.at.1)) };
        app.wait("the surface laid out", |a| a.at("web", 0.5, 0.5).is_some());
        let mut input = VirtualInput::connect(&hyprland).unwrap();

        // Controls: with nothing over them, the surface counts input and the button clicks.
        let free = app.at("web", 0.7, 0.85).unwrap();
        let under = app.at("under", 0.5, 0.5).unwrap();
        let before = app.counts();
        wiggle(&mut input, free);
        click(&mut input, free);
        let after = app.counts();
        assert!(
            after.moved > before.moved && after.down > before.down && after.up > before.up,
            "control: {before} -> {after}"
        );
        click(&mut input, under);
        app.status_has("under=1");

        // Popover over the button and the surface.
        app.click_node("open-popover");
        app.status_has("popover=open");
        app.wait("the popover laid out", |a| a.at("pop", 0.5, 0.5).is_some());
        let pop = |fx, fy| app.at("pop", fx, fy).unwrap();
        let (top_left, bottom_right) = (pop(0., 0.), pop(1., 1.));
        let inside =
            |p: (f64, f64)| p.0 > top_left.0 && p.0 < bottom_right.0 && p.1 > top_left.1 && p.1 < bottom_right.1;
        let over_button = (under.0, under.1.max(top_left.1 + 10.));
        let web_left = app.at("web", 0., 0.5).unwrap().0;
        let over_web = (web_left + 20., pop(0.5, 0.6).1);
        assert!(inside(over_button) && inside(over_web), "the popover covers both: {top_left:?} {bottom_right:?}");
        let base = app.counts();
        for at in [over_web, over_button] {
            wiggle(&mut input, at);
            click(&mut input, at);
        }
        input.move_to(over_web.0, over_web.1);
        let _ = input.sync();
        let inner = app.counts();
        assert!(app.status().contains("popover=open"), "a press inside closed the popover: {}", app.status());
        assert_eq!(inner, base, "input inside the popover reached the surface under it");
        assert!(app.status().contains("under=1 "), "the button under the popover was clicked: {}", app.status());

        // Outside, on the surface below the popover: closes it, consumed with its release.
        let outside = (web_left + 40., bottom_right.1 + 30.);
        assert!(!inside(outside));
        input.move_to(outside.0, outside.1);
        let _ = input.sync();
        let base = app.counts();
        click(&mut input, outside);
        app.status_has("popover=closed");
        let dismissed = app.counts();
        assert_eq!((dismissed.down, dismissed.up), (base.down, base.up), "the dismissing press reached the surface");
        click(&mut input, outside);
        let live = app.counts();
        assert_eq!(
            (live.down, live.up),
            (base.down + 1, base.up + 1),
            "with the popover gone the surface takes presses"
        );

        // Modal panel: presses outside are blocked, Esc closes it.
        app.click_node("open-panel");
        app.status_has("panel=open");
        app.wait("the panel laid out", |a| a.at("panel", 0.5, 0.5).is_some());
        let base = app.counts();
        click(&mut input, app.at("panel", 0.5, 0.5).unwrap());
        click(&mut input, under);
        click(&mut input, outside);
        let blocked = app.counts();
        assert_eq!((blocked.down, blocked.up), (base.down, base.up), "the modal panel let a press through: {blocked}");
        assert!(app.status().contains("under=1 ") && app.status().contains("panel=open"), "{}", app.status());
        input.key(keys::ESC, &[]).unwrap();
        app.status_has("panel=closed");
        click(&mut input, under);
        app.status_has("under=2");
        drop(app);
        let _ = std::fs::remove_dir_all(&dir);
        println!("ND_M4_OVERLAY_LEAK_HYPRLAND_OK");
    }
}
