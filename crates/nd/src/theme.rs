//! Theming for apps (ARCHITECTURE.md 4): `cx.theme()`, the app and window override layers, and
//! the live sources (`theme.toml`, matugen `theme.json`, the OS appearance) wired to the
//! running app. Subtree overrides are `.theme(..)` on a container (`SubtreeOverrides`).

use std::sync::LazyLock;

pub use nd_theme::*;
use nd_ui::{App, Window, WindowId};

/// The app's theme registry, as a global.
pub struct ThemeState(pub ThemeRegistry);

static DEFAULT: LazyLock<Theme> = LazyLock::new(|| ThemeRegistry::default().app_theme().clone());

pub trait ActiveTheme {
    /// The tokens of the window being rendered (its override applied), or the app's outside
    /// any window.
    fn theme(&self) -> &Theme;
}

impl ActiveTheme for App {
    fn theme(&self) -> &Theme {
        let Some(state) = self.try_global::<ThemeState>() else { return &DEFAULT };
        match self.current_window() {
            Some(window) => state.0.window_theme(window),
            None => state.0.app_theme(),
        }
    }
}

/// Runs `f` on the registry, then pushes the result to the backend and re-renders every view
/// when anything changed.
pub fn update_registry(cx: &mut App, f: impl FnOnce(&mut ThemeRegistry)) {
    if !cx.has_global::<ThemeState>() {
        cx.set_global(ThemeState(ThemeRegistry::default()));
    }
    let Some(state) = cx.try_global::<ThemeState>() else { return };
    let mut registry = state.0.clone();
    let before = registry.generation();
    f(&mut registry);
    if registry.generation() == before {
        return;
    }
    cx.set_global(ThemeState(registry));
    sync(cx);
}

/// Pushes what native controls need to the backend and marks every view for re-render.
pub(crate) fn sync(cx: &mut App) {
    let reduce = cx.try_global::<ThemeState>().is_some_and(|s| s.0.os().metrics.reduce_motion == Some(true));
    cx.set_reduce_motion(reduce);
    // Text measures in the theme's fonts.
    cx.remeasure();
    #[cfg(target_os = "macos")]
    if let Some(state) = cx.try_global::<ThemeState>() {
        let registry = &state.0;
        // A window being built (its override set inside `open_window`) is not listed by the app
        // yet; the registry knows it.
        let mut windows: Vec<WindowId> = cx.window_ids().collect();
        windows.extend(registry.overridden_windows().filter(|w| !windows.contains(w)).collect::<Vec<_>>());
        let natives: Vec<(WindowId, NativeTheme)> = windows.iter().map(|w| (*w, registry.window_native(*w))).collect();
        let app = registry.app_native();
        let system = registry.system_scheme();
        if let Some(backend) = cx.backend_as::<nd_backend_appkit::AppKitBackend>() {
            backend.set_theme_registry(registry.clone());
            backend.set_system_scheme(system);
            backend.set_default_theme(app.clone());
            backend.set_app_appearance(app.appearance);
            for (window, native) in natives {
                backend.set_window_theme(window, native);
            }
        }
    }
    #[cfg(any(not(target_os = "macos"), feature = "gpui"))]
    if let Some(state) = cx.try_global::<ThemeState>()
        && let Some(backend) = cx.backend_as::<crate::gpui_backend::GpuiBackend>()
    {
        backend.set_theme(state.0.clone());
    }
    cx.refresh_windows();
}

/// Window-level theme overrides.
pub trait WindowThemeExt {
    /// Overrides the theme for this window only, over the app's.
    fn set_theme(&mut self, theme: ThemeOverride, cx: &mut App);
    /// Drops this window's override.
    fn clear_theme(&mut self, cx: &mut App);
}

impl WindowThemeExt for Window {
    fn set_theme(&mut self, theme: ThemeOverride, cx: &mut App) {
        let id = self.id();
        update_registry(cx, |r| r.set_window_override(id, Some(theme)));
    }

    fn clear_theme(&mut self, cx: &mut App) {
        let id = self.id();
        update_registry(cx, |r| r.set_window_override(id, None));
    }
}

/// App-level overrides at runtime (the builder's `Application::theme` sets the first one).
pub fn set_app_theme(cx: &mut App, theme: ThemeOverride) {
    update_registry(cx, |r| r.set_app_override(theme));
}

/// Forgets a closed window's override.
pub fn forget_window(cx: &mut App, window: WindowId) {
    update_registry(cx, |r| r.forget_window(window));
}
