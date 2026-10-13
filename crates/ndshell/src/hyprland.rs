//! One private headless Hyprland per test run (ARCHITECTURE.md 10.2, 10.3), under its own
//! `XDG_RUNTIME_DIR`, so its Wayland socket, IPC socket, seat, focus and cursor belong to that run
//! alone and any number of runs go in parallel. Nothing here reads or writes the user's Hyprland
//! config: the instance gets a generated Lua config in its runtime dir.
//!
//! `HYPRLAND_HEADLESS_ONLY` is not read by Hyprland 0.56 or aquamarine, and aquamarine's headless
//! backend has no allocator of its own: it borrows the DRM fd of a DRM or Wayland backend. A DRM
//! backend needs the seat, which the user's session holds. So each instance nests on a private
//! labwc running wlroots' headless backend on a render node (`WLR_RENDER_DRM_DEVICE`), which lends
//! Hyprland's Wayland backend its dmabuf device and shows nothing anywhere. labwc because
//! aquamarine binds `xdg_wm_base` v6 and sway and cage offer v5. Hyprland's own output on labwc is
//! disabled; tests run on `HEADLESS-1`, created over IPC the way Hyprland's `hyprtester` does it.
//!
//! Windows float, borders are 2 px [`BORDER`], the background is [`BACKGROUND`], and animations,
//! rounding, shadows and blur are off, so captures show exact geometry. `ND_HYPRLAND` and
//! `ND_LABWC` override the binaries (default: on `PATH`), `ND_RENDER_NODE` the render node (default:
//! the first `/dev/dri/renderD*`), and `ND_KEEP_HYPRLAND=1` keeps the runtime dir and its logs.

use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

pub const OUTPUT: &str = "HEADLESS-1";
/// Window border, RGB, drawn active and inactive alike.
pub const BORDER: [u8; 3] = [0x00, 0xff, 0x00];
pub const BORDER_SIZE: u32 = 2;
/// The compositor background behind every window.
pub const BACKGROUND: [u8; 3] = [0x00, 0x00, 0xff];

#[derive(Clone, Debug)]
pub struct Options {
    pub width: u32,
    pub height: u32,
    pub refresh: u32,
    /// Lua appended to the generated config.
    pub extra_config: String,
}

impl Default for Options {
    fn default() -> Self {
        Self { width: 1600, height: 1000, refresh: 60, extra_config: String::new() }
    }
}

/// A window as `j/clients` reports it, in logical pixels of the layout.
#[derive(Clone, Debug, PartialEq)]
pub struct Client {
    pub address: String,
    pub class: String,
    pub pid: i64,
    pub at: (i32, i32),
    pub size: (i32, i32),
    pub floating: bool,
}

#[derive(Debug)]
pub struct Hyprland {
    child: Child,
    parent: Child,
    runtime_dir: PathBuf,
    signature: String,
    wayland_display: String,
    pub options: Options,
}

