//! The chrome intents apps declare (ARCHITECTURE.md 3.6): `scaffold`, `toolbar`, `sidebar`,
//! `inspector`, `titlebar`, `glass`, from `nd-components`, plus the imperative ones on a window:
//! sheets, the settings window, alerts, confirmations, the about panel, file pickers, window
//! tabs and the app's menu bar.

use std::path::PathBuf;

pub use nd_components::services::{about, alert, open_dialog, save_dialog, set_menus};
pub use nd_components::*;
use nd_ui::prelude::*;
use nd_ui::{WindowId, WindowKind};

/// The size a sheet opens at when the app names none.
fn sheet_size() -> Size<Pixels> {
    size(px(480.), px(360.))
}

fn sheets_of(cx: &App, parent: WindowId) -> Vec<WindowId> {
    cx.window_ids()
        .filter(|id| cx.window(*id).is_some_and(|w| w.options().kind == WindowKind::Sheet { parent }))
        .collect()
}

/// Closes `window` after the current handler returns, outside every native callback.
fn close_later(cx: &mut App, window: WindowId) {
    #[cfg(target_os = "macos")]
    {
        let _ = cx;
        nd_backend_appkit::defer_with_app(move |app| app.close_window(window));
    }
    #[cfg(not(target_os = "macos"))]
    cx.close_window(window);
}

use nd_gpui::gpui::Size;

/// Sheets, settings, alerts and pickers on a window.
pub trait WindowChromeExt {
    /// Opens `build`'s view as a sheet on this window: window-modal and attached on macOS, a
    /// dialog with a scrim elsewhere.
    fn open_sheet<V: Render>(
        &mut self,
        cx: &mut App,
        build: impl FnOnce(&mut Window, &mut App) -> Entity<V>,
    ) -> WindowHandle<V>;

    /// As [`WindowChromeExt::open_sheet`], at `size`.
    fn open_sheet_sized<V: Render>(
        &mut self,
        cx: &mut App,
        size: Size<Pixels>,
        build: impl FnOnce(&mut Window, &mut App) -> Entity<V>,
    ) -> WindowHandle<V>;

    /// Closes this window if it is a sheet, else every sheet open on it.
    fn close_sheet(&mut self, cx: &mut App);

    fn alert(&self, cx: &mut App, alert: Alert, done: impl FnOnce(AlertResponse, &mut App) + 'static);

    /// Asks before a destructive action: `action` in the platform's destructive style and a
    /// Cancel. `done` hears true when the user confirmed.
    fn confirm(
        &self,
        cx: &mut App,
        title: impl Into<SharedString>,
        message: Option<SharedString>,
        action: impl Into<SharedString>,
        done: impl FnOnce(bool, &mut App) + 'static,
    );

    /// Shows the app's about panel over this window.
    fn show_about(&self, cx: &mut App, about: AboutPanel);

    fn open_dialog(
        &self,
        cx: &mut App,
        dialog: OpenDialog,
        done: impl FnOnce(Option<Vec<PathBuf>>, &mut App) + 'static,
    );

    fn save_dialog(&self, cx: &mut App, dialog: SaveDialog, done: impl FnOnce(Option<PathBuf>, &mut App) + 'static);
}

impl WindowChromeExt for Window {
    fn open_sheet<V: Render>(
        &mut self,
        cx: &mut App,
        build: impl FnOnce(&mut Window, &mut App) -> Entity<V>,
    ) -> WindowHandle<V> {
        self.open_sheet_sized(cx, sheet_size(), build)
    }

    fn open_sheet_sized<V: Render>(
        &mut self,
        cx: &mut App,
        size: Size<Pixels>,
        build: impl FnOnce(&mut Window, &mut App) -> Entity<V>,
    ) -> WindowHandle<V> {
        let parent = self.id();
        let options =
            WindowOptions::main(format!("{}.sheet", self.options().id)).kind(WindowKind::Sheet { parent }).size(size);
        cx.open_window(options, build)
    }

    fn close_sheet(&mut self, cx: &mut App) {
        if matches!(self.options().kind, WindowKind::Sheet { .. }) {
            close_later(cx, self.id());
            return;
        }
        for sheet in sheets_of(cx, self.id()) {
            close_later(cx, sheet);
        }
    }

    fn alert(&self, cx: &mut App, alert: Alert, done: impl FnOnce(AlertResponse, &mut App) + 'static) {
        nd_components::services::alert(cx, self.id(), alert, done);
    }

    fn confirm(
        &self,
        cx: &mut App,
        title: impl Into<SharedString>,
        message: Option<SharedString>,
        action: impl Into<SharedString>,
        done: impl FnOnce(bool, &mut App) + 'static,
    ) {
        let mut alert = Alert::confirm(title, action);
        alert.message = message;
        nd_components::services::alert(cx, self.id(), alert, move |r, cx| done(r.0 == 0, cx));
    }

    fn show_about(&self, cx: &mut App, about: AboutPanel) {
        nd_components::services::about(cx, Some(self.id()), about);
    }

    fn open_dialog(
        &self,
        cx: &mut App,
        dialog: OpenDialog,
        done: impl FnOnce(Option<Vec<PathBuf>>, &mut App) + 'static,
    ) {
        nd_components::services::open_dialog(cx, self.id(), dialog, done);
    }

    fn save_dialog(&self, cx: &mut App, dialog: SaveDialog, done: impl FnOnce(Option<PathBuf>, &mut App) + 'static) {
        nd_components::services::save_dialog(cx, self.id(), dialog, done);
    }
}

/// Opens the app's settings window, or brings the open one forward. On macOS it is a
/// preference-style window (toolbar tabs, `cmd-,` from the app menu when the app binds it).
pub fn open_settings<V: Render>(
    cx: &mut App,
    build: impl FnOnce(&mut Window, &mut App) -> Entity<V>,
) -> Option<WindowHandle<V>> {
    let open = cx.window_ids().find(|id| cx.window(*id).is_some_and(|w| w.options().kind == WindowKind::Settings));
    if let Some(id) = open {
        #[cfg(target_os = "macos")]
        if let Some(window) = cx.backend_as::<nd_backend_appkit::AppKitBackend>().and_then(|b| b.ns_window(id)) {
            window.makeKeyAndOrderFront(None);
        }
        let _ = id;
        return None;
    }
    let options =
        WindowOptions::main("settings").title("Settings").kind(WindowKind::Settings).size(size(px(560.), px(420.)));
    Some(cx.open_window(options, build))
}

/// Opens `build`'s view as a new tab of window tab group `group` (ARCHITECTURE.md 5.5): a
/// native window tab on macOS, a tab in the group window's tab bar elsewhere. The tab joins the
/// group's key window and is selected.
pub fn open_tab<V: Render>(
    cx: &mut App,
    group: impl Into<SharedString>,
    build: impl FnOnce(&mut Window, &mut App) -> Entity<V>,
) -> WindowHandle<V> {
    let group = group.into();
    let options = WindowOptions::main(format!("{group}.tab")).tab_group(group);
    cx.open_window(options, build)
}
