//! `ndshell agent run`: serves jobs in the desktop session. See the module docs in `agent`.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute};
use windows::Win32::Graphics::Gdi::{
    ClientToScreen, EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITOR_DEFAULTTONEAREST, MONITORINFO,
    MonitorFromPoint,
};
use windows::Win32::System::Console::{FreeConsole, GetConsoleProcessList};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClientRect, GetForegroundWindow, GetWindowTextW, GetWindowThreadProcessId, IsIconic,
    IsWindowVisible, IsZoomed, PostMessageW, SMTO_ABORTIFHUNG, SWP_NOACTIVATE, SWP_NOZORDER, SendMessageTimeoutW,
    SetWindowPos, WM_CLOSE, WM_NCHITTEST,
};
use windows::core::BOOL;

use super::capture::{EdgeProbe, Recording, Target};
use super::input;

type Lines = Arc<Mutex<Vec<String>>>;

struct Launched {
    child: Child,
    lines: Lines,
}

#[derive(Default)]
struct Agent {
    launched: HashMap<u32, Launched>,
    recording: Option<Recording>,
}

fn log_line(text: &str) {
    let path = super::home().join("agent.log");
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{text}");
    }
    eprintln!("{text}");
}

pub fn run() -> Result<(), String> {
    // Task Scheduler gives a console exe its own console window on the desktop; drop it when no
    // one else shares it (a terminal that ran `ndshell agent run` keeps its window).
    let mut sharing = [0u32; 2];
    if unsafe { GetConsoleProcessList(&mut sharing) } == 1 {
        let _ = unsafe { FreeConsole() };
    }
    nd_os_windows::dpi::enable_per_monitor_v2();
    let expected = super::token()?;
    let addr = super::addr();
    let listener = TcpListener::bind(&addr).map_err(|e| format!("bind {addr}: {e} (is another agent running?)"))?;
    log_line(&format!("ndshell agent: listening on {addr}, pid {}", std::process::id()));
    let mut agent = Agent::default();
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        if let Err(e) = serve(&mut agent, stream, &expected) {
            log_line(&format!("ndshell agent: connection ended: {e}"));
        }
    }
    Ok(())
}

fn serve(agent: &mut Agent, stream: TcpStream, expected: &str) -> Result<(), String> {
    stream.set_nodelay(true).ok();
    let mut writer = stream.try_clone().map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(stream);
    let reply = |writer: &mut TcpStream, value: Value| -> Result<(), String> {
        let mut line = value.to_string();
        line.push('\n');
        writer.write_all(line.as_bytes()).map_err(|e| e.to_string())
    };
    let mut line = String::new();
    reader.read_line(&mut line).map_err(|e| e.to_string())?;
    let hello: Value = serde_json::from_str(&line).unwrap_or(Value::Null);
    if !super::token_matches(hello["token"].as_str().unwrap_or(""), expected) {
        let _ = reply(&mut writer, json!({ "ok": false, "error": "bad token" }));
        return Err("bad token".into());
    }
    reply(&mut writer, json!({ "ok": true, "pid": std::process::id() }))?;
    loop {
        line.clear();
        if reader.read_line(&mut line).map_err(|e| e.to_string())? == 0 {
            return Ok(());
        }
        let request: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                reply(&mut writer, json!({ "ok": false, "error": format!("bad json: {e}") }))?;
                continue;
            }
        };
        let answer = match handle(agent, &request) {
            Ok(mut value) => {
                value["ok"] = json!(true);
                value
            }
            Err(error) => json!({ "ok": false, "error": error }),
        };
        reply(&mut writer, answer)?;
    }
}

fn arg_i32(request: &Value, key: &str) -> Result<i32, String> {
    request[key].as_i64().map(|v| v as i32).ok_or_else(|| format!("`{key}` must be an integer"))
}

fn point(request: &Value, key: &str) -> Result<(i32, i32), String> {
    let p = &request[key];
    match (p[0].as_i64(), p[1].as_i64()) {
        (Some(x), Some(y)) => Ok((x as i32, y as i32)),
        _ => Err(format!("`{key}` must be [x, y]")),
    }
}

fn hwnd_arg(request: &Value) -> Result<HWND, String> {
    request["hwnd"].as_i64().map(|h| HWND(h as isize as *mut _)).ok_or_else(|| "`hwnd` must be an integer".into())
}

