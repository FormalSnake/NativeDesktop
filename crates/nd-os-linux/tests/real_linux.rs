//! Tier 3 on Linux: `examples/hello` on the GPUI backend with `LinuxHooks`, in a private headless
//! Hyprland per test (`ndshell::hyprland`), driven by the virtual pointer and keyboard and read
//! back through output capture and hello's `ND_DIAG_WINDOW` lines. Each test owns its compositor,
//! seat and cursor, so they run in parallel and never touch the user's session. Needs a built
//! hello next to this test binary (`cargo build -p hello`) or `ND_HELLO_BIN`. `ND_GATE_OUT=<dir>`
//! keeps BMPs of the resize gate's idle and mid-drag frames and of any frame that fails.
//!
//! The resize gate (ARCHITECTURE.md 5.2, Linux leg): `ND_DEBUG_EXPOSE=1` paints magenta behind
//! hello's tree; the virtual pointer drags the right and then the bottom border, so
//! Hyprland runs its own interactive resize and sends a configure per motion; every frame the
//! output presents is captured. Inside the green border no frame may show magenta (content laid
//! out late) or the blue compositor background (a buffer that lags the window). A control run
//! that lays the content out 300 ms late must show magenta, which proves the detector works.

#![cfg(target_os = "linux")]

mod common;

mod gate_m2 {
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use ndshell::hyprland::{Client, Hyprland, Options};
    use ndshell::wayland::{Modifier, VirtualInput, keys, screenshot};

    use crate::common::probe::Probe;
    use crate::common::resize::{Recording, analyze, edge_drags, gate_out, phase, record, set_phase};

    /// What hello reports under Hyprland: server-side, no minimize, nothing drawn, no insets.
    pub const HYPRLAND_CHROME: &str =
        "decorations=server controls=none caps=maximize,fullscreen fullscreen=false insets=0,0";

    /// `examples/hello`, built into the same target dir as this test (`target/debug/deps/..`).
    pub fn hello_bin() -> PathBuf {
        if let Some(bin) = std::env::var_os("ND_HELLO_BIN") {
            return bin.into();
        }
        let exe = std::env::current_exe().unwrap();
        let bin = exe.parent().and_then(|deps| deps.parent()).unwrap().join("hello");
        assert!(bin.exists(), "build hello first (cargo build -p hello) or set ND_HELLO_BIN; looked at {bin:?}");
        bin
    }

    /// hello launched, drawn and mapped.
    fn ready(hyprland: &Hyprland, env: &[(&str, &str)]) -> (Probe, Client) {
        Probe::ready(hyprland, hello_bin(), env, HYPRLAND_CHROME)
    }

    fn start() -> Hyprland {
        Hyprland::start(Options::default()).unwrap_or_else(|e| panic!("headless Hyprland: {e}"))
    }

    #[test]
    fn real_resize_drag() {
        let hyprland = start();
        let (probe, client) = ready(&hyprland, &[("ND_DEBUG_EXPOSE", "1")]);
        let mut input = VirtualInput::connect(&hyprland).unwrap();
        let recording = Arc::new(Mutex::new(Recording::default()));
        if let Some(out) = gate_out() {
            let mut r = recording.lock().unwrap();
            r.keep = vec![("idle", 0, out.join("idle.bmp")), ("drag", 20, out.join("mid-drag.bmp"))];
            r.offending = Some(out.clone());
        }
        let capture = record(&hyprland, recording.clone());
        set_phase(&recording, Some("idle"));
        input.move_to(5., 5.);
        input.sync().unwrap();
        std::thread::sleep(Duration::from_millis(150));

        set_phase(&recording, Some("drag"));
        edge_drags(&mut input, &client, 240.);
        set_phase(&recording, None);
        let after = hyprland.wait_for_client(probe.child.id(), Duration::from_secs(1)).unwrap();
        let count = capture.stop().unwrap();

        let layouts = probe.lines.lock().unwrap().iter().filter(|l| l.starts_with("nd: viewport")).count();
        println!("hello laid out {layouts} sizes");
        let idle = phase(&recording, "idle");
        let frames = phase(&recording, "drag");
        let mut sizes: Vec<(u32, u32)> = frames.iter().map(|f| f.size).collect();
        sizes.sort_unstable();
        sizes.dedup();
        let magenta = frames.iter().filter(|f| f.magenta > 0).count();
        let background = frames.iter().filter(|f| f.background > 0).count();
        println!(
            "{count} frames captured; drag: {} frames, {} sizes, {magenta} with magenta, {background} with background; \
             window {:?} -> {:?}",
            frames.len(),
            sizes.len(),
            client.size,
            after.size
        );
        assert!(!idle.is_empty() && idle.iter().all(|f| f.magenta == 0), "idle frames: {idle:?}");
        assert!(
            after.size.0 >= client.size.0 + 200 && after.size.1 >= client.size.1 + 120,
            "the border drags resized the window: {:?} -> {:?}",
            client.size,
            after.size
        );
        assert!(frames.len() >= 30 && sizes.len() >= 10, "{} frames at {} sizes", frames.len(), sizes.len());
        assert_eq!(magenta, 0, "content reaches the window edge in every presented frame");
        assert_eq!(background, 0, "the buffer covers the window in every presented frame");
        assert!(
            probe.last("nd: viewport").is_some_and(|l| l.ends_with(&format!("{}x{}", after.size.0, after.size.1))),
            "hello laid out at the final size: {:?} vs {:?}",
            probe.last("nd: viewport"),
            after.size
        );
        drop(probe);

        // Control: content laid out 300 ms late must be caught.
        let (control, client) =
            ready(&hyprland, &[("ND_DEBUG_EXPOSE", "1"), ("ND_DEBUG_DEFER_LIVE_LAYOUT", "1")]);
        let recording = Arc::new(Mutex::new(Recording::default()));
        if let Some(out) = gate_out() {
            recording.lock().unwrap().keep = vec![("control", 20, out.join("control.bmp"))];
        }
        let capture = record(&hyprland, recording.clone());
        set_phase(&recording, Some("control"));
        edge_drags(&mut input, &client, 160.);
        capture.stop().unwrap();
        let frames = phase(&recording, "control");
        let magenta = frames.iter().filter(|f| f.magenta > 0).count();
        println!("control: {magenta} of {} frames show the window background", frames.len());
        assert!(magenta > 0, "the detector sees late layout ({} frames)", frames.len());
        drop(control);
        println!("ND_M2_LINUX_RESIZE_GATE_OK");
    }

