//! macOS backend. Day one carries the dispatch spike (ARCHITECTURE.md 3.12): nd owns
//! `NSApplication` ([`NdAppKitPlatform`]), its window subclass routes key-downs into nd dispatch
//! before AppKit's first responder ([`NdWindow`]), and AppKit's first responder mirrors nd focus
//! both ways ([`AppKitDispatch`]), with the key view loop computed by nd.
#![cfg(target_os = "macos")]

mod dispatch;
mod keys;
mod platform;
mod swift;
mod toolbar;
mod window;

pub use dispatch::AppKitDispatch;
pub use keys::keystroke;
pub use platform::NdAppKitPlatform;
pub use swift::SwiftGauge;
pub use toolbar::ToolbarHost;
pub use window::{KeyRouter, NdWindow};

pub use objc2;
pub use objc2_app_kit;
pub use objc2_foundation;
