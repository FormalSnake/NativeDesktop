//! Updates: Velopack's installer, deltas and channels, behind a feed whose ed25519 signature is
//! checked before Velopack reads it (ARCHITECTURE.md 11.3). Velopack alone trusts whatever
//! hashes the feed lists, so a compromised host could otherwise ship any package.
//!
//! The feed is a directory of static files with relative names, served from any host. The app
//! carries the feed URL and public key from `nativedesktop.toml`, baked in by `nd::build::app()`
//! and read back with [`build_config!`]. Builds made with `ND_NO_UPDATES=1` (Nix) carry neither,
//! a binary running from `/nix/store` refuses to update, and wasm builds compile the client out.

#[cfg(all(feature = "client", not(target_arch = "wasm32")))]
mod client;
pub mod feed;
#[cfg(all(feature = "client", not(target_arch = "wasm32")))]
pub mod source;

#[cfg(all(feature = "client", not(target_arch = "wasm32")))]
pub use client::{Error, Update, Updater, WhatsNew, mark_whats_new_seen, startup, whats_new};

/// Feed and key the app was built with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UpdateConfig {
    /// Base URL (or `file://` path) of the static feed directory.
    pub feed: &'static str,
    /// Base64 ed25519 public key.
    pub public_key: &'static str,
}

/// The [`UpdateConfig`] baked into the calling crate by its build script, `None` without
/// `[updates]` or in a `ND_NO_UPDATES=1` build. Expands `option_env!` at the call site, so it
/// must be called from the app's own crate (`nd::manifest!()` does).
#[macro_export]
macro_rules! build_config {
    () => {
        match (::core::option_env!("ND_UPDATE_FEED"), ::core::option_env!("ND_UPDATE_PUBLIC_KEY")) {
            (Some(feed), Some(public_key)) => Some($crate::UpdateConfig { feed, public_key }),
            _ => None,
        }
    };
}

/// Why this process does not update itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unavailable {
    /// No `[updates]` in `nativedesktop.toml`, or a build with `ND_NO_UPDATES=1`.
    NotConfigured,
    /// Running from the Nix store: Nix owns the files.
    Nix,
    /// The web build has nothing to update.
    Web,
    /// Not inside a Velopack-packed bundle (`cargo run`, tests).
    NotInstalled(String),
    Misconfigured(String),
}

impl std::fmt::Display for Unavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotConfigured => f.write_str("updates are not configured"),
            Self::Nix => f.write_str("installed through Nix, which owns updates"),
            Self::Web => f.write_str("web builds do not update"),
            Self::NotInstalled(why) => write!(f, "not a packaged install: {why}"),
            Self::Misconfigured(why) => write!(f, "update configuration: {why}"),
        }
    }
}

impl std::error::Error for Unavailable {}

/// Velopack's channel for a release channel on this OS: `mac-stable`, `linux-beta`,
/// `win-stable`. `nd release` packs with the same names, so OSes sharing a feed never collide.
pub fn velopack_channel(channel: &str) -> String {
    let os = if cfg!(target_os = "macos") {
        "mac"
    } else if cfg!(windows) {
        "win"
    } else {
        "linux"
    };
    format!("{os}-{channel}")
}

/// Set when this build or this process must never update.
pub fn compiled_out() -> Option<Unavailable> {
    if cfg!(target_arch = "wasm32") {
        return Some(Unavailable::Web);
    }
    let exe = std::env::current_exe().ok()?;
    exe.starts_with("/nix/store").then_some(Unavailable::Nix)
}
