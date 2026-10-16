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

mod gate_m2 {
    use std::io::{BufRead, BufReader};
    use std::path::PathBuf;
    use std::process::{Child, Command, Stdio};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use ndshell::hyprland::{BACKGROUND, BORDER, BORDER_SIZE, Client, Hyprland, Options};
    use ndshell::wayland::{Capture, CapturedFrame, Modifier, VirtualInput, keys, screenshot};

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

    pub struct Probe {
        child: Child,
        lines: Arc<Mutex<Vec<String>>>,
    }

    impl Probe {
        /// hello as a client of `command`'s compositor, with `ND_DIAG_WINDOW` on and its data
        /// (settings, logs, bundles) in a dir of its own.
        pub fn launch(mut command: Command, env: &[(&str, &str)]) -> Self {
            let data = std::env::temp_dir().join(format!("nd-hello-{}-{:?}", std::process::id(), Instant::now()));
            command
                .env("ND_DIAG_WINDOW", "1")
                .env("ND_APP_DATA_DIR", &data)
                .envs(env.iter().copied())
                .stdout(Stdio::null())
                .stderr(Stdio::piped());
            let mut child = command.spawn().expect("hello starts");
            let lines = Arc::new(Mutex::new(Vec::new()));
            let stderr = child.stderr.take().unwrap();
            let sink = lines.clone();
            std::thread::spawn(move || {
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    sink.lock().unwrap().push(line);
                }
            });
            Self { child, lines }
        }

        pub fn has(&self, needle: &str) -> bool {
            self.lines.lock().unwrap().iter().any(|l| l.contains(needle))
        }