fn rect_json(r: RECT) -> Value {
    json!([r.left, r.top, r.right - r.left, r.bottom - r.top])
}

fn handle(agent: &mut Agent, request: &Value) -> Result<Value, String> {
    let launched: Vec<u32> = agent.launched.keys().copied().collect();
    let real_input = || input::check_foreground(&launched);
    match request["cmd"].as_str().unwrap_or("") {
        "ping" => Ok(json!({
            "pid": std::process::id(),
            "per_monitor_v2": nd_os_windows::dpi::is_per_monitor_v2(),
            "monitors": monitors(),
            "cursor": input::cursor(),
        })),
        "launch" => launch(agent, request),
        "output" => {
            let pid = arg_i32(request, "pid")? as u32;
            let entry = agent.launched.get_mut(&pid).ok_or("not launched by this agent")?;
            let exited = entry.child.try_wait().ok().flatten().map(|s| s.code());
            Ok(json!({ "lines": *entry.lines.lock().unwrap(), "exited": exited }))
        }
        "kill" => {
            let pid = arg_i32(request, "pid")? as u32;
            if let Some(mut entry) = agent.launched.remove(&pid) {
                let _ = entry.child.kill();
                let _ = entry.child.wait();
            }
            Ok(json!({}))
        }
        "window" => {
            // By the process that owns it, or by its exact title (an app another process started).
            let pid = request["pid"].as_u64().map(|p| p as u32);
            let title = request["title"].as_str().map(str::to_owned);
            if pid.is_none() && title.is_none() {
                return Err("`window` needs `pid` or `title`".into());
            }
            let timeout = Duration::from_millis(request["timeout_ms"].as_u64().unwrap_or(10_000));
            let end = Instant::now() + timeout;
            loop {
                if let Some(hwnd) = top_window(pid, title.as_deref()) {
                    return Ok(window_info(hwnd));
                }
                if Instant::now() > end {
                    return Err(format!("no visible window for pid {pid:?}, title {title:?}"));
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
        "set_frame" => {
            // Moves and sizes a window in screen pixels: onto another monitor, for pacing runs.
            let hwnd = hwnd_arg(request)?;
            let r = &request["rect"];
            let v = |i: usize| r[i].as_i64().map(|v| v as i32).ok_or("`rect` must be [x, y, w, h]");
            unsafe { SetWindowPos(hwnd, None, v(0)?, v(1)?, v(2)?, v(3)?, SWP_NOZORDER | SWP_NOACTIVATE) }
                .map_err(|e| e.to_string())?;
            Ok(window_info(hwnd))
        }
        "close" => {
            // What the user's close does: the app gets WM_CLOSE and quits through its own path.
            let hwnd = hwnd_arg(request)?;
            unsafe { PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)) }.map_err(|e| e.to_string())?;
            Ok(json!({}))
        }
        "window_info" => Ok(window_info(hwnd_arg(request)?)),
        "hit_test" => {
            let hwnd = hwnd_arg(request)?;
            let (x, y) = point(request, "at")?;
            let lparam = LPARAM(((y as u16 as isize) << 16) | (x as u16 as isize));
            let mut result = 0usize;
            unsafe {
                SendMessageTimeoutW(hwnd, WM_NCHITTEST, WPARAM(0), lparam, SMTO_ABORTIFHUNG, 2000, Some(&mut result));
            }
            Ok(json!({ "code": result as i32 }))
        }
        "move" => {
            real_input()?;
            let (x, y) = point(request, "to")?;
            input::move_to(x, y)?;
            Ok(json!({ "cursor": input::cursor() }))
        }
        "click" => {
            real_input()?;
            let (x, y) = point(request, "at")?;
            input::click(x, y)?;
            Ok(json!({}))
        }
        "button" => {
            real_input()?;
            input::button(request["down"].as_bool().unwrap_or(false))?;
            Ok(json!({}))
        }
        "key" => {
            real_input()?;
            let vk = arg_i32(request, "vk")? as u16;
            input::key(vk, true)?;
            input::key(vk, false)?;
            Ok(json!({}))
        }
        "drag" => {
            real_input()?;
            let from = point(request, "from")?;
            let to = point(request, "to")?;
            let steps = request["steps"].as_u64().unwrap_or(40) as u32;
            let ms = request["ms"].as_u64().unwrap_or(600);
            input::drag(from, to, steps, Duration::from_millis(ms))?;
            Ok(json!({ "cursor": input::cursor() }))
        }
        "pen" => {
            real_input()?;
            let points = request["points"]
                .as_array()
                .ok_or("`points` must be [[x, y, pressure], ..]")?
                .iter()
                .map(|p| {
                    (
                        p[0].as_i64().unwrap_or(0) as i32,
                        p[1].as_i64().unwrap_or(0) as i32,
                        p[2].as_f64().unwrap_or(0.5) as f32,
                    )
                })
                .collect::<Vec<_>>();
            input::pen_stroke(&points, Duration::from_millis(request["interval_ms"].as_u64().unwrap_or(8)))?;
            Ok(json!({}))
        }
        "capture" => {
            // `crop` is [x, y, w, h]: window-relative for a window, screen coordinates for a monitor.
            let (target, origin) = if request["hwnd"].is_i64() {
                (Target::Window(hwnd_arg(request)?), (0, 0))
            } else {
                let (x, y) = point(request, "monitor_at")?;
                let monitor = unsafe { MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST) };
                let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
                let _ = unsafe { GetMonitorInfoW(monitor, &mut info) };
                (Target::Monitor(monitor), (info.rcMonitor.left, info.rcMonitor.top))
            };
            let mut image = super::capture::still(&target, Duration::from_secs(5))?;
            if let Some(r) = request["crop"].as_array() {
                let v = |i: usize| r.get(i).and_then(Value::as_i64).unwrap_or(0);
                let x = (v(0) - origin.0 as i64).max(0) as u32;
                let y = (v(1) - origin.1 as i64).max(0) as u32;
                image = image.crop(x, y, v(2).max(0) as u32, v(3).max(0) as u32);
            }
            if let Some(out) = request["out"].as_str() {
                image.save_png(&PathBuf::from(out))?;
            }
            Ok(json!({ "size": [image.width, image.height], "grid": image.grid(8) }))
        }
        "record_start" => {
            let hwnd = hwnd_arg(request)?;
            let probe = &request["probe"];
            let key = probe["key"]
                .as_array()
                .map(|key| [0, 1, 2].map(|i| key.get(i).and_then(Value::as_u64).unwrap_or(0) as u8));
            let probe = EdgeProbe {
                key,
                tolerance: probe["tolerance"].as_u64().unwrap_or(24) as u8,
                scan: probe["scan"].as_u64().unwrap_or(64) as u32,
                skip: probe["skip"].as_u64().unwrap_or(48) as u32,
            };
            let keep = request["keep"]
                .as_array()
                .map(|k| {
                    k.iter().filter_map(|e| Some((e[0].as_u64()? as usize, PathBuf::from(e[1].as_str()?)))).collect()
                })
                .unwrap_or_default();
            agent.recording = Some(Recording::start(hwnd, probe, keep)?);
            Ok(json!({}))
        }
        "record_count" => Ok(json!({ "frames": agent.recording.as_ref().map_or(0, Recording::count) })),
        "record_stop" => {
            let recording = agent.recording.take().ok_or("no recording running")?;
            let (frames, errors) = recording.stop();
            let frames: Vec<Value> = frames
                .iter()
                .map(|f| json!({ "t": f.t_ms, "w": f.width, "h": f.height, "right": f.right, "bottom": f.bottom, "magenta": f.magenta }))
                .collect();
            Ok(json!({ "frames": frames, "errors": errors }))
        }
        other => Err(format!("unknown cmd `{other}`")),
    }
}

