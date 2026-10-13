//! The one crate apps depend on: `use nd::prelude::*;`.
//!
//! M0 state: the vocabulary, retained tree and layout are real (`nd-ui`); `scaffold`,
//! `toolbar` and the theme are composed stand-ins until `nd-components` and `nd-theme` land,
//! and `Application::run` has no platform host yet.

#[cfg(feature = "build")]
pub mod build;
mod chrome;
pub mod theme;

use std::fmt;

pub use nd_core as core;
pub use nd_ui::{actions, backend, dispatch, events, transaction};

#[cfg(target_os = "macos")]
pub use nd_backend_appkit as appkit;

pub use chrome::{Scaffold, Toolbar, scaffold, toolbar};

pub mod prelude {
    pub use nd_ui::prelude::*;

    pub use crate::theme::{ActiveTheme, Theme};
    pub use crate::{Application, Scaffold, Toolbar, scaffold, toolbar};
}

#[derive(Debug)]
pub enum Error {
    /// No platform host is linked for this target yet.
    NoBackend,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NoBackend => write!(f, "no platform backend is linked for this target"),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T = (), E = Error> = std::result::Result<T, E>;

/// Identity of the app, from its `Cargo.toml` (`nativedesktop.toml` from M7).
#[derive(Clone, Debug)]
pub struct Manifest {
    pub name: &'static str,
    pub version: &'static str,
}

#[macro_export]
macro_rules! manifest {
    () => {
        $crate::Manifest { name: env!("CARGO_PKG_NAME"), version: env!("CARGO_PKG_VERSION") }
    };
}

pub struct Application {
    manifest: Manifest,
}

impl Application {
    pub fn new(manifest: Manifest) -> Self {
        Self { manifest }
    }

    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// Runs the app on the platform host. Until a host is linked this returns
    /// [`Error::NoBackend`] without calling `init`.
    pub fn run<R>(self, init: impl FnOnce(&mut nd_ui::App) -> R + 'static) -> Result {
        let _ = init;
        Err(Error::NoBackend)
    }
}
