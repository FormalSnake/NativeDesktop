//! The only crate that names GPUI (ARCHITECTURE.md 2, 2.1). Everything else reaches it as
//! `nd_gpui::gpui`. The pinned snapshot, the fork branch and the patch queue are described in
//! `fork.toml`; `scripts/gpui-fork` applies, vendors, bumps and gates them.

pub use gpui;

#[cfg(feature = "platform")]
pub use gpui_platform;

/// gpui-fast under its own name, so the baseline can open a window on either.
#[cfg(feature = "fast")]
pub mod fast {
    pub use gpui_fast as gpui;
    pub use gpui_fast_platform as gpui_platform;
}