fn launch(agent: &mut Agent, request: &Value) -> Result<Value, String> {
    let exe = request["exe"].as_str().ok_or("`exe` must be a path")?;
    let mut command = Command::new(exe);
    if let Some(args) = request["args"].as_array() {
        command.args(args.iter().filter_map(Value::as_str));
    }
    if let Some(env) = request["env"].as_object() {
        for (k, v) in env {
            command.env(k, v.as_str().unwrap_or(""));
        }
    }
    if let Some(cwd) = request["cwd"].as_str() {
        command.current_dir(cwd);
    }
    // CREATE_NO_WINDOW: a console-subsystem app started from the agent, which has no console,
    // would otherwise open one on the desktop.
    command.creation_flags(0x0800_0000);
    command.stdout(Stdio::piped()).stderr(Stdio::piped()).stdin(Stdio::null());
    let mut child = command.spawn().map_err(|e| format!("{exe}: {e}"))?;
    let lines: Lines = Arc::new(Mutex::new(Vec::new()));
    for pipe in [
        child.stdout.take().map(|p| Box::new(p) as Box<dyn Read + Send>),
        child.stderr.take().map(|p| Box::new(p) as _),
    ]
    .into_iter()
    .flatten()
    {
        let sink = lines.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(pipe).lines().map_while(Result::ok) {
                let mut lines = sink.lock().unwrap();
                if lines.len() >= 5000 {
                    lines.remove(0);
                }
                lines.push(line);
            }
        });
    }
    let pid = child.id();
    agent.launched.insert(pid, Launched { child, lines });
    Ok(json!({ "pid": pid }))
}

