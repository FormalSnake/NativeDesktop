use std::fmt;

use crate::chrome;
#[cfg(target_os = "macos")]
use crate::theme::Theme;

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

/// Writes a `-report` incident bundle with `message` and returns its directory, or `None` when
/// diagnostics are not running.
pub fn report_issue(message: &str) -> Option<std::path::PathBuf> {
    nd_diag::global()?.report_issue(message).ok()
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

    /// Runs the app on the platform host and returns when it quits. Where no host is linked
    /// yet (everything but macOS until M2) this returns [`Error::NoBackend`] without calling
    /// `init`. Diagnostics (logs, incident bundles, crash and hang capture) start first; in the
    /// copy of the executable launched as the crash helper this never returns.
    pub fn run<R>(self, init: impl FnOnce(&mut nd_ui::App) -> R + 'static) -> Result {
        #[cfg(target_os = "macos")]
        {
            nd_diag::init(nd_diag::Config::new(self.manifest.name, self.manifest.version, "appkit"));
            use nd_backend_appkit::objc2_foundation::MainThreadMarker;
            nd_backend_appkit::run_with(
                move |cx| {
                    let colors = nd_backend_appkit::system_colors(MainThreadMarker::new().expect("main thread"));
                    cx.set_global(Theme::from_system(colors));
                    init(cx);
                },
                |sender| {
                    if let Err(e) = nd_automation::start_from_env(move |job| sender.send(job)) {
                        eprintln!("nd: automation socket: {e}");
                    }
                },
            )
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = init;
            Err(Error::NoBackend)
        }
    }
}