        pub fn wait_for(&self, needle: &str, timeout: Duration) -> bool {
            let end = Instant::now() + timeout;
            while Instant::now() < end {
                if self.has(needle) {
                    return true;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            false
        }

        pub fn last(&self, prefix: &str) -> Option<String> {
            self.lines.lock().unwrap().iter().rev().find(|l| l.starts_with(prefix)).cloned()
        }

        pub fn log(&self) -> String {
            self.lines.lock().unwrap().join("\n")
        }

        /// Launched, drawn, and mapped as a window.
        fn ready(hyprland: &Hyprland, env: &[(&str, &str)]) -> (Self, Client) {
            let probe = Self::launch(hyprland.command(hello_bin()), env);
            assert!(
                probe.wait_for(HYPRLAND_CHROME, Duration::from_secs(10)),
                "hello did not report Hyprland's chrome:\n{}\nHyprland:\n{}",
                probe.log(),
                hyprland.log_tail()
            );
            // Settled: the same geometry on two reads 100 ms apart.
            let pid = probe.child.id();
            let mut client = hyprland.wait_for_client(pid, Duration::from_secs(5)).unwrap();
            let end = Instant::now() + Duration::from_secs(3);
            loop {
                std::thread::sleep(Duration::from_millis(100));
                let again = hyprland.wait_for_client(pid, Duration::from_secs(1)).unwrap();
                if again == client || Instant::now() > end {
                    break;
                }
                client = again;
            }
            assert!(client.floating, "the runner's config floats every window: {client:?}");
            (probe, client)
        }
    }

    impl Drop for Probe {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    fn start() -> Hyprland {
        Hyprland::start(Options::default()).unwrap_or_else(|e| panic!("headless Hyprland: {e}"))
    }

    fn near(c: [u8; 3], target: [u8; 3]) -> bool {
        c.iter().zip(target).all(|(a, b)| a.abs_diff(b) <= 24)
    }

    /// One analyzed frame: the window's extent inside its border and what shows inside it.
    #[derive(Clone, Copy, Debug, Default)]
    struct Frame {
        size: (u32, u32),
        magenta: u32,
        background: u32,
    }

    fn is_border(f: &CapturedFrame, x: u32, y: u32) -> bool {
        near(f.rgb(x, y), BORDER)
    }

    /// Finds the window by its border (the only green on the output) and counts the pixels inside
    /// it that are not the app's content: every pixel of a 24 px band along the right and bottom
    /// edges, where a resize exposes first (one drag step moves the edge 2 to 4 px), and an 8 px
    /// grid over the rest.
    fn analyze(f: &CapturedFrame) -> Option<Frame> {
        // Coarse box from every 32nd row and column (the border is 2 px, a sparse grid misses it),
        // then exact edges along the row and column through its middle.
        let (mut cx0, mut cy0, mut cx1, mut cy1) = (u32::MAX, u32::MAX, 0, 0);
        let mut hit = |x: u32, y: u32| (cx0, cy0, cx1, cy1) = (cx0.min(x), cy0.min(y), cx1.max(x), cy1.max(y));
        for y in (0..f.height).step_by(32) {
            for x in (0..f.width).filter(|&x| is_border(f, x, y)) {
                hit(x, y);
            }
        }
        for x in (0..f.width).step_by(32) {
            for y in (0..f.height).filter(|&y| is_border(f, x, y)) {
                hit(x, y);
            }
        }
        if cx0 >= cx1 || cy0 >= cy1 {
            return None;
        }
        // Three rows and three columns, so a cursor over one of them cannot hide an edge.
        let thirds = |a: u32, b: u32| [a + (b - a) / 4, a + (b - a) / 2, a + (b - a) * 3 / 4];
        let rows = thirds(cy0, cy1);
        let cols = thirds(cx0, cx1);
        let x0 = rows.iter().filter_map(|&y| (0..f.width).find(|&x| is_border(f, x, y))).min()?;
        let x1 = rows.iter().filter_map(|&y| (0..f.width).rev().find(|&x| is_border(f, x, y))).max()?;
        let y0 = cols.iter().filter_map(|&x| (0..f.height).find(|&y| is_border(f, x, y))).min()?;
        let y1 = cols.iter().filter_map(|&x| (0..f.height).rev().find(|&y| is_border(f, x, y))).max()?;
        // The window's content box: inside the border on every side.
        let (ix0, iy0, ix1, iy1) = (x0 + BORDER_SIZE, y0 + BORDER_SIZE, x1 + 1 - BORDER_SIZE, y1 + 1 - BORDER_SIZE);
        if ix1 <= ix0 + 100 || iy1 <= iy0 + 100 {
            return None;
        }
        let mut frame = Frame { size: (ix1 - ix0, iy1 - iy0), ..Default::default() };
        let mut look = |x: u32, y: u32| {
            let c = f.rgb(x, y);
            frame.magenta += u32::from(near(c, [0xff, 0x00, 0xff]));
            frame.background += u32::from(near(c, BACKGROUND));
        };
        let band = 24;
        for y in iy0..iy1 {
            for x in ix1 - band..ix1 {
                look(x, y);
            }
        }
        for y in iy1 - band..iy1 {
            for x in ix0..ix1 - band {
                look(x, y);
            }
        }
        for y in (iy0..iy1 - band).step_by(8) {
            for x in (ix0..ix1 - band).step_by(8) {
                look(x, y);
            }
        }
        Some(frame)
    }

    #[derive(Default)]
    struct Recording {
        phase: Option<&'static str>,
        frames: Vec<(&'static str, Frame)>,
        keep: Vec<(&'static str, usize, PathBuf)>,
        /// Where to keep the first frame that shows magenta, background, or no window.
        offending: Option<PathBuf>,
        /// Phases of frames in which no window was found.
        lost: Vec<&'static str>,
    }

    fn record(hyprland: &Hyprland, recording: Arc<Mutex<Recording>>) -> Capture {
        Capture::start(hyprland, move |captured| {
            let mut r = recording.lock().unwrap();
            let Some(phase) = r.phase else { return };
            let Some(frame) = analyze(captured) else {
                if let Some(dir) = &r.offending {
                    let _ = captured.write_bmp(&dir.join(format!("lost-{}.bmp", r.lost.len())));
                }
                r.lost.push(phase);
                return;
            };
            let n = r.frames.iter().filter(|(p, _)| *p == phase).count();
            if let Some(i) = r.keep.iter().position(|(p, at, _)| *p == phase && *at == n) {
                let (.., path) = r.keep.remove(i);
                let _ = captured.write_bmp(&path);
            }
            if (frame.magenta > 0 || frame.background > 0)
                && let Some(dir) = &r.offending
            {
                let n = r.frames.len();
                let _ =
                    captured.write_bmp(&dir.join(format!("{phase}-{n}-m{}-b{}.bmp", frame.magenta, frame.background)));
            }
            r.frames.push((phase, frame));
        })
        .unwrap_or_else(|e| panic!("capture: {e}"))
    }

    fn set_phase(recording: &Mutex<Recording>, phase: Option<&'static str>) {
        recording.lock().unwrap().phase = phase;
    }

    fn phase(recording: &Mutex<Recording>, name: &str) -> Vec<Frame> {
        recording.lock().unwrap().frames.iter().filter(|(p, _)| *p == name).map(|(_, f)| *f).collect()
    }

    /// Drags the right border out by `grow`, then the bottom border by `grow * 0.6`.
    fn edge_drags(input: &mut VirtualInput, client: &Client, grow: f64) {
        let (x, y) = (f64::from(client.at.0), f64::from(client.at.1));
        let (w, h) = (f64::from(client.size.0), f64::from(client.size.1));
        let b = f64::from(BORDER_SIZE);
        let step = Duration::from_millis(16);
        input.drag((x + w + b / 2., y + h / 2.), grow, 0., 60, step).unwrap();
        input.drag((x + (w + grow) / 2., y + h + b / 2.), 0., grow * 0.6, 60, step).unwrap();
        std::thread::sleep(Duration::from_millis(200));
    }

    fn gate_out() -> Option<PathBuf> {
        let dir = PathBuf::from(std::env::var_os("ND_GATE_OUT")?);
        std::fs::create_dir_all(&dir).ok()?;
        Some(dir)
    }

    #[test]
    fn real_resize_drag() {
        let hyprland = start();
        let (probe, client) = Probe::ready(&hyprland, &[("ND_DEBUG_EXPOSE", "1")]);
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
            Probe::ready(&hyprland, &[("ND_DEBUG_EXPOSE", "1"), ("ND_DEBUG_DEFER_LIVE_LAYOUT", "1")]);
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
        let (probe, _) = Probe::ready(&hyprland, &[]);
        std::thread::sleep(Duration::from_millis(300));
        let line = probe.last("nd: chrome").unwrap_or_default();
        assert!(line.ends_with(HYPRLAND_CHROME), "Hyprland answers server-side, nothing drawn, no insets: {line}");
        assert!(analyze(&screenshot(&hyprland).unwrap()).is_some(), "hello's window is on the output");
    }

    #[test]
    fn real_wayland_app_id_is_the_manifest_id() {
        let hyprland = start();
        let (_probe, client) = Probe::ready(&hyprland, &[]);
        assert_eq!(client.class, "dev.formalsnake.hello", "hello's `[app] id` is the window's class: {client:?}");
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
        let (probe, before) = Probe::ready(&hyprland, &[]);
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
        let (probe, client) = Probe::ready(&hyprland, &[]);
        let mut input = VirtualInput::connect(&hyprland).unwrap();
        input.click(f64::from(client.at.0) + 100., f64::from(client.at.1) + 200.).unwrap();
        input.key(keys::A, &[]).unwrap();
        input.key(keys::S, &[Modifier::Control]).unwrap();
        assert!(probe.wait_for("nd: key a", Duration::from_secs(2)), "plain key:\n{}", probe.log());
        assert!(probe.wait_for("nd: key ctrl-s", Duration::from_secs(2)), "chord:\n{}", probe.log());
    }
}
