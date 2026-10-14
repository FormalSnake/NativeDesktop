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

    pub use crate::chrome::fuzzy;
    pub use crate::chrome::panes::{self, PaneResized, dock_view, pane_tree, tiles_view};
    pub use crate::chrome::{
        AboutPanel, AlertPresentation, Split, SplitPane, SplitResized, StatusPage, TabAddRequested, TabChanged,
        TabCloseRequested, TabPage, TabStrip, TabStyle, TabView, open_tab, split, split_pane, status_page, tab_page,
        tab_strip, tab_view,
    };
    pub use crate::chrome::{
        ActionVisibility, Banner, BannerActionClicked, CommandPalette, DismissReason, PaletteActivated,
        PaletteCancelled, PaletteItem, PaletteQueryChanged, PaletteSubmitted, SourceAction, SourceActionClicked,
        SourceActivated, SourceCollapsed, SourceExpanded, SourceNode, SourceTree, Toast, ToastActionClicked,
        ToastDismissed, ToastOverlay, banner, command_palette, palette_item, source_action, source_node,
        source_section, source_tree, toast, toast_overlay,
    };
    pub use crate::chrome::{
        Alert, AlertResponse, AlertStyle, ButtonExt, ContextMenu, Glass, GlassGroup, Icon, Mapping, Menu, MenuItem,
        MenuRole, OnPlatform, OpenDialog, Pane, PaneCollapsed, Pattern, Patterns, SaveDialog, Select, SelectChanged,
        Separator, Slider, SliderChanged, SourceItem, SourceSelected, SubtreeOverrides, SystemItem, TabSelected,
        Titlebar as TitlebarElement, WindowChromeExt, form, form_row, form_section, glass, glass_group, icon_button,
        inspector, open_settings, select, separator, set_menus, sidebar, slider, source_item, source_list, system_item,
        titlebar, toolbar_sidebar_toggle,
    };
    pub use crate::chrome::{
        ButtonGroup, Checkbox, CivilDateTime, Clamp, ColorChanged, ColorWell, ComboBox, DateChanged, DatePicker,
        DatePickerMode, DatePickerStyle, FieldIconClicked, FormField, FormSection, IconEdge, Label, LabelTone, Link,
        LinkActivated, Progress, RowActivated, Stepper, StepperChanged, Switch, TextArea, TextChanged, TextEvent,
        TextField, TextFieldState, TextStyle, TextSubmitted, Toggled, button_group, checkbox, clamp, color_well,
        combo_box, date_picker, form_field, label, link, navigation_row, progress, radio_group, row, search_field,
        secure_field, segmented, settings_group, sheet_layout, spinner, stepper, switch, switch_row, text_area,
        text_field,
    };
    pub use crate::controls::{Canvas, DragMoveEvent, Gestures, ImageCache, ImageSource, Painter, Processor, canvas};
    pub use crate::native::{ElementId, NativeStyled};
    pub use crate::theme::{
        ActiveTheme, Appearance, GlassVariant, Material, Theme, ThemeOverride, ThemeRefinement, WindowThemeExt,
    };
    pub use crate::{Application, Scaffold, Toolbar, scaffold, toolbar};
    #[cfg(feature = "settings-ui")]
    pub use nd_components::forms::settings::settings_form;
    pub use nd_virtual::{
        ActivateEvent, Align, Alignment, Column, ColumnResized, ExpandEvent, Follow, ListState, SelectEvent, Selection,
        SortChanged, SortDirection, TreeNode, column, grid, list, table, tree,
    };
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

/// Identity of the app, from its `Cargo.toml`, plus the update feed `nd::build::app()` baked in.
#[derive(Clone, Debug)]
pub struct Manifest {
    pub name: &'static str,
    pub version: &'static str,
    pub updates: Option<nd_update::UpdateConfig>,
}

#[macro_export]
macro_rules! manifest {
    () => {
        $crate::Manifest {
            name: env!("CARGO_PKG_NAME"),
            version: env!("CARGO_PKG_VERSION"),
            updates: $crate::update::build_config!(),
        }
    };
}

/// Release notes of the update this launch is the result of, until [`mark_whats_new_seen`].
#[cfg(not(target_arch = "wasm32"))]
pub fn whats_new(manifest: &Manifest) -> Option<nd_update::WhatsNew> {
    nd_update::whats_new(&update_state_dir(manifest), manifest.version)
}

#[cfg(not(target_arch = "wasm32"))]
pub fn mark_whats_new_seen(manifest: &Manifest) {
    nd_update::mark_whats_new_seen(&update_state_dir(manifest), manifest.version);
}

#[cfg(not(target_arch = "wasm32"))]
fn update_state_dir(manifest: &Manifest) -> std::path::PathBuf {
    nd_core::paths::app_data_dir(manifest.name).join("updates")
}

/// Starts the background check of a packaged app: first after a short delay (`ND_UPDATE_DELAY_MS`,
/// default 10 s), then every six hours until an update is downloaded, which then applies when
/// the app quits.
#[cfg(target_arch = "wasm32")]
fn start_updates(_manifest: &Manifest) {}

