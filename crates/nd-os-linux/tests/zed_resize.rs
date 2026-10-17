//! Interactive resize of a plain GPUI window built from a zed checkout, on a headless Hyprland and
//! a headless labwc. This measures GPUI itself for the upstream PRs (docs/plan/upstream-gpui.md),
//! not nd: `ZED_PROBE_BIN` is a GPUI example that fills its window with [`CONTENT`] and prints
//! `zed-probe: frame <w>x<h>` to stderr on every render. The virtual pointer drags the right and
//! then the bottom border, 60 steps each 16 ms apart, while every presented frame is captured.
//!
//! Per compositor it prints how many frames the app rendered during the drag (the frame rate it
//! kept up), and how many captured frames showed the compositor background or anything other than
//! the app's colour in a 24 px band along the right and bottom edges of the window.
//!
//!   ZED_PROBE_BIN=<probe> ZED_LABEL=<name> cargo test -p nd-os-linux --test zed_resize -- --ignored --nocapture
//!
//! The probe's Vulkan driver is whatever `VK_ICD_FILENAMES` says (the dev shell's lavapipe for a
//! software GPU, mesa's `intel_icd` for the iGPU).

#![cfg(target_os = "linux")]

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ndshell::wayland::{Capture, CapturedFrame, VirtualInput, screenshot_socket};

const CONTENT: [u8; 3] = [0xff, 0x80, 0x00];
const BORDER: [u8; 3] = [0x00, 0xff, 0x00];
const BAND: u32 = 24;

fn near(c: [u8; 3], target: [u8; 3]) -> bool {
    c.iter().zip(target).all(|(a, b)| a.abs_diff(b) <= 24)
}