    #[test]
    fn real_no_controls_under_hyprland() {
        let hyprland = start();
        // `ready` waited for HYPRLAND_CHROME; nothing may follow it.
        let (probe, _) = ready(&hyprland, &[]);
        std::thread::sleep(Duration::from_millis(300));
        let line = probe.last("nd: chrome").unwrap_or_default();
        assert!(line.ends_with(HYPRLAND_CHROME), "Hyprland answers server-side, nothing drawn, no insets: {line}");
        assert!(analyze(&screenshot(&hyprland).unwrap()).is_some(), "hello's window is on the output");
    }

    /// Weston has no xdg-decoration, so hello draws its own controls: close always, and what
    /// Weston's `wm_capabilities` offers, at GNOME's default layout's side with room reserved.
    #[test]
    fn real_controls_under_client_decorations() {
        let weston = ndshell::weston::Weston::start(1280, 800).unwrap_or_else(|e| panic!("headless weston: {e}"));
        let app = Probe::launch(weston.command(hello_bin()), &[]);
        let end = Instant::now() + Duration::from_secs(10);
        while Instant::now() < end && !app.has("decorations=client") {
            std::thread::sleep(Duration::from_millis(20));
        }
        let line = app.last("nd: chrome").unwrap_or_default();
        assert!(line.contains("decorations=client"), "no xdg-decoration means client-side:\n{}", app.log());
        assert!(line.contains("close"), "close is always drawn: {line}");
        assert!(!line.contains("insets=0,0"), "the drawn controls reserve titlebar room: {line}");
    }

    #[test]
    fn real_drag_region_moves_window() {
        let hyprland = start();
        let (probe, before) = ready(&hyprland, &[]);
        let mut input = VirtualInput::connect(&hyprland).unwrap();
        let (x, y) = (f64::from(before.at.0), f64::from(before.at.1));
        // hello's toolbar row is the title row; press in its middle, clear of the "Add one" button.
        // The compositor follows motion only once hello's `xdg_toplevel.move` reached it, so the
        // pointer waits for hello to report the request (it lags the press under load).
        // A hand arrives over the row before it presses: hello sees the pointer enter and hover first.
        let target = (x + f64::from(before.size.0) / 2., y + 12.);
        for i in (0..=5).rev() {
            input.move_to(target.0 - f64::from(i) * 8., target.1 + f64::from(i) * 4.);
            std::thread::sleep(Duration::from_millis(20));
        }
        input.sync().unwrap();
        std::thread::sleep(Duration::from_millis(50));
        input.button(true);
        input.sync().unwrap();
        assert!(
            probe.wait_for("nd: window move", Duration::from_secs(3)),
            "no move from the press at {target:?} (window {before:?}):\n{}\nHyprland:\n{}",
            probe.log(),
            hyprland.log_tail()
        );
        std::thread::sleep(Duration::from_millis(30));
        let (px, py) = input.position();
        for i in 1..=30 {
            let t = f64::from(i) / 30.;
            input.move_to(px + 150. * t, py + 90. * t);
            std::thread::sleep(Duration::from_millis(16));
        }
        input.sync().unwrap();
        input.button(false);
        input.sync().unwrap();
        std::thread::sleep(Duration::from_millis(100));
        let after = hyprland.wait_for_client(probe.child.id(), Duration::from_secs(1)).unwrap();
        let moved = (after.at.0 - before.at.0, after.at.1 - before.at.1);
        assert!(
            (moved.0 - 150).abs() <= 4 && (moved.1 - 90).abs() <= 4,
            "xdg_toplevel.move from the drag region follows the pointer: moved {moved:?}"
        );
        assert_eq!(after.size, before.size, "a move, not a resize");

        // Content below the title row is not a drag region.
        let content = (f64::from(after.at.0) + 200., f64::from(after.at.1) + 300.);
        input.drag(content, 100., 0., 20, Duration::from_millis(16)).unwrap();
        let still = hyprland.wait_for_client(probe.child.id(), Duration::from_secs(1)).unwrap();
        assert_eq!(still.at, after.at, "dragging content does not move the window");
    }

    #[test]
    fn real_virtual_keyboard_reaches_app() {
        let hyprland = start();
        let (probe, client) = ready(&hyprland, &[]);
        let mut input = VirtualInput::connect(&hyprland).unwrap();
        input.click(f64::from(client.at.0) + 100., f64::from(client.at.1) + 200.).unwrap();
        input.key(keys::A, &[]).unwrap();
        input.key(keys::S, &[Modifier::Control]).unwrap();
        assert!(probe.wait_for("nd: key a", Duration::from_secs(2)), "plain key:\n{}", probe.log());
        assert!(probe.wait_for("nd: key ctrl-s", Duration::from_secs(2)), "chord:\n{}", probe.log());
    }
}
