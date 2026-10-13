//! Test-only OS helpers (ARCHITECTURE.md 10.2): real HID input, the preflight that tier 3 tests run
//! before posting it, and TCC grants. Never linked by apps.

#[cfg(target_os = "macos")]
pub mod grant;
#[cfg(target_os = "macos")]
pub mod input;
#[cfg(target_os = "macos")]
pub mod preflight;