impl Hyprland {
    pub fn start(options: Options) -> Result<Self, String> {
        let runtime_dir = new_runtime_dir()?;
        let mut parent = start_parent(&runtime_dir)?;
        let parent_display = match wait_socket(&runtime_dir, &mut parent, "labwc") {
            Ok(display) => display,
            Err(why) => {
                let _ = parent.kill();
                let _ = parent.wait();
                return Err(why);
            }
        };
        let config = runtime_dir.join("hyprland.lua");
        fs::write(&config, config_lua(&options)).map_err(|e| format!("write {}: {e}", config.display()))?;
        let log = fs::File::create(runtime_dir.join("hyprland.log")).map_err(|e| format!("log: {e}"))?;
        let binary = std::env::var_os("ND_HYPRLAND").unwrap_or_else(|| "Hyprland".into());
        let mut command = Command::new(&binary);
        command
            .arg("--config")
            .arg(&config)
            .env("XDG_RUNTIME_DIR", &runtime_dir)
            .env("XDG_CONFIG_HOME", runtime_dir.join("config"))
            .env("WAYLAND_DISPLAY", &parent_display)
            // Keeps the instance out of the user's systemd and dbus activation environment.
            .env("HYPRLAND_NO_SD_VARS", "1")
            .env_remove("DISPLAY")
            .env_remove("HYPRLAND_INSTANCE_SIGNATURE")
            // The nd dev shell points these at its own mesa and lavapipe; the compositor renders
            // with the system's driver.
            .env_remove("LD_LIBRARY_PATH")
            .env_remove("VK_ICD_FILENAMES")
            .current_dir(&runtime_dir)
            .stdin(Stdio::null())
            .stdout(log.try_clone().map_err(|e| e.to_string())?)
            .stderr(log);
        let child = match command.spawn() {
            Ok(child) => child,
            Err(e) => {
                let _ = parent.kill();
                let _ = parent.wait();
                return Err(format!("{}: {e}", Path::new(&binary).display()));
            }
        };
        let mut this =
            Self { child, parent, runtime_dir, signature: String::new(), wayland_display: String::new(), options };
        this.wait_ready()?;
        Ok(this)
    }

