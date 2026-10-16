//! M2 resize gate, Windows leg (ARCHITECTURE.md 5.2, 5.4 and 16), tier 3. Runs on the Windows
//! machine, from an ssh session or a terminal, and does everything through the `ndshell` agent
//! in the desktop session: it launches `examples/hello` with magenta painted behind its tree and
//! its root opaque (`ND_DEBUG_EXPOSE=1`), records every frame DWM composes for the window with
//! Windows.Graphics.Capture, and drags the window's right then bottom edge with `SendInput`, so
//! Windows runs its real size loop. In every frame the content's background must run to the
//! window's edge as it did before the drag (a swap chain behind the window size leaves a strip of
//! another colour there) and no magenta may show (a late nd layout).
//!
//! Also checked: the caption hit test (`HTMAXBUTTON` on the drawn maximize button, `HTCAPTION`
//! on the title row), the Snap Layouts flyout opening on hover over the maximize button, a
//! maximize and restore through the drawn button, and a pen stroke arriving through `WM_POINTER`
//! with its pressure. A control run with `ND_DEBUG_DEFER_LIVE_LAYOUT=1` must fail the check,
//! which proves the detector sees a late layout.
//!
//! Needs the agent (`ndshell agent install`) and a built `hello` (`cargo build -p hello`;
//! `ND_HELLO_BIN` overrides). Prints `ND_M2_WINDOWS_RESIZE_GATE_OK`.
//! `ND_GATE_OUT=<dir>` keeps a mid-drag frame, the Snap Layouts flyout before and after hover, and
//! `hello` with Mica, as PNG.

fn main() {
    let name = "gate_m2_windows::real_resize_drag";
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--list") {
        if cfg!(windows) && !args.iter().any(|a| a == "--ignored") {
            println!("{name}: test");
        }
        return;
    }
    let filters: Vec<&String> = args.iter().filter(|a| !a.starts_with('-')).collect();
    let selected = filters.is_empty() || filters.iter().any(|f| name.contains(f.as_str()));
    let allowed = std::env::var_os("NEXTEST").is_some() || std::env::var_os("ND_REAL").is_some_and(|v| v == "1");
    if !cfg!(windows) || !selected || args.iter().any(|a| a == "--ignored") {
        return;
    }
    if allowed {
        #[cfg(windows)]
        gate::run();
    } else {
        println!("{name}: skipped, it moves the real cursor; run with nextest --profile real or ND_REAL=1");
    }
}

#[cfg(windows)]
mod gate {
    use std::path::PathBuf;
    use std::time::Duration;

    use ndshell::agent::Client;
    use serde_json::{Value, json};

    const HTCLIENT: i64 = 1;
    const HTCAPTION: i64 = 2;
    const HTMINBUTTON: i64 = 8;
    const HTMAXBUTTON: i64 = 9;
    const HTRIGHT: i64 = 11;
    const HTBOTTOM: i64 = 15;
    const HTCLOSE: i64 = 20;
    const TITLE: f64 = 32.;
    const BUTTON: f64 = 46.;

    struct Report {
        failures: Vec<String>,
    }

    impl Report {
        fn check(&mut self, ok: bool, what: String) {
            println!("{} {what}", if ok { "ok  " } else { "FAIL" });
            if !ok {
                self.failures.push(what);
            }
        }
    }

    fn sleep(ms: u64) {
        std::thread::sleep(Duration::from_millis(ms));
    }

