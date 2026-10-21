//! The GPUI backend (ARCHITECTURE.md 3.7, 3.8): Linux, Windows and web, and macOS behind
//! `ND_BACKEND=gpui` for testing. [`GpuiBackend`] implements the backend seam (3.15) over a
//! retained copy of each window's tree; [`GpuiHost`] runs an `nd-ui` app on GPUI with one GPUI
//! window per app window, nd-owned key dispatch (3.12), a display clock per window (5.3) and the
//! automation and diagnostics hooks. OS crates plug in through [`OsHooks`].

mod backend;
mod budget;
mod canvas;
mod clock;
pub mod custom;
pub mod dialogs;
mod drawn;
mod fade;
mod fluent_icons;
mod forms;
mod gestures;
mod host;
mod input;
mod media;
pub mod nav;
pub mod occlusion;
pub mod os;
mod overlay;
pub mod render;
mod rich;
mod scrollbar;
mod split;
pub mod status;
pub mod tab_bar;
mod virtual_list;
mod window_tabs;

pub use backend::{ActivationListener, FileDropListener, GpuiBackend};
pub use clock::{DisplayClock, Pacing};
#[cfg(target_os = "linux")]
pub use host::on_x11;
pub use host::{AppJob, AppSender, GpuiHost, HostBuilder, NdView, prefer_x11, run, run_with};
pub use os::{ChromeContext, FrameRequest, NoOsHooks, OsHooks};
pub use overlay::{GpuiChrome, MenuResponse, MenuRow};
pub use render::Look;
pub use window_tabs::TabGroup;
