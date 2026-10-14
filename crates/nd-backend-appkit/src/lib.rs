//! macOS backend. nd owns `NSApplication` ([`NdAppKitPlatform`]); [`AppKitBackend`] implements
//! the backend seam (ARCHITECTURE.md 3.15) over NSViews; [`WindowHost`] gives a window its
//! content root, display clock and traffic light placement; [`run`] drives an `nd-ui` app on
//! them. [`NdWindow`] routes key-downs into nd dispatch before AppKit's first responder, and
//! [`AppKitDispatch`] keeps AppKit's first responder and key view loop in step with nd focus.
#![cfg(target_os = "macos")]

mod app_host;
mod backend;
mod button;
mod controls;
pub mod custom;
mod diag;
mod dispatch;
mod events;
mod form;
mod forms;
mod frame_clock;
mod gestures;
mod glass;
mod host;
mod icons;
mod keys;
mod look;
mod media;
mod overlay;
mod palette;
mod platform;
mod scaffold;
mod scroll_view;
mod services;
mod source_list;
mod surface;
mod swift;
mod system_colors;
mod titlebar;
mod toast;
mod toolbar;
mod traffic_lights;
mod views;
mod virtual_list;
mod window;

pub use app_host::{AppHost, AppJob, AppSender, defer_with_app, run, run_with};
pub use backend::{AppKitBackend, PaletteLayout, PaletteRowLayout};
pub use dispatch::AppKitDispatch;
pub use frame_clock::{FrameClock, FrameTick};
pub use host::WindowHost;
pub use keys::keystroke;
pub use platform::NdAppKitPlatform;
pub use services::{AppKitChrome, menu_bar};
pub use swift::SwiftGauge;
pub use system_colors::{AppearanceObserver, os_theme, scheme_of};
pub use toolbar::ToolbarHost;
pub use window::{KeyRouter, NdWindow};

pub use objc2;
pub use objc2_app_kit;
pub use objc2_foundation;