    fn hello_bin() -> PathBuf {
        if let Some(bin) = std::env::var_os("ND_HELLO_BIN") {
            return bin.into();
        }
        // Final binaries sit in the target dir, not beside this test in the build dir.
        let target = std::env::var_os("CARGO_TARGET_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target"));
        target.join("debug").join("hello.exe")
    }

    struct Hello<'a> {
        agent: &'a mut Client,
        pid: i64,
        hwnd: i64,
    }

    #[derive(Clone, Copy, Debug)]
    struct Geometry {
        /// Visible frame, physical pixels.
        frame: [i64; 4],
        /// Client area origin and size, physical pixels.
        client: [i64; 4],
        scale: f64,
        maximized: bool,
    }

    fn quad(v: &Value) -> [i64; 4] {
        [0, 1, 2, 3].map(|i| v[i].as_i64().unwrap_or(0))
    }

    impl<'a> Hello<'a> {
        fn launch(agent: &'a mut Client, defer: bool, expose: bool) -> Result<Self, String> {
            let bin = hello_bin();
            if !bin.exists() {
                return Err(format!("build hello first (cargo build -p hello): {}", bin.display()));
            }
            let reply = agent.call(json!({
                "cmd": "launch",
                "exe": bin.display().to_string(),
                "env": {
                    "ND_DIAG_WINDOW": "1",
                    "ND_DEBUG_EXPOSE": if expose { "1" } else { "0" },
                    "ND_DEBUG_DEFER_LIVE_LAYOUT": if defer { "1" } else { "0" },
                },
            }))?;
            let pid = reply["pid"].as_i64().unwrap();
            let info = agent.call(json!({ "cmd": "window", "pid": pid, "timeout_ms": 15000 }))?;
            let hwnd = info["hwnd"].as_i64().unwrap();
            Ok(Self { agent, pid, hwnd })
        }

        fn geometry(&mut self) -> Geometry {
            let info = self.agent.call(json!({ "cmd": "window_info", "hwnd": self.hwnd })).unwrap();
            Geometry {
                frame: quad(&info["frame"]),
                client: quad(&info["client"]),
                scale: info["dpi"].as_f64().unwrap_or(96.) / 96.,
                maximized: info["maximized"] == true,
            }
        }

        fn output(&mut self) -> Vec<String> {
            let reply = self.agent.call(json!({ "cmd": "output", "pid": self.pid })).unwrap_or_default();
            reply["lines"]
                .as_array()
                .map(|l| l.iter().filter_map(|s| s.as_str().map(str::to_owned)).collect())
                .unwrap_or_default()
        }

        /// The hit test at a screen point. The cursor goes there first: GPUI answers for its own
        /// drag regions from the last mouse move it saw, not from the point asked about.
        fn hit(&mut self, x: i64, y: i64) -> i64 {
            let _ = self.agent.call(json!({ "cmd": "move", "to": [x, y] }));
            sleep(60);
            let reply =
                self.agent.call(json!({ "cmd": "hit_test", "hwnd": self.hwnd, "at": [x, y] })).unwrap_or_default();
            reply["code"].as_i64().unwrap_or(-1)
        }

        fn call(&mut self, request: Value) -> Result<Value, String> {
            self.agent.call(request)
        }

        /// Screen point of the centre of a caption button, counted from the right (0 = close).
        fn button_center(g: &Geometry, from_right: f64) -> (i64, i64) {
            let x = g.client[0] + g.client[2] - ((from_right + 0.5) * BUTTON * g.scale) as i64;
            let y = g.client[1] + (TITLE / 2. * g.scale) as i64;
            (x, y)
        }
    }

    impl Drop for Hello<'_> {
        fn drop(&mut self) {
            let _ = self.agent.call(json!({ "cmd": "kill", "pid": self.pid }));
        }
    }

    struct EdgeRun {
        frames: usize,
        sizes: usize,
        bad: usize,
        baseline: String,
        grew: (i64, i64),
    }

    /// Records while dragging the right edge out by `dx` and the bottom edge by `dy` (physical px).
    fn edge_drags(hello: &mut Hello, dx: i64, dy: i64, keep: Option<PathBuf>, r: &mut Report) -> EdgeRun {
        let g = hello.geometry();
        let [fx, fy, fw, fh] = g.frame;
        let mut request = json!({
            "cmd": "record_start",
            "hwnd": hello.hwnd,
            "probe": { "tolerance": 6, "scan": (24. * g.scale) as i64, "skip": (TITLE * 2.5 * g.scale) as i64 },
        });
        if let Some(keep) = keep {
            request["keep"] = json!([[20, keep.display().to_string()]]);
        }
        hello.call(request).expect("recording starts");
        sleep(300);

        // The resize zone is the invisible border just outside the visible frame.
        let mut right = (fx + fw + 3, fy + fh / 2);
        if hello.hit(right.0, right.1) != HTRIGHT {
            right = (fx + fw - 1, fy + fh / 2);
        }
        r.check(hello.hit(right.0, right.1) == HTRIGHT, format!("HTRIGHT at the right edge {right:?}"));
        hello
            .call(json!({ "cmd": "drag", "from": [right.0, right.1], "to": [right.0 + dx, right.1], "steps": 60, "ms": 900 }))
            .expect("right edge drag");
        sleep(250);
        let g2 = hello.geometry();
        let mut bottom = (g2.frame[0] + g2.frame[2] / 2, g2.frame[1] + g2.frame[3] + 3);
        if hello.hit(bottom.0, bottom.1) != HTBOTTOM {
            bottom = (bottom.0, g2.frame[1] + g2.frame[3] - 1);
        }
        r.check(hello.hit(bottom.0, bottom.1) == HTBOTTOM, format!("HTBOTTOM at the bottom edge {bottom:?}"));
        hello
            .call(json!({ "cmd": "drag", "from": [bottom.0, bottom.1], "to": [bottom.0, bottom.1 + dy], "steps": 40, "ms": 700 }))
            .expect("bottom edge drag");
        sleep(400);
        let reply = hello.call(json!({ "cmd": "record_stop" })).expect("recording stops");
        let g3 = hello.geometry();
        let frames = reply["frames"].as_array().cloned().unwrap_or_default();
        let edge = |f: &Value, key: &str| -> Option<(i64, i64)> { Some((f[key][0].as_i64()?, f[key][1].as_i64()?)) };
        let first = frames.first();
        let base = first.map(|f| (edge(f, "right"), edge(f, "bottom")));
        let off: Vec<&Value> = frames
            .iter()
            .filter(|f| {
                let (right, bottom) = (edge(f, "right"), edge(f, "bottom"));
                let flat = |e: Option<(i64, i64)>| e.is_some_and(|(lo, hi)| lo == hi);
                let exposed = f["magenta"].as_i64().unwrap_or(0) > 0;
                exposed || !(flat(right) && flat(bottom) && Some((right, bottom)) == base)
            })
            .collect();
        for f in off.iter().take(4) {
            println!("     off: {f}");
        }
        let bad = off.len();
        let mut sizes: Vec<(i64, i64)> =
            frames.iter().map(|f| (f["w"].as_i64().unwrap_or(0), f["h"].as_i64().unwrap_or(0))).collect();
        sizes.sort_unstable();
        sizes.dedup();
        EdgeRun {
            frames: frames.len(),
            sizes: sizes.len(),
            bad,
            baseline: format!("{base:?}"),
            grew: (g3.frame[2] - fw, g3.frame[3] - fh),
        }
    }

    pub fn run() {
        let out = std::env::var_os("ND_GATE_OUT").map(PathBuf::from);
        if let Some(out) = &out {
            let _ = std::fs::create_dir_all(out);
        }
        let _lock = ndshell::lock::acquire("gate_m2_windows::real_resize_drag").expect("real-input lock");
        let mut agent = match Client::connect() {
            Ok(agent) => agent,
            Err(e) => {
                println!("ND_M2_WINDOWS_RESIZE_GATE_FAILED agent: {e}");
                std::process::exit(1);
            }
        };
        agent.set_timeout(Some(Duration::from_secs(30)));
        let ping = agent.call(json!({ "cmd": "ping" })).expect("agent answers");
        println!("     agent: per-monitor v2 {}, monitors {}", ping["per_monitor_v2"], ping["monitors"]);
        let mut r = Report { failures: Vec::new() };

        let mut hello = match Hello::launch(&mut agent, false, true) {
            Ok(h) => h,
            Err(e) => {
                println!("ND_M2_WINDOWS_RESIZE_GATE_FAILED launch: {e}");
                std::process::exit(1);
            }
        };
        sleep(800);
        let g = hello.geometry();
        println!("     window {:?}, client {:?}, scale {}", g.frame, g.client, g.scale);
        let title = (g.client[0] + g.client[2] / 2, g.client[1] + (TITLE / 2. * g.scale) as i64);
        // A click on the caption activates without moving.
        hello.call(json!({ "cmd": "click", "at": [title.0, title.1] })).expect("click title row");
        sleep(400);

        let max = Hello::button_center(&g, 1.);
        let close = Hello::button_center(&g, 0.);
        let min = Hello::button_center(&g, 2.);
        let content = (g.client[0] + g.client[2] / 2, g.client[1] + g.client[3] / 2);
        r.check(hello.hit(max.0, max.1) == HTMAXBUTTON, format!("HTMAXBUTTON on the drawn maximize button {max:?}"));
        r.check(hello.hit(close.0, close.1) == HTCLOSE, format!("HTCLOSE on the drawn close button {close:?}"));
        r.check(hello.hit(min.0, min.1) == HTMINBUTTON, format!("HTMINBUTTON on the drawn minimize button {min:?}"));
        r.check(hello.hit(title.0, title.1) == HTCAPTION, format!("HTCAPTION on the title row {title:?}"));
        r.check(hello.hit(content.0, content.1) == HTCLIENT, format!("HTCLIENT in the content {content:?}"));

        // Real-drag resize.
        let keep = out.as_ref().map(|o| o.join("mid-drag.png"));
        let run = edge_drags(&mut hello, (240. * g.scale) as i64, (144. * g.scale) as i64, keep, &mut r);
        r.check(
            run.grew.0 >= (200. * g.scale) as i64 && run.grew.1 >= (110. * g.scale) as i64,
            format!("the drags resized the window by {:?} px", run.grew),
        );
        r.check(
            run.frames >= 30 && run.sizes >= 10,
            format!("Windows.Graphics.Capture recorded {} frames at {} distinct window sizes", run.frames, run.sizes),
        );
        r.check(
            run.bad == 0,
            format!(
                "content edge at the window edge in every frame ({} of {} off, baseline {})",
                run.bad, run.frames, run.baseline
            ),
        );

        // Snap Layouts: hovering the drawn maximize button opens the system flyout below it.
        let g = hello.geometry();
        let max = Hello::button_center(&g, 1.);
        let region = [
            max.0 - (320. * g.scale) as i64,
            max.1 - (16. * g.scale) as i64,
            (480. * g.scale) as i64,
            (260. * g.scale) as i64,
        ];
        hello.call(json!({ "cmd": "move", "to": [content.0, content.1 + (60. * g.scale) as i64] })).unwrap();
        sleep(800);
        let mut before = json!({ "cmd": "capture", "monitor_at": [max.0, max.1], "crop": region });
        let mut after = before.clone();
        if let Some(out) = &out {
            before["out"] = json!(out.join("snap-before.png").display().to_string());
            after["out"] = json!(out.join("snap-layouts.png").display().to_string());
        }
        let before = hello.call(before).expect("capture before hover");
        hello.call(json!({ "cmd": "move", "to": [max.0 - 4, max.1 + 2] })).unwrap();
        sleep(250);
        hello.call(json!({ "cmd": "move", "to": [max.0, max.1] })).unwrap();
        sleep(1800);
        let after = hello.call(after).expect("capture on hover");
        let diff = grid_diff(&before["grid"], &after["grid"]);
        if snap_flyout_enabled() {
            r.check(
                diff >= 6,
                format!(
                    "hover over the maximize button opens the Snap Layouts flyout ({diff} of 64 cells below it changed)"
                ),
            );
        } else {
            println!("skip Snap Layouts flyout: off in Settings on this machine (EnableSnapAssistFlyout = 0)");
        }
        hello.call(json!({ "cmd": "move", "to": [content.0, content.1] })).unwrap();
        sleep(800);

        // Maximize and restore through the drawn button.
        hello.call(json!({ "cmd": "click", "at": [max.0, max.1] })).unwrap();
        sleep(900);
        let maxed = hello.geometry();
        r.check(maxed.maximized, "a click on the drawn maximize button maximizes".into());
        let restore = Hello::button_center(&maxed, 1.);
        r.check(
            hello.hit(restore.0, restore.1) == HTMAXBUTTON,
            format!("HTMAXBUTTON on the restore button at {restore:?}"),
        );
        hello.call(json!({ "cmd": "click", "at": [restore.0, restore.1] })).unwrap();
        sleep(900);
        r.check(!hello.geometry().maximized, "a second click restores".into());

        // The window menu, opened by the chrome since the window has no WS_SYSMENU: a right-click
        // on the title row, Alt+Space, and the taskbar button's menu (WM_POPUPSYSTEMMENU).
        let g = hello.geometry();
        let title = (g.client[0] + g.client[2] / 2, g.client[1] + (TITLE / 2. * g.scale) as i64);
        let menu_open = |hello: &mut Hello| {
            hello.call(json!({ "cmd": "popup_menu" })).ok().and_then(|r| r["open"].as_bool()).unwrap_or(false)
        };
        let lparam = (title.1 << 16) | (title.0 & 0xffff);
        let opens: [(&str, Value); 3] = [
            ("a right-click on the title row", json!({ "cmd": "click", "at": [title.0, title.1], "button": "right" })),
            ("Alt+Space", json!({ "cmd": "key", "vk": 0x20, "mods": [0x12] })),
            (
                "the taskbar button's menu",
                json!({ "cmd": "post", "hwnd": hello.hwnd, "msg": 0x0313, "lparam": lparam }),
            ),
        ];
        for (how, open) in opens {
            hello.call(json!({ "cmd": "click", "at": [content.0, content.1] })).unwrap();
            sleep(300);
            hello.call(open).unwrap();
            sleep(600);
            r.check(menu_open(&mut hello), format!("{how} opens the window menu"));
            hello.call(json!({ "cmd": "key", "vk": 0x1b })).unwrap();
            sleep(300);
        }
        r.check(!menu_open(&mut hello), "Esc closes the window menu".into());

        // Pen through WM_POINTER.
        let g = hello.geometry();
        let (cx, cy) = (g.client[0] + g.client[2] / 4, g.client[1] + g.client[3] * 3 / 4);
        let stroke: Vec<Value> = (0..12).map(|i| json!([cx + i * 6, cy, 0.5])).collect();
        let pen = hello.call(json!({ "cmd": "pen", "points": stroke }));
        sleep(300);
        let lines = hello.output();
        let pen_lines: Vec<&String> = lines.iter().filter(|l| l.starts_with("nd: pen")).collect();
        r.check(
            pen.is_ok() && pen_lines.iter().any(|l| l.contains("Down pressure=0.50")),
            format!("a synthetic pen stroke reaches WM_POINTER with pressure ({:?}; {pen_lines:?})", pen.err()),
        );
        drop(hello);

        if let Some(out) = &out {
            let mut plain = Hello::launch(&mut agent, false, false).expect("hello starts");
            sleep(800);
            let g = plain.geometry();
            plain.call(json!({ "cmd": "click", "at": [g.client[0] + g.client[2] / 2, g.client[1] + (16. * g.scale) as i64] })).unwrap();
            plain.call(json!({ "cmd": "move", "to": [g.client[0] + g.client[2] / 2, g.client[1] + g.client[3] + (80. * g.scale) as i64] })).unwrap();
            sleep(800);
            let shot = plain.call(json!({ "cmd": "capture", "hwnd": plain.hwnd, "out": out.join("hello-mica.png").display().to_string() }));
            r.check(shot.is_ok(), format!("hello with Mica captured ({:?})", shot.err()));
        }

        if std::env::var_os("ND_GATE_SKIP_CONTROL").is_none() {
            let mut control = Hello::launch(&mut agent, true, true).expect("control hello starts");
            sleep(800);
            let g = control.geometry();
            control.call(json!({ "cmd": "click", "at": [g.client[0] + g.client[2] / 2, g.client[1] + (16. * g.scale) as i64] })).unwrap();
            sleep(400);
            let mut scratch = Report { failures: Vec::new() };
            let run = edge_drags(&mut control, (160. * g.scale) as i64, (96. * g.scale) as i64, None, &mut scratch);
            r.check(
                run.bad > 0,
                format!("control: with layout deferred, {} of {} frames are caught off the edge", run.bad, run.frames),
            );
        }

        if r.failures.is_empty() {
            println!("ND_M2_WINDOWS_RESIZE_GATE_OK");
            std::process::exit(0);
        }
        println!("ND_M2_WINDOWS_RESIZE_GATE_FAILED {} check(s)", r.failures.len());
        std::process::exit(1);
    }

    /// How many cells of two 8 by 8 capture grids differ by more than 24 levels in a channel.
    /// Settings > System > Multitasking > "Show snap layouts when I hover over a window's maximize
    /// button". Off, the flyout never opens for any window.
    fn snap_flyout_enabled() -> bool {
        let key = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced";
        let out = std::process::Command::new("reg").args(["query", key, "/v", "EnableSnapAssistFlyout"]).output();
        !out.is_ok_and(|o| String::from_utf8_lossy(&o.stdout).split_whitespace().last() == Some("0x0"))
    }

    fn grid_diff(a: &Value, b: &Value) -> usize {
        let (Some(a), Some(b)) = (a.as_array(), b.as_array()) else { return 0 };
        a.iter()
            .zip(b)
            .filter(|(ca, cb)| {
                (0..3).any(|i| (ca[i].as_f64().unwrap_or(0.) - cb[i].as_f64().unwrap_or(0.)).abs() > 24.)
            })
            .count()
    }
}
