//! macOS backend. nd owns `NSApplication` ([`NdAppKitPlatform`]); [`AppKitBackend`] implements
//! the backend seam (ARCHITECTURE.md 3.15) over NSViews; [`WindowHost`] gives a window its
//! content root, display clock and traffic light placement; [`NdWindow`] routes key-downs into
//! nd dispatch before AppKit's first responder, and [`AppKitDispatch`] keeps AppKit's first
//! responder and key view loop in step with nd focus.
#![cfg(target_os = "macos")]

mod backend;
mod button;
mod dispatch;
mod events;
mod frame_clock;
mod host;
mod keys;
mod platform;
pub mod props;
mod toolbar;
mod traffic_lights;
mod views;
mod window;

pub use backend::AppKitBackend;
pub use dispatch::AppKitDispatch;
pub use events::{EventQueue, NativeEvent, NativeEventKind};
pub use frame_clock::{FrameClock, FrameTick};
pub use host::{HostOptions, WindowHost};
pub use keys::keystroke;
pub use platform::NdAppKitPlatform;
pub use toolbar::ToolbarHost;
pub use window::{KeyRouter, NdWindow};

pub use objc2;
pub use objc2_app_kit;
pub use objc2_foundation;