#[cfg(not(target_arch = "wasm32"))]
fn start_updates(manifest: &Manifest) {
    nd_update::startup();
    let Some(config) = manifest.updates else { return };
    match nd_update::Updater::new(&config, &update_state_dir(manifest), None) {
        Ok(updater) => {
            let delay = std::env::var("ND_UPDATE_DELAY_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(10_000);
            updater.spawn_background(
                std::time::Duration::from_millis(delay),
                std::time::Duration::from_secs(6 * 3600),
                |update| eprintln!("nd: update {} ready, applies on quit", update.version),
                |error| eprintln!("nd: {error}"),
            );
        }
        Err(nd_update::Unavailable::NotInstalled(_)) => {}
        Err(why) => eprintln!("nd: updates off: {why}"),
    }
}

/// Writes a `-report` incident bundle with `message` and returns its directory, or `None` when
/// diagnostics are not running, which is always the case on the web.
#[nd_macros::unavailable(web)]
pub fn report_issue(message: &str) -> Option<std::path::PathBuf> {
    nd_diag::global()?.report_issue(message).ok()
}

pub struct Application {
    manifest: Manifest,
    pub(crate) theme: Option<crate::theme::ThemeOverride>,
    pub(crate) patterns: crate::chrome::Patterns,
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
        let sender = host.sender();
        sources::watch(&watched, move |job| sender.send(job));
        host
    }

    /// Runs the app on the platform host and returns when it quits. Where no host is linked
    /// yet (everything but macOS until M2) this returns [`Error::NoBackend`] without calling
    /// `init`. Diagnostics (logs, incident bundles, crash and hang capture) start first; in the
    /// copy of the executable launched as the crash helper this never returns.
    pub fn run<R>(self, init: impl FnOnce(&mut nd_ui::App) -> R + 'static) -> Result {
        #[cfg(all(target_os = "macos", feature = "gpui"))]
        if std::env::var_os("ND_BACKEND").is_some_and(|v| v == "gpui") {
            return self.run_gpui(init);
        }
        #[cfg(target_os = "macos")]
        {
            nd_diag::init(nd_diag::Config::new(self.manifest.name, self.manifest.version, "appkit"));
            start_updates(&self.manifest);
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
                    sources::watch(&watched, move |job| sender.send(job));
                },
            )
        }
        #[cfg(not(target_os = "macos"))]
        self.run_gpui(init)
    }

    #[cfg(any(not(target_os = "macos"), feature = "gpui"))]
    fn run_gpui<R>(self, init: impl FnOnce(&mut nd_ui::App) -> R + 'static) -> Result {
        nd_diag::init(nd_diag::Config::new(self.manifest.name, self.manifest.version, "gpui"));
        start_updates(&self.manifest);
        let files = crate::theme::ThemeFiles::for_app(self.manifest.name);
        let watched = files.clone();
        let app_theme = self.theme.unwrap_or_default();
        let patterns = self.patterns;
        #[cfg(target_os = "linux")]
        let hooks = nd_os_linux::LinuxHooks::new(self.manifest.name);
        #[cfg(target_arch = "wasm32")]
        let hooks = nd_os_web::WebHooks;
        #[cfg(not(any(target_os = "linux", target_arch = "wasm32")))]
        let hooks = nd_backend_gpui::NoOsHooks;
        nd_backend_gpui::run_with(
            hooks,
            move |cx| {
                gpui_host::install(cx, &files, app_theme, patterns);
                if let Some(backend) = cx.backend_as::<nd_backend_gpui::GpuiBackend>() {
                    let chrome = nd_backend_gpui::GpuiChrome::new(backend.clone());
                    cx.set_global(nd_components::Chrome(chrome));
                }
                init(cx);
                crate::theme::sync(cx);
            },
            move |sender| {
                let jobs = sender.clone();
                if let Err(e) = nd_automation::start_from_env(move |job| jobs.send(job)) {
                    eprintln!("nd: automation socket: {e}");
                }
                // A browser tab has no theme files to watch.
                #[cfg(not(target_arch = "wasm32"))]
                sources::watch(&watched, move |job| sender.send(job));
                #[cfg(target_arch = "wasm32")]
                let _ = (watched, sender);
            },
        );
        Ok(())
    }
}

/// The theme registry and services of an app on the GPUI backend.
#[cfg(any(not(target_os = "macos"), feature = "gpui"))]
pub(crate) mod gpui_host {
    use nd_backend_gpui::GpuiBackend;
    use nd_theme::{OsTheme, Preset, ThemeFiles, ThemeRefinement, ThemeRegistry};
    use nd_ui::App;

    use crate::chrome::Patterns;
    use crate::theme::{ThemeState, update_registry};