/// The largest visible top-level window `pid` owns, or titled `title`.
fn top_window(pid: Option<u32>, title: Option<&str>) -> Option<HWND> {
    struct Search<'a> {
        pid: Option<u32>,
        title: Option<&'a str>,
        found: Vec<HWND>,
    }
    unsafe extern "system" fn each(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let search = unsafe { &mut *(lparam.0 as *mut Search) };
        if !unsafe { IsWindowVisible(hwnd) }.as_bool() {
            return BOOL::from(true);
        }
        let mut owner = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut owner)) };
        let pid_ok = search.pid.is_none_or(|p| p == owner);
        let title_ok = search.title.is_none_or(|t| {
            let mut buf = [0u16; 256];
            let n = unsafe { GetWindowTextW(hwnd, &mut buf) }.max(0) as usize;
            String::from_utf16_lossy(&buf[..n]) == t
        });
        if pid_ok && title_ok {
            search.found.push(hwnd);
        }
        BOOL::from(true)
    }
    let mut search = Search { pid, title, found: Vec::new() };
    let _ = unsafe { EnumWindows(Some(each), LPARAM(&raw mut search as isize)) };
    search.found.into_iter().max_by_key(|&hwnd| {
        let r = frame(hwnd);
        (r.right - r.left) as i64 * (r.bottom - r.top) as i64
    })
}

/// The visible frame (what the user sees, without the invisible resize borders).
fn frame(hwnd: HWND) -> RECT {
    let mut r = RECT::default();
    let _ = unsafe {
        DwmGetWindowAttribute(hwnd, DWMWA_EXTENDED_FRAME_BOUNDS, (&raw mut r).cast(), size_of::<RECT>() as u32)
    };
    r
}

fn window_info(hwnd: HWND) -> Value {
    let mut client = RECT::default();
    let mut origin = POINT::default();
    unsafe {
        let _ = GetClientRect(hwnd, &mut client);
        let _ = ClientToScreen(hwnd, &mut origin);
    }
    json!({
        "hwnd": hwnd.0 as isize as i64,
        "frame": rect_json(frame(hwnd)),
        "client": [origin.x, origin.y, client.right, client.bottom],
        "dpi": unsafe { GetDpiForWindow(hwnd) },
        "maximized": unsafe { IsZoomed(hwnd) }.as_bool(),
        "minimized": unsafe { IsIconic(hwnd) }.as_bool(),
        "foreground": unsafe { GetForegroundWindow() } == hwnd,
    })
}

fn monitors() -> Vec<Value> {
    unsafe extern "system" fn each(monitor: HMONITOR, _: HDC, _: *mut RECT, lparam: LPARAM) -> BOOL {
        let out = unsafe { &mut *(lparam.0 as *mut Vec<Value>) };
        let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        if unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
            out.push(json!({ "rect": rect_json(info.rcMonitor), "work": rect_json(info.rcWork), "primary": info.dwFlags & 1 == 1 }));
        }
        BOOL::from(true)
    }
    let mut out: Vec<Value> = Vec::new();
    let _ = unsafe { EnumDisplayMonitors(None, None, Some(each), LPARAM(&raw mut out as isize)) };
    out
}
