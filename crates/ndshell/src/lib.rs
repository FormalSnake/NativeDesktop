//! Test-only OS helpers (ARCHITECTURE.md 10.2): real HID input, the preflight that tier 3 tests run
//! before posting it, the machine-wide lock real-input runs queue on, TCC grants, and the Windows
//! agent that posts input and captures frames from the desktop session. Never linked by apps.

pub mod agent;

#[cfg(target_os = "macos")]
pub mod grant;
#[cfg(target_os = "macos")]
pub mod input;
pub mod lock;
#[cfg(target_os = "macos")]
pub mod preflight;
