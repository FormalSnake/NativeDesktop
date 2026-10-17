//! Test-only OS helpers (ARCHITECTURE.md 10.2): real HID input, the preflight that tier 3 tests run
//! before posting it, the machine-wide lock real-input runs queue on, TCC grants, the Windows
//! agent that posts input and captures frames from the desktop session, and on Linux a private
//! headless Hyprland per run with virtual input and output capture. Never linked by apps.

pub mod agent;

#[cfg(target_os = "macos")]
pub mod grant;
#[cfg(target_os = "linux")]
pub mod hyprland;
#[cfg(target_os = "macos")]
pub mod input;
#[cfg(target_os = "linux")]
pub mod labwc;
pub mod lock;
#[cfg(target_os = "macos")]
pub mod preflight;
#[cfg(target_os = "linux")]
pub mod wayland;
#[cfg(target_os = "linux")]
pub mod weston;
#[cfg(target_os = "macos")]
pub mod window_list;