    /// The registry from the platform's light or dark setting and the app's files, kept
    /// following the platform as it changes.
    pub fn install(cx: &mut App, files: &ThemeFiles, app_theme: ThemeRefinement, patterns: Patterns) {
        let preset = if cfg!(target_os = "macos") { Preset::Adwaita } else { Preset::current() };
        let mut registry = ThemeRegistry::new(preset);
        if let Some(backend) = cx.backend_as::<GpuiBackend>() {
            registry.set_os(OsTheme { scheme: backend.platform_scheme(), ..OsTheme::default() });
            backend.on_platform_scheme(|app, scheme| {
                update_registry(app, |r| r.set_os(OsTheme { scheme: Some(scheme), ..r.os().clone() }));
            });
        }
        registry.set_app_override(app_theme);
        super::sources::load(&mut registry, files);
        cx.set_global(ThemeState(registry));
        cx.set_global(patterns);
        // The web's chrome: drawn menu bar and alerts, browser pickers. It keeps the GPUI
        // backend's own chrome, if one is installed, for what both draw alike.
        #[cfg(target_arch = "wasm32")]
        if let Some(backend) = cx.backend_as::<GpuiBackend>() {
            let inner = cx.try_global::<nd_components::Chrome>().map(|c| c.0.clone());
            cx.set_global(nd_components::Chrome(nd_os_web::WebChrome::new(backend.clone(), inner)));
        }
    }
}

/// The palette sources every backend shares: the app's `theme.toml` and matugen `theme.json`,
/// read at launch and watched for live reload.
mod sources {
    #![cfg_attr(target_arch = "wasm32", allow(unused_imports))]
    use nd_theme::{MatugenTheme, ThemeFileWatcher, ThemeFiles, ThemeRegistry, theme_toml};
    use nd_ui::App;

    use crate::theme::update_registry;

    #[cfg(not(target_arch = "wasm32"))]
    pub type Job = Box<dyn FnOnce(&mut App) + Send>;

    #[cfg(not(target_arch = "wasm32"))]
    struct Watchers(#[allow(dead_code)] Vec<ThemeFileWatcher>);

    fn read<T, E: std::fmt::Display>(
        path: Option<&std::path::Path>,
        parse: impl Fn(&str) -> Result<T, E>,
    ) -> Option<T> {
        let path = path?;
        let text = std::fs::read_to_string(path).ok()?;
        parse(&text).map_err(|e| eprintln!("nd: {}: {e}", path.display())).ok()
    }

    /// Loads both files into `registry` once.
    pub fn load(registry: &mut ThemeRegistry, files: &ThemeFiles) {
        if let Some(user) = read(files.toml.as_deref(), theme_toml::parse) {
            registry.set_user(user);
        }
        registry.set_matugen(read(files.matugen.as_deref(), MatugenTheme::parse));
    }

    /// `theme.toml` and `theme.json` reload live: each new parseable version reaches the app on
    /// the main thread through `send` and recolours it in the next frame.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn watch(files: &ThemeFiles, send: impl Fn(Job) + Clone + Send + 'static) {
        let mut watchers = Vec::new();
        if let Some(path) = &files.toml {
            let send = send.clone();
            let watcher = ThemeFileWatcher::spawn(path, theme_toml::parse, move |theme| {
                send(Box::new(move |app| update_registry(app, |r| r.set_user(theme))));
            });
            watchers.extend(watcher.map_err(|e| eprintln!("nd: watching {}: {e}", path.display())).ok());
        }
        if let Some(path) = &files.matugen {
            let send = send.clone();
            let watcher = ThemeFileWatcher::spawn(path, MatugenTheme::parse, move |theme| {
                send(Box::new(move |app| update_registry(app, |r| r.set_matugen(Some(theme)))));
            });
            watchers.extend(watcher.map_err(|e| eprintln!("nd: watching {}: {e}", path.display())).ok());
        }
        send(Box::new(move |app| app.set_global(Watchers(watchers))));
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use std::rc::Rc;

    use nd_backend_appkit::objc2_foundation::MainThreadMarker;
    use nd_backend_appkit::{AppKitBackend, AppKitChrome, AppearanceObserver, os_theme};
    use nd_theme::{Preset, ThemeFiles, ThemeRefinement, ThemeRegistry};
    use nd_ui::App;

    use crate::chrome::{Chrome, Patterns};
    use crate::theme::{ThemeState, update_registry};

    /// Keeps the appearance observer and the file watchers alive with the app.
    struct ThemeSources {
        _observer: AppearanceObserver,
    }

    /// The theme registry from the OS and the app's files, the chrome services, the default
    /// menu bar, and the observer that re-reads the OS on an appearance or accent change.
    pub fn install(cx: &mut App, files: &ThemeFiles, app_theme: ThemeRefinement, patterns: Patterns) {
        let mtm = MainThreadMarker::new().expect("main thread");
        let mut registry = ThemeRegistry::new(Preset::MacOs);
        registry.set_os(os_theme(mtm));
        registry.set_app_override(app_theme);
        super::sources::load(&mut registry, files);
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
}
