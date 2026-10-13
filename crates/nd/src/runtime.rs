use std::fmt;

use crate::chrome;
#[cfg(target_os = "macos")]
use crate::theme::Theme;

pub use chrome::{Scaffold, Toolbar, scaffold, toolbar};

pub mod prelude {
    pub use nd_ui::prelude::*;

    // Named here, so they win over `nd_ui::prelude`'s `NativeElement` leaf struct.
    pub use crate::native::NativeElement;
    pub use nd_macros::IntoElement;

    // GPUI's `actions!` and `#[derive(Action)]` expand to `gpui::` paths.
    #[doc(hidden)]
    pub use nd_gpui::gpui;
    pub use nd_gpui::gpui::{Action, Bounds, Font, KeyBinding, Point, Size, actions};

    pub use crate::chrome::{
        ContextMenu, Glass, GlassVariant, Separator, Sidebar, SubtreeOverrides, Titlebar as TitlebarElement, glass,
        separator, sidebar, titlebar,
    };
    pub use crate::controls::{
        Bezel, ButtonExt, Canvas, DragMoveEvent, Gestures, Icon, ImageCache, ImageSource, ListState, Mapping,
        OnPlatform, Painter, Processor, Select, SelectChanged, SelectEvent, Slider, SliderChanged, SourceList,
        Titlebar, WindowControlsExt, WindowOptionsExt, WindowThemeExt, canvas, select, slider, source_list, source_row,
    };
    pub use crate::native::{ElementId, NativeStyled};
    pub use crate::theme::{ActiveTheme, Appearance, Pattern, Patterns, Theme, ThemeOverride, ThemeRefinement};
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
    theme: Option<crate::theme::ThemeOverride>,
    patterns: crate::theme::Patterns,
}

impl Application {
    pub fn new(manifest: Manifest) -> Self {
        Self { manifest, theme: None, patterns: crate::theme::Patterns::default() }
    }

    /// The app-level theme override, on top of the platform preset. Recorded until `nd-theme`.
    pub fn theme(mut self, theme: crate::theme::ThemeOverride) -> Self {
        self.theme = Some(theme);
        self
    }

    /// App-level pattern choices, over `nativedesktop.toml` `[patterns]`. Recorded until
    /// `nd-components`.
    pub fn patterns(mut self, patterns: crate::theme::Patterns) -> Self {
        self.patterns = patterns;
        self
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
