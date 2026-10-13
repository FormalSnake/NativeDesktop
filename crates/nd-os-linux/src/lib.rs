//! Linux OS services for the GPUI backend (ARCHITECTURE.md 2, 5.4). M2: window chrome. The
//! decoration mode and `wm_capabilities` decide whether the app draws window controls at all
//! ([`chrome`]); [`window`] reads them from a GPUI window and runs drag regions and controls.

pub mod chrome;
pub mod window;
