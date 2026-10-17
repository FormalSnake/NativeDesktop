//! A private headless labwc (wlroots), for checking window behaviour on a second compositor
//! family next to Hyprland. It runs wlroots' headless backend on a render node, as the labwc that
//! hosts each Hyprland run does, and shows nothing anywhere. Server-side decorations are a 2 px
//! [`BORDER`] and a title bar of the same green with no title or buttons, so a capture finds the
//! window the way it finds a Hyprland one. Clients reach it through [`Labwc::command`]; virtual
//! input and capture go through [`Labwc::wayland_socket`]. `ND_LABWC` overrides the binary.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use crate::hyprland::{PrivateBus, new_runtime_dir, render_node, wait_socket};

/// Window border and title bar, RGB.
pub const BORDER: [u8; 3] = [0x00, 0xff, 0x00];
pub const BORDER_SIZE: u32 = 2;

const RC: &str = r#"<?xml version="1.0"?>
<labwc_config>
  <core><decoration>server</decoration><gap>0</gap></core>
  <placement><policy>center</policy></placement>
  <theme><cornerRadius>0</cornerRadius><dropShadows>no</dropShadows></theme>
  <resize><popupShow>Never</popupShow><drawContents>yes</drawContents></resize>
</labwc_config>
"#;

const THEME: &str = "border.width: 2
window.active.border.color: #00ff00
window.inactive.border.color: #00ff00
window.active.title.bg.color: #00ff00
window.inactive.title.bg.color: #00ff00
window.active.label.text.color: #00ff00
window.inactive.label.text.color: #00ff00
window.active.button.unpressed.image.color: #00ff00
window.inactive.button.unpressed.image.color: #00ff00
window.active.button.hover.image.color: #00ff00
window.inactive.button.hover.image.color: #00ff00
";

#[derive(Debug)]
pub struct Labwc {
    child: Child,
    bus: PrivateBus,
    runtime_dir: PathBuf,
    wayland_display: String,
}

impl Labwc {
    pub fn start() -> Result<Self, String> {
        let runtime_dir = new_runtime_dir()?;
        let config = runtime_dir.join("config").join("labwc");
        fs::create_dir_all(&config).map_err(|e| format!("{}: {e}", config.display()))?;
        fs::write(config.join("rc.xml"), RC).map_err(|e| e.to_string())?;
        fs::write(config.join("themerc-override"), THEME).map_err(|e| e.to_string())?;
        let bus = PrivateBus::start(&runtime_dir)?;
        let log = fs::File::create(runtime_dir.join("labwc.log")).map_err(|e| format!("log: {e}"))?;
        let binary = std::env::var_os("ND_LABWC").unwrap_or_else(|| "labwc".into());
        let mut child = Command::new(&binary)
            .env("XDG_RUNTIME_DIR", &runtime_dir)
            .env("XDG_CONFIG_HOME", runtime_dir.join("config"))
            .env("WLR_BACKENDS", "headless")
            .env("WLR_LIBINPUT_NO_DEVICES", "1")
            .env("WLR_RENDER_DRM_DEVICE", render_node()?)
            .env("DBUS_SESSION_BUS_ADDRESS", bus.address())
            .env("LABWC_UPDATE_ACTIVATION_ENV", "0")
            // The nd dev shell points these at its own mesa and lavapipe; the compositor renders
            // with the system's driver.
            .env_remove("LD_LIBRARY_PATH")
            .env_remove("VK_ICD_FILENAMES")
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("DISPLAY")
            .env_remove("HYPRLAND_INSTANCE_SIGNATURE")
            .current_dir(&runtime_dir)
            .stdin(Stdio::null())
            .stdout(log.try_clone().map_err(|e| e.to_string())?)
            .stderr(log)
            .spawn()
            .map_err(|e| format!("{}: {e} (the nd dev shell provides labwc)", Path::new(&binary).display()))?;
        match wait_socket(&runtime_dir, &mut child, "labwc") {
            Ok(wayland_display) => Ok(Self { child, bus, runtime_dir, wayland_display }),
            Err(why) => {
                let _ = child.kill();
                let _ = child.wait();
                Err(why)
            }
        }
    }

    pub fn wayland_socket(&self) -> PathBuf {
        self.runtime_dir.join(&self.wayland_display)
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

    pub fn log_tail(&self) -> String {
        let log = fs::read_to_string(self.runtime_dir.join("labwc.log")).unwrap_or_default();
        let lines: Vec<&str> = log.lines().collect();
        lines[lines.len().saturating_sub(15)..].join("\n")
    }
}

impl Drop for Labwc {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.bus.stop();
        if std::env::var_os("ND_KEEP_HYPRLAND").is_none() {
            let _ = fs::remove_dir_all(&self.runtime_dir);
        }
    }
}
