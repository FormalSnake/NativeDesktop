//! What the Linux tier 3 tests share: an app launched as a client of a headless compositor with
//! its stderr collected, and the resize gate's frame analysis.

#![allow(dead_code)]

pub mod probe {
    use std::io::{BufRead, BufReader};
    use std::path::PathBuf;
    use std::process::{Child, Command, Stdio};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use ndshell::hyprland::{Client, Hyprland};

    pub struct Probe {
        pub child: Child,
        pub lines: Arc<Mutex<Vec<String>>>,
    }

    impl Probe {
        /// The app as a client of `command`'s compositor, with `ND_DIAG_WINDOW` on and its data
        /// (settings, logs, bundles) in a dir of its own.
        pub fn launch(mut command: Command, env: &[(&str, &str)]) -> Self {
            let data = std::env::temp_dir().join(format!("nd-probe-{}-{:?}", std::process::id(), Instant::now()));
            command
                .env("ND_DIAG_WINDOW", "1")
                .env("ND_APP_DATA_DIR", &data)
                .envs(env.iter().copied())
                .stdout(Stdio::null())
                .stderr(Stdio::piped());
            let mut child = command.spawn().expect("the app starts");
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

        pub fn count(&self, prefix: &str) -> usize {
            self.lines.lock().unwrap().iter().filter(|l| l.starts_with(prefix)).count()
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

        /// `bin` launched, drawn (it printed `marker`), and mapped as a settled window.
        pub fn ready(hyprland: &Hyprland, bin: PathBuf, env: &[(&str, &str)], marker: &str) -> (Self, Client) {
            let probe = Self::launch(hyprland.command(bin), env);
            assert!(
                probe.wait_for(marker, Duration::from_secs(20)),
                "no {marker:?} from the app:\n{}\nHyprland:\n{}",
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
}

/// The resize gate (ARCHITECTURE.md 5.2): every frame the output presents during a border drag,
/// found by the window's border and checked for what shows inside it.
pub mod resize {
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use ndshell::hyprland::{BACKGROUND, BORDER, BORDER_SIZE, Client, Hyprland};
    use ndshell::wayland::{Capture, CapturedFrame, VirtualInput};

    pub fn near(c: [u8; 3], target: [u8; 3]) -> bool {
        c.iter().zip(target).all(|(a, b)| a.abs_diff(b) <= 24)
    }

    /// One analyzed frame: the window's extent inside its border and what shows inside it.
    #[derive(Clone, Copy, Debug, Default)]
    pub struct Frame {
        pub size: (u32, u32),
        pub magenta: u32,
        pub background: u32,
    }

    pub fn is_border(f: &CapturedFrame, x: u32, y: u32) -> bool {
        near(f.rgb(x, y), BORDER)
    }

    /// Finds the window by its border (the only green on the output) and counts the pixels inside
    /// it that are not the app's content: every pixel of a 24 px band along the right and bottom
    /// edges, where a resize exposes first (one drag step moves the edge 2 to 4 px), and an 8 px
    /// grid over the rest.
    pub fn analyze(f: &CapturedFrame) -> Option<Frame> {
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
    pub struct Recording {
        pub phase: Option<&'static str>,
        pub frames: Vec<(&'static str, Frame)>,
        pub keep: Vec<(&'static str, usize, PathBuf)>,
        /// Where to keep the first frame that shows magenta, background, or no window.
        pub offending: Option<PathBuf>,
        /// Phases of frames in which no window was found.
        pub lost: Vec<&'static str>,
    }

    pub fn record(hyprland: &Hyprland, recording: Arc<Mutex<Recording>>) -> Capture {
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

    pub fn set_phase(recording: &Mutex<Recording>, phase: Option<&'static str>) {
        recording.lock().unwrap().phase = phase;
    }

    pub fn phase(recording: &Mutex<Recording>, name: &str) -> Vec<Frame> {
        recording.lock().unwrap().frames.iter().filter(|(p, _)| *p == name).map(|(_, f)| *f).collect()
    }

    /// Drags the right border out by `grow`, then the bottom border by `grow * 0.6`.
    pub fn edge_drags(input: &mut VirtualInput, client: &Client, grow: f64) {
        let (x, y) = (f64::from(client.at.0), f64::from(client.at.1));
        let (w, h) = (f64::from(client.size.0), f64::from(client.size.1));
        let b = f64::from(BORDER_SIZE);
        let step = Duration::from_millis(16);
        input.drag((x + w + b / 2., y + h / 2.), grow, 0., 60, step).unwrap();
        input.drag((x + (w + grow) / 2., y + h + b / 2.), 0., grow * 0.6, 60, step).unwrap();
        std::thread::sleep(Duration::from_millis(200));
    }

    pub fn gate_out() -> Option<PathBuf> {
        let dir = PathBuf::from(std::env::var_os("ND_GATE_OUT")?);
        std::fs::create_dir_all(&dir).ok()?;
        Some(dir)
    }
}