    fn wait_ready(&mut self) -> Result<(), String> {
        let hypr = self.runtime_dir.join("hypr");
        let end = Instant::now() + Duration::from_secs(15);
        loop {
            if let Ok(Some(status)) = self.child.try_wait() {
                return Err(format!("Hyprland exited with {status}; log: {}", self.log_tail()));
            }
            if let Some((signature, display)) = read_lock(&hypr) {
                self.signature = signature;
                self.wayland_display = display;
                if self.request("version").is_ok() {
                    break;
                }
            }
            if Instant::now() > end {
                return Err(format!("Hyprland did not come up in 15 s; log: {}", self.log_tail()));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let errors = self.request("configerrors")?;
        if !errors.trim().is_empty() {
            return Err(format!("generated config has errors: {errors}"));
        }
        let created = self.request(&format!("output create headless {OUTPUT}"))?;
        if created.trim() != "ok" {
            return Err(format!("output create headless {OUTPUT}: {created}"));
        }
        let mut monitors = String::new();
        while Instant::now() < end {
            monitors = self.request("j/monitors")?;
            let parsed: serde_json::Value = serde_json::from_str(&monitors).map_err(|e| e.to_string())?;
            let ours = parsed.as_array().into_iter().flatten().find(|m| m["name"] == OUTPUT);
            if ours.is_some_and(|m| m["width"] == self.options.width && m["height"] == self.options.height) {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        Err(format!("{OUTPUT} did not come up at {}x{}: {monitors}", self.options.width, self.options.height))
    }

    /// The instance's private runtime dir; its Wayland socket lives here.
    pub fn runtime_dir(&self) -> &Path {
        &self.runtime_dir
    }

    pub fn wayland_socket(&self) -> PathBuf {
        self.runtime_dir.join(&self.wayland_display)
    }

    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// The environment a client needs to land on this instance and nowhere else.
    pub fn client_env(&self) -> Vec<(&'static str, String)> {
        vec![
            ("XDG_RUNTIME_DIR", self.runtime_dir.display().to_string()),
            ("WAYLAND_DISPLAY", self.wayland_display.clone()),
            ("HYPRLAND_INSTANCE_SIGNATURE", self.signature.clone()),
        ]
    }

    /// `program` set up to run as a client of this instance.
    pub fn command(&self, program: impl AsRef<std::ffi::OsStr>) -> Command {
        let mut command = Command::new(program);
        command.env_remove("DISPLAY").env_remove("WAYLAND_SOCKET");
        for (key, value) in self.client_env() {
            command.env(key, value);
        }
        command
    }

    /// One IPC request (`hyprctl`'s wire format: `[flags]/command`), returning the reply.
    pub fn request(&self, message: &str) -> Result<String, String> {
        let path = self.runtime_dir.join("hypr").join(&self.signature).join(".socket.sock");
        let mut stream = UnixStream::connect(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        stream.set_read_timeout(Some(Duration::from_secs(5))).map_err(|e| e.to_string())?;
        stream.write_all(message.as_bytes()).map_err(|e| format!("ipc write: {e}"))?;
        let mut reply = String::new();
        stream.read_to_string(&mut reply).map_err(|e| format!("ipc read: {e}"))?;
        Ok(reply)
    }

    pub fn clients(&self) -> Result<Vec<Client>, String> {
        let reply = self.request("j/clients")?;
        let parsed: serde_json::Value = serde_json::from_str(&reply).map_err(|e| format!("{e}: {reply}"))?;
        let pair = |v: &serde_json::Value| (v[0].as_i64().unwrap_or(0) as i32, v[1].as_i64().unwrap_or(0) as i32);
        Ok(parsed
            .as_array()
            .into_iter()
            .flatten()
            .map(|c| Client {
                address: c["address"].as_str().unwrap_or_default().to_owned(),
                class: c["class"].as_str().unwrap_or_default().to_owned(),
                pid: c["pid"].as_i64().unwrap_or(-1),
                at: pair(&c["at"]),
                size: pair(&c["size"]),
                floating: c["floating"].as_bool().unwrap_or(false),
            })
            .collect())
    }

    /// Waits for a mapped window of `pid`.
    pub fn wait_for_client(&self, pid: u32, timeout: Duration) -> Result<Client, String> {
        let end = Instant::now() + timeout;
        loop {
            if let Some(c) = self.clients()?.into_iter().find(|c| c.pid == i64::from(pid) && c.size.0 > 0) {
                return Ok(c);
            }
            if Instant::now() > end {
                return Err(format!("no window from pid {pid} after {timeout:?}: {:?}", self.clients()));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    pub fn log_tail(&self) -> String {
        let log = fs::read_to_string(self.runtime_dir.join("hyprland.log")).unwrap_or_default();
        let lines: Vec<&str> = log.lines().collect();
        lines[lines.len().saturating_sub(15)..].join("\n")
    }
}

impl Drop for Hyprland {
    fn drop(&mut self) {
        use rustix::process::{Pid, Signal, kill_process};
        let _ = kill_process(Pid::from_child(&self.child), Signal::TERM);
        let end = Instant::now() + Duration::from_secs(3);
        while Instant::now() < end {
            if let Ok(Some(_)) = self.child.try_wait() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = self.parent.kill();
        let _ = self.parent.wait();
        if std::env::var_os("ND_KEEP_HYPRLAND").is_none() {
            let _ = fs::remove_dir_all(&self.runtime_dir);
        }
    }
}

/// `$XDG_RUNTIME_DIR/nd<pid in hex>-<n>`, mode 0700 as the Wayland spec asks of a runtime dir. The
/// name stays short: Hyprland puts `hypr/<60-character signature>/.socket2.sock` under it, and a
/// unix socket path is capped at 107 bytes.
pub(crate) fn new_runtime_dir() -> Result<PathBuf, String> {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let base = std::env::var_os("XDG_RUNTIME_DIR").map_or_else(std::env::temp_dir, PathBuf::from);
    let dir = base.join(format!("nd{:x}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("config")).map_err(|e| format!("{}: {e}", dir.display()))?;
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).map_err(|e| format!("{}: {e}", dir.display()))?;
    Ok(dir)
}

fn render_node() -> Result<PathBuf, String> {
    if let Some(node) = std::env::var_os("ND_RENDER_NODE") {
        return Ok(node.into());
    }
    let mut nodes: Vec<PathBuf> = fs::read_dir("/dev/dri")
        .map_err(|e| format!("/dev/dri: {e}"))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("renderD")))
        .collect();
    nodes.sort();
    nodes.into_iter().next().ok_or_else(|| "no /dev/dri/renderD* node for the headless compositor".into())
}

/// labwc on wlroots' headless backend: no outputs to show, no input devices, its config and
/// sockets in `dir`.
fn start_parent(dir: &Path) -> Result<Child, String> {
    let log = fs::File::create(dir.join("labwc.log")).map_err(|e| format!("log: {e}"))?;
    let binary = std::env::var_os("ND_LABWC").unwrap_or_else(|| "labwc".into());
    Command::new(&binary)
        .env("XDG_RUNTIME_DIR", dir)
        .env("XDG_CONFIG_HOME", dir.join("config"))
        .env("WLR_BACKENDS", "headless")
        .env("WLR_LIBINPUT_NO_DEVICES", "1")
        .env("WLR_RENDER_DRM_DEVICE", render_node()?)
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("DISPLAY")
        .env_remove("HYPRLAND_INSTANCE_SIGNATURE")
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(log.try_clone().map_err(|e| e.to_string())?)
        .stderr(log)
        .spawn()
        .map_err(|e| format!("{}: {e} (the nd dev shell provides labwc)", Path::new(&binary).display()))
}

/// The parent's Wayland socket name, once it listens.
pub(crate) fn wait_socket(dir: &Path, parent: &mut Child, name: &str) -> Result<String, String> {
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        if let Ok(Some(status)) = parent.try_wait() {
            let log = fs::read_to_string(dir.join(format!("{name}.log"))).unwrap_or_default();
            return Err(format!("{name} exited with {status}: {log}"));
        }
        let socket = fs::read_dir(dir).ok().into_iter().flatten().flatten().find_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let is_socket = name.starts_with("wayland-") && !name.ends_with(".lock");
            is_socket.then_some(name)
        });
        if let Some(name) = socket
            && UnixStream::connect(dir.join(&name)).is_ok()
        {
            return Ok(name);
        }
        if Instant::now() > end {
            return Err(format!("{name} did not open a Wayland socket in 10 s"));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// `hypr/<signature>/hyprland.lock` holds the compositor's pid, then its Wayland socket name.
fn read_lock(hypr: &Path) -> Option<(String, String)> {
    for entry in fs::read_dir(hypr).ok()?.flatten() {
        let lock = fs::read_to_string(entry.path().join("hyprland.lock")).ok()?;
        let display = lock.lines().nth(1)?.trim().to_owned();
        if !display.is_empty() {
            return Some((entry.file_name().to_string_lossy().into_owned(), display));
        }
    }
    None
}

fn rgb([r, g, b]: [u8; 3]) -> String {
    format!("rgb({r:02x}{g:02x}{b:02x})")
}

fn config_lua(o: &Options) -> String {
    format!(
        r#"-- Generated by ndshell for one headless test run.
hl.monitor({{ output = "{OUTPUT}", mode = "{w}x{h}@{hz}", position = "0x0", scale = "1" }})
hl.monitor({{ output = "", disabled = true }})

hl.config({{
    general = {{
        gaps_in = 0,
        gaps_out = 0,
        border_size = {BORDER_SIZE},
        resize_on_border = true,
        extend_border_grab_area = 15,
        hover_icon_on_border = false,
        col = {{ active_border = "{border}", inactive_border = "{border}" }},
    }},
    decoration = {{
        rounding = 0,
        shadow = {{ enabled = false }},
        blur = {{ enabled = false }},
    }},
    animations = {{ enabled = false }},
    misc = {{
        disable_hyprland_logo = true,
        disable_splash_rendering = true,
        force_default_wallpaper = 0,
        background_color = "{background}",
        enable_anr_dialog = false,
        disable_watchdog_warning = true,
    }},
    -- Not drawn into captures; the virtual pointer still moves it.
    cursor = {{ invisible = true }},
    ecosystem = {{ no_update_news = true, no_donation_nag = true }},
    xwayland = {{ enabled = false }},
}})

hl.window_rule({{ name = "nd-float", match = {{ class = ".*" }}, float = true }})
{extra}
"#,
        w = o.width,
        h = o.height,
        hz = o.refresh,
        border = rgb(BORDER),
        background = rgb(BACKGROUND),
        extra = o.extra_config,
    )
}
