use std::fmt;

use crate::chrome;

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

    pub use nd_ui::{Bezel, ButtonVariant, Titlebar, WindowKind};

    pub use crate::chrome::{
        Alert, AlertResponse, AlertStyle, ButtonExt, ContextMenu, Glass, GlassGroup, Icon, Mapping, Menu, MenuItem,
        MenuRole, OnPlatform, OpenDialog, Pane, PaneCollapsed, Pattern, Patterns, SaveDialog, Select, SelectChanged,
        Separator, Slider, SliderChanged, SubtreeOverrides, SystemItem, Titlebar as TitlebarElement, WindowChromeExt,
        glass, glass_group, icon_button, inspector, open_settings, select, separator, set_menus, sidebar, slider,
        system_item, titlebar, toolbar_sidebar_toggle,
    };
    pub use crate::controls::{
        Canvas, DragMoveEvent, Gestures, ImageCache, ImageSource, ListState, Painter, Processor, SelectEvent,
        SourceList, canvas, source_list, source_row,
    };
    pub use crate::native::{ElementId, NativeStyled};
    pub use crate::theme::{
        ActiveTheme, Appearance, GlassVariant, Material, Theme, ThemeOverride, ThemeRefinement, WindowThemeExt,
    };
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
    patterns: crate::chrome::Patterns,
}

impl Application {
    pub fn new(manifest: Manifest) -> Self {
        Self { manifest, theme: None, patterns: crate::chrome::Patterns::default() }
    }

    /// The app-level theme override, on top of the platform preset and the palette sources.
    pub fn theme(mut self, theme: crate::theme::ThemeOverride) -> Self {
        self.theme = Some(theme);
        self
    }

    /// App-level pattern choices, over `nativedesktop.toml` `[patterns]`. Readable as a global.
    pub fn patterns(mut self, patterns: crate::chrome::Patterns) -> Self {
        self.patterns = patterns;
        self
    }

    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// Builds the app on AppKit as [`Application::run`] does (theme sources, chrome services,
    /// `init`) without starting the run loop: tier 2 tests pump events through the returned
    /// host's platform and read the windows back. Theme files are watched as in a real run.
    #[cfg(target_os = "macos")]
    pub fn host<R>(self, init: impl FnOnce(&mut nd_ui::App) -> R) -> nd_backend_appkit::AppHost {
        let files = crate::theme::ThemeFiles::for_app(self.manifest.name);
        let watched = files.clone();
        let host = nd_backend_appkit::AppHost::new(move |cx| {
            mac::install(cx, &files, self.theme.unwrap_or_default(), self.patterns);
            init(cx);
            crate::theme::sync(cx);
        });
        mac::watch_theme_files(&watched, host.sender());
        host
    }

    /// Runs the app on the platform host and returns when it quits. Where no host is linked
    /// yet (everything but macOS until M2) this returns [`Error::NoBackend`] without calling
    /// `init`. Diagnostics (logs, incident bundles, crash and hang capture) start first; in the
    /// copy of the executable launched as the crash helper this never returns.
    pub fn run<R>(self, init: impl FnOnce(&mut nd_ui::App) -> R + 'static) -> Result {
        #[cfg(target_os = "macos")]
        {
            nd_diag::init(nd_diag::Config::new(self.manifest.name, self.manifest.version, "appkit"));
            let files = crate::theme::ThemeFiles::for_app(self.manifest.name);
            let watched = files.clone();
            nd_backend_appkit::run_with(
                move |cx| {
                    mac::install(cx, &files, self.theme.unwrap_or_default(), self.patterns);
                    init(cx);
                    crate::theme::sync(cx);
                },
                move |sender| {
                    let jobs = sender.clone();
                    if let Err(e) = nd_automation::start_from_env(move |job| jobs.send(job)) {
                        eprintln!("nd: automation socket: {e}");
                    }
                    mac::watch_theme_files(&watched, sender);
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

#[cfg(target_os = "macos")]
mod mac {
    use std::rc::Rc;

    use nd_backend_appkit::objc2_foundation::MainThreadMarker;
    use nd_backend_appkit::{AppKitBackend, AppKitChrome, AppSender, AppearanceObserver, os_theme};
    use nd_theme::{MatugenTheme, Preset, ThemeFileWatcher, ThemeFiles, ThemeRefinement, ThemeRegistry, theme_toml};
    use nd_ui::App;

    use crate::chrome::{Chrome, Patterns};
    use crate::theme::{ThemeState, update_registry};

    /// Keeps the appearance observer and the file watchers alive with the app.
    struct ThemeSources {
        _observer: AppearanceObserver,
    }

    struct Watchers(#[allow(dead_code)] Vec<ThemeFileWatcher>);

    fn read<T, E: std::fmt::Display>(
        path: Option<&std::path::Path>,
        parse: impl Fn(&str) -> Result<T, E>,
    ) -> Option<T> {
        let path = path?;
        let text = std::fs::read_to_string(path).ok()?;
        parse(&text).map_err(|e| eprintln!("nd: {}: {e}", path.display())).ok()
    }

    /// The theme registry from the OS and the app's files, the chrome services, the default
    /// menu bar, and the observer that re-reads the OS on an appearance or accent change.
    pub fn install(cx: &mut App, files: &ThemeFiles, app_theme: ThemeRefinement, patterns: Patterns) {
        let mtm = MainThreadMarker::new().expect("main thread");
        let mut registry = ThemeRegistry::new(Preset::MacOs);
        registry.set_os(os_theme(mtm));
        registry.set_app_override(app_theme);
        if let Some(user) = read(files.toml.as_deref(), theme_toml::parse) {
            registry.set_user(user);
        }
        registry.set_matugen(read(files.matugen.as_deref(), MatugenTheme::parse));
        cx.set_global(ThemeState(registry));
        if let Some(backend) = cx.backend_as::<AppKitBackend>() {
            cx.set_global(Chrome(Rc::new(AppKitChrome::new(backend.clone()))));
        }
        cx.set_global(patterns);
        crate::chrome::set_menus(cx, []);
        let observer = AppearanceObserver::new(mtm, move || {
            nd_backend_appkit::defer_with_app(move |app| update_registry(app, |r| r.set_os(os_theme(mtm))));
        });
        cx.set_global(ThemeSources { _observer: observer });
    }

    /// `theme.toml` and `theme.json` reload live: each new parseable version reaches the app on
    /// the main thread and recolours it in the next frame.
    pub fn watch_theme_files(files: &ThemeFiles, sender: AppSender) {
        let mut watchers = Vec::new();
        if let Some(path) = &files.toml {
            let send = sender.clone();
            let watcher = ThemeFileWatcher::spawn(path, theme_toml::parse, move |theme| {
                send.send(Box::new(move |app| update_registry(app, |r| r.set_user(theme))));
            });
            watchers.extend(watcher.map_err(|e| eprintln!("nd: watching {}: {e}", path.display())).ok());
        }
        if let Some(path) = &files.matugen {
            let send = sender.clone();
            let watcher = ThemeFileWatcher::spawn(path, MatugenTheme::parse, move |theme| {
                send.send(Box::new(move |app| update_registry(app, |r| r.set_matugen(Some(theme)))));
            });
            watchers.extend(watcher.map_err(|e| eprintln!("nd: watching {}: {e}", path.display())).ok());
        }
        sender.send(Box::new(move |app| app.set_global(Watchers(watchers))));
    }
}
