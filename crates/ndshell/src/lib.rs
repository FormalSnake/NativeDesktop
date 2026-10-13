//! Test-only OS helpers (ARCHITECTURE.md 10.2): real HID input, the preflight that tier 3 tests run
//! before posting it, the machine-wide lock real-input runs queue on, and TCC grants. Never linked
//! by apps.

#[cfg(target_os = "macos")]
pub mod grant;
#[cfg(target_os = "macos")]
pub mod input;
pub mod lock;
#[cfg(target_os = "macos")]
pub mod preflight;
