//! The GPUI backend (ARCHITECTURE.md 3.7, 3.8): Linux, Windows and web, and macOS behind
//! `ND_BACKEND=gpui` for testing. [`GpuiBackend`] implements the backend seam (3.15) over a
//! retained copy of each window's tree; [`GpuiHost`] runs an `nd-ui` app on GPUI with one GPUI
//! window per app window, nd-owned key dispatch (3.12), a display clock per window (5.3) and the
//! automation and diagnostics hooks. OS crates plug in through [`OsHooks`].

mod backend;
mod budget;
mod clock;
mod drawn;
mod gestures;
mod host;
mod input;
pub mod os;
pub mod render;

pub use backend::GpuiBackend;
pub use clock::{DisplayClock, Pacing};
pub use host::{AppJob, AppSender, GpuiHost, HostBuilder, NdView, run, run_with};
pub use os::{ChromeContext, FrameRequest, NoOsHooks, OsHooks};
pub use render::Look;
