//! A private headless Weston, for the client-side decoration path: Weston has no xdg-decoration,
//! so every client draws its own frame, as on GNOME. It renders with pixman into memory and shows
//! nothing; clients reach it through [`Weston::command`]. `ND_WESTON` overrides the binary.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use crate::hyprland::{new_runtime_dir, wait_socket};

#[derive(Debug)]
pub struct Weston {
    child: Child,
    runtime_dir: PathBuf,
    wayland_display: String,
}

impl Weston {
    pub fn start(width: u32, height: u32) -> Result<Self, String> {
        let runtime_dir = new_runtime_dir()?;
        let log = fs::File::create(runtime_dir.join("weston.log")).map_err(|e| format!("log: {e}"))?;
        let binary = std::env::var_os("ND_WESTON").unwrap_or_else(|| "weston".into());
        let mut child = Command::new(&binary)
            .args(["--backend=headless", "--renderer=pixman", "--no-config", "--socket=wayland-nd"])
            // Headless Weston has no input devices; without a seat GPUI refuses to connect.
            .arg("--fake-seat")
            .arg(format!("--width={width}"))
            .arg(format!("--height={height}"))
            .env("XDG_RUNTIME_DIR", &runtime_dir)
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("DISPLAY")
            .current_dir(&runtime_dir)
            .stdin(Stdio::null())
            .stdout(log.try_clone().map_err(|e| e.to_string())?)
            .stderr(log)
            .spawn()
            .map_err(|e| format!("{}: {e} (the nd dev shell provides weston)", Path::new(&binary).display()))?;
        match wait_socket(&runtime_dir, &mut child, "weston") {
            Ok(wayland_display) => Ok(Self { child, runtime_dir, wayland_display }),
            Err(why) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = fs::remove_dir_all(&runtime_dir);
                Err(why)
            }
        }
    }

    /// `program` set up to run as a client of this instance.
    pub fn command(&self, program: impl AsRef<std::ffi::OsStr>) -> Command {
        let mut command = Command::new(program);
        command
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_SOCKET")
            .env_remove("HYPRLAND_INSTANCE_SIGNATURE")
            .env("XDG_RUNTIME_DIR", &self.runtime_dir)
            .env("WAYLAND_DISPLAY", &self.wayland_display);
        command
    }
}

impl Drop for Weston {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_dir_all(&self.runtime_dir);
    }
}