/// The window's outer border box: the extent of the green pixels.
fn border_box(f: &CapturedFrame) -> Option<(u32, u32, u32, u32)> {
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
    for y in (0..f.height).step_by(2) {
        for x in (0..f.width).step_by(2) {
            if near(f.rgb(x, y), BORDER) {
                (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
            }
        }
    }
    // Exact edges along the middle row and column.
    let (my, mx) = ((y0 + y1) / 2, (x0 + x1) / 2);
    if x0 >= x1 || y0 >= y1 {
        return None;
    }
    let x0 = (0..f.width).find(|&x| near(f.rgb(x, my), BORDER))?;
    let x1 = (0..f.width).rev().find(|&x| near(f.rgb(x, my), BORDER))?;
    let y0 = (0..f.height).find(|&y| near(f.rgb(mx, y), BORDER))?;
    let y1 = (0..f.height).rev().find(|&y| near(f.rgb(mx, y), BORDER))?;
    Some((x0, y0, x1, y1))
}

/// The content box inside the border and below any title bar (green rows under the top border).
fn content_box(f: &CapturedFrame) -> Option<(u32, u32, u32, u32)> {
    let (x0, y0, x1, y1) = border_box(f)?;
    let mx = (x0 + x1) / 2;
    let top = (y0..y1).find(|&y| !near(f.rgb(mx, y), BORDER))?;
    let (ix0, iy0, ix1, iy1) = (x0 + 2, top, x1 - 1, y1 - 1);
    (ix1 > ix0 + 2 * BAND && iy1 > iy0 + 2 * BAND).then_some((ix0, iy0, ix1, iy1))
}

#[derive(Debug, Default, Clone, Copy)]
struct Seen {
    frames: u32,
    background: u32,
    foreign: u32,
    lost: u32,
}

fn analyze(f: &CapturedFrame, background: [u8; 3], seen: &mut Seen) {
    let Some((ix0, iy0, ix1, iy1)) = content_box(f) else {
        seen.lost += 1;
        return;
    };
    seen.frames += 1;
    let (mut bg, mut other) = (false, false);
    let mut look = |x: u32, y: u32| {
        let c = f.rgb(x, y);
        if near(c, background) {
            bg = true;
        } else if !near(c, CONTENT) {
            other = true;
        }
    };
    for y in iy0..iy1 {
        for x in ix1 - BAND..ix1 {
            look(x, y);
        }
    }
    for y in iy1 - BAND..iy1 {
        for x in ix0..ix1 - BAND {
            look(x, y);
        }
    }
    seen.background += u32::from(bg);
    seen.foreign += u32::from(other && !bg);
}

struct Probe {
    child: Child,
    lines: Arc<Mutex<Vec<(Instant, String)>>>,
}

impl Probe {
    fn launch(mut command: Command) -> Self {
        command.stdout(Stdio::null()).stderr(Stdio::piped());
        let mut child = command.spawn().expect("the probe starts");
        let lines = Arc::new(Mutex::new(Vec::new()));
        let sink = lines.clone();
        let stderr = child.stderr.take().unwrap();
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                sink.lock().unwrap().push((Instant::now(), line));
            }
        });
        Self { child, lines }
    }

    fn frames_between(&self, from: Instant, to: Instant) -> usize {
        let lines = self.lines.lock().unwrap();
        lines.iter().filter(|(at, l)| *at >= from && *at <= to && l.starts_with("zed-probe: frame")).count()
    }

    fn wait_frame(&self) -> bool {
        let end = Instant::now() + Duration::from_secs(20);
        while Instant::now() < end {
            if self.lines.lock().unwrap().iter().any(|(_, l)| l.starts_with("zed-probe: frame")) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    fn log(&self) -> String {
        let lines = self.lines.lock().unwrap();
        lines.iter().rev().take(20).map(|(_, l)| l.as_str()).collect::<Vec<_>>().join("\n")
    }
}

impl Drop for Probe {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Drags the right border out by 240 px and the bottom border by 144 px, capturing throughout.
fn drag_and_measure(socket: &std::path::Path, probe: &Probe, compositor: &str) {
    let still = screenshot_socket(socket).unwrap();
    let background = still.rgb(3, 3);
    let (x0, y0, x1, y1) = border_box(&still).unwrap_or_else(|| panic!("{compositor}: no window on the output"));
    let mut input = VirtualInput::connect_socket(socket, (still.width, still.height)).unwrap();
    let seen = Arc::new(Mutex::new(Seen::default()));
    let sink = seen.clone();
    let capture = Capture::start_socket(socket, move |f| analyze(f, background, &mut sink.lock().unwrap())).unwrap();
    let step = Duration::from_millis(16);
    let start = Instant::now();
    let (x0, y0, x1, y1) = (f64::from(x0), f64::from(y0), f64::from(x1), f64::from(y1));
    input.drag((x1, (y0 + y1) / 2.), 240., 0., 60, step).unwrap();
    input.drag(((x0 + x1 + 240.) / 2., y1), 0., 144., 60, step).unwrap();
    let end = Instant::now();
    std::thread::sleep(Duration::from_millis(300));
    capture.stop().unwrap();
    let after = screenshot_socket(socket).unwrap();
    let grown = border_box(&after).map(|(a, b, c, d)| (c - a, d - b));
    let rendered = probe.frames_between(start, end);
    let seconds = (end - start).as_secs_f64();
    let seen = *seen.lock().unwrap();
    let label = std::env::var("ZED_LABEL").unwrap_or_default();
    println!(
        "zed-resize {label} {compositor}: {rendered} frames rendered in {seconds:.2} s ({:.0} Hz) for 120 steps; \
         {} frames captured, {} with background, {} with other pixels at the edges, {} without a window; \
         window {}x{} -> {:?}",
        rendered as f64 / seconds,
        seen.frames,
        seen.background,
        seen.foreign,
        seen.lost,
        x1 - x0,
        y1 - y0,
        grown,
    );
}

fn probe_bin() -> String {
    std::env::var("ZED_PROBE_BIN").expect("ZED_PROBE_BIN: a GPUI probe built from a zed checkout")
}

#[test]
#[ignore = "needs ZED_PROBE_BIN, Hyprland and labwc"]
fn zed_resize_hyprland() {
    let hyprland = ndshell::hyprland::Hyprland::start(ndshell::hyprland::Options::default()).unwrap();
    let probe = Probe::launch(hyprland.command(probe_bin()));
    assert!(probe.wait_frame(), "probe never drew:\n{}\n{}", probe.log(), hyprland.log_tail());
    hyprland.wait_for_client(probe.child.id(), Duration::from_secs(5)).unwrap();
    std::thread::sleep(Duration::from_millis(500));
    drag_and_measure(&hyprland.wayland_socket(), &probe, "hyprland");
}

#[test]
#[ignore = "needs ZED_PROBE_BIN, Hyprland and labwc"]
fn zed_resize_labwc() {
    let labwc = ndshell::labwc::Labwc::start().unwrap();
    let probe = Probe::launch(labwc.command(probe_bin()));
    assert!(probe.wait_frame(), "probe never drew:\n{}\n{}", probe.log(), labwc.log_tail());
    std::thread::sleep(Duration::from_millis(800));
    drag_and_measure(&labwc.wayland_socket(), &probe, "labwc");
}

/// X11 pacing per monitor: Xwayland on a headless Hyprland whose first output runs at 144 Hz and
/// second at 240 Hz. The probe renders every frame, so its frame rate is the slower of the rate GPUI
/// paces it at and the rate the output presents at (Vulkan FIFO). The slower output comes first,
/// so a window paced by the first CRTC shows on the faster one. Prints the rate on each output,
/// moving the focused window between the workspaces the outputs show.
#[test]
#[ignore = "needs ZED_PROBE_BIN and Hyprland with Xwayland"]
fn zed_x11_refresh() {
    let x11_sockets = || -> std::collections::BTreeSet<String> {
        std::fs::read_dir("/tmp/.X11-unix")
            .map(|d| d.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect())
            .unwrap_or_default()
    };
    let before = x11_sockets();
    let options = ndshell::hyprland::Options {
        refresh: 144,
        extra_config: r#"
hl.monitor({ output = "HEADLESS-2", mode = "1280x800@240", position = "1600x0", scale = "1" })
hl.config({ xwayland = { enabled = true } })
"#
        .into(),
        ..Default::default()
    };
    let hyprland = ndshell::hyprland::Hyprland::start(options).unwrap();
    let created = hyprland.request("output create headless HEADLESS-2").unwrap();
    assert_eq!(created.trim(), "ok", "second output: {created}");
    let end = Instant::now() + Duration::from_secs(10);
    let display = loop {
        if let Some(new) = x11_sockets().difference(&before).next() {
            break format!(":{}", new.trim_start_matches('X'));
        }
        assert!(Instant::now() < end, "Xwayland never started: {}", hyprland.log_tail());
        std::thread::sleep(Duration::from_millis(100));
    };
    std::thread::sleep(Duration::from_millis(500));
    println!(
        "monitors: {}",
        hyprland
            .request("j/monitors")
            .unwrap()
            .lines()
            .filter(|l| l.contains("refreshRate") || l.contains("\"name\""))
            .collect::<Vec<_>>()
            .join(" ")
    );
    let mut command = hyprland.command(probe_bin());
    command.env_remove("WAYLAND_DISPLAY").env("DISPLAY", &display);
    let probe = Probe::launch(command);
    assert!(probe.wait_frame(), "probe never drew:\n{}\n{}", probe.log(), hyprland.log_tail());
    hyprland.wait_for_client(probe.child.id(), Duration::from_secs(5)).unwrap();
    let label = std::env::var("ZED_LABEL").unwrap_or_default();
    let rate = |at: &str| {
        std::thread::sleep(Duration::from_secs(1));
        let start = Instant::now();
        std::thread::sleep(Duration::from_secs(3));
        let frames = probe.frames_between(start, Instant::now());
        let c = hyprland.wait_for_client(probe.child.id(), Duration::from_secs(1)).unwrap();
        println!("zed-x11 {label}: window at {:?} ({at}): {:.0} fps", c.at, frames as f64 / 3.0);
    };
    rate("first output, 144 Hz");
    let moved = hyprland.request("dispatch hl.dsp.window.move({ workspace = 2 })").unwrap();
    println!("move: {}", moved.trim());
    rate("second output, 240 Hz");
    let back = hyprland.request("dispatch hl.dsp.window.move({ workspace = 1 })").unwrap();
    println!("move back: {}", back.trim());
    rate("first output again");
}
