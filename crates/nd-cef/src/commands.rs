//! Chrome commands (`CefCommandHandler::OnChromeCommand`) of a Chrome-style page. Its browser UI
//! is hidden, so the commands that would show it (a new tab or window, history, downloads,
//! settings) go to the app as [`nd_webview::ChromeCommand`]; the ones that act on the page itself
//! run; every other one is refused, since it would open Chrome UI the app does not have.
//!
//! Command ids move between Chromium versions, so they are looked up by their `IDC_` name at run
//! time, as `cef_command_ids.h` asks.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::CString;

use nd_webview::BrowserCommand;

/// What a page does with a command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Route {
    /// Chromium runs it on the page.
    Allow,
    /// The app hears it; Chromium does nothing.
    App(BrowserCommand),
}

/// Commands that act on the page alone.
const ALLOWED: &[&str] = &[
    "IDC_BACK",
    "IDC_FORWARD",
    "IDC_RELOAD",
    "IDC_RELOAD_BYPASSING_CACHE",
    "IDC_RELOAD_CLEARING_CACHE",
    "IDC_STOP",
    "IDC_ZOOM_PLUS",
    "IDC_ZOOM_NORMAL",
    "IDC_ZOOM_MINUS",
    "IDC_CUT",
    "IDC_COPY",
    "IDC_PASTE",
    "IDC_CLOSE_FIND_OR_STOP",
    "IDC_DEV_TOOLS",
    "IDC_DEV_TOOLS_CONSOLE",
    "IDC_DEV_TOOLS_DEVICES",
    "IDC_DEV_TOOLS_INSPECT",
    "IDC_DEV_TOOLS_TOGGLE",
];

/// macOS prints through the system panel (`disable-print-preview`); Chromium's print preview
/// elsewhere is browser UI.
#[cfg(target_os = "macos")]
const PRINT: Route = Route::Allow;
#[cfg(not(target_os = "macos"))]
const PRINT: Route = Route::App(BrowserCommand::Print);

fn routed() -> Vec<(&'static str, Route)> {
    use BrowserCommand::*;
    let app = |name, command| (name, Route::App(command));
    vec![
        app("IDC_NEW_TAB", NewTab),
        app("IDC_NEW_TAB_TO_RIGHT", NewTab),
        app("IDC_NEW_WINDOW", NewWindow),
        app("IDC_NEW_INCOGNITO_WINDOW", NewPrivateWindow),
        app("IDC_RESTORE_TAB", ReopenClosedTab),
        app("IDC_CLOSE_TAB", CloseTab),
        app("IDC_CLOSE_WINDOW", CloseWindow),
        app("IDC_SELECT_NEXT_TAB", NextTab),
        app("IDC_SELECT_PREVIOUS_TAB", PreviousTab),
        app("IDC_SHOW_HISTORY", History),
        app("IDC_SHOW_DOWNLOADS", Downloads),
        app("IDC_SHOW_BOOKMARK_MANAGER", Bookmarks),
        app("IDC_BOOKMARK_THIS_TAB", BookmarkPage),
        app("IDC_TAB_SEARCH", TabSearch),
        app("IDC_OPTIONS", Settings),
        app("IDC_MANAGE_EXTENSIONS", Extensions),
        app("IDC_CLEAR_BROWSING_DATA", ClearBrowsingData),
        app("IDC_SAVE_PAGE", SavePage),
        app("IDC_VIEW_SOURCE", ViewSource),
        app("IDC_OPEN_FILE", OpenFile),
        app("IDC_FIND", Find),
        app("IDC_FIND_NEXT", FindNext),
        app("IDC_FIND_PREVIOUS", FindPrevious),
        app("IDC_FOCUS_LOCATION", FocusAddress),
        app("IDC_FOCUS_SEARCH", FocusAddress),
        app("IDC_FULLSCREEN", Fullscreen),
        app("IDC_HOME", Home),
        app("IDC_TASK_MANAGER", TaskManager),
        app("IDC_TASK_MANAGER_SHORTCUT", TaskManager),
        app("IDC_EXIT", Quit),
        ("IDC_PRINT", PRINT),
        ("IDC_BASIC_PRINT", PRINT),
    ]
}

thread_local! {
    static TABLE: RefCell<Option<HashMap<i32, Route>>> = const { RefCell::new(None) };
    /// The command of the context menu item being run: Chromium hands it to `OnChromeCommand`
    /// too, after `OnContextMenuCommand`, and the page's own menu items run.
    static MENU_COMMAND: Cell<Option<i32>> = const { Cell::new(None) };
}

fn id_for(name: &str) -> Option<i32> {
    let name = CString::new(name).ok()?;
    // SAFETY: a NUL-terminated name that outlives the call; CEF returns 0 for unknown names.
    let id = unsafe { cef::sys::cef_id_for_command_id_name(name.as_ptr()) };
    (id != 0).then_some(id)
}

fn table() -> HashMap<i32, Route> {
    let mut table = HashMap::new();
    for name in ALLOWED {
        if let Some(id) = id_for(name) {
            table.insert(id, Route::Allow);
        }
    }
    for (name, route) in routed() {
        if let Some(id) = id_for(name) {
            table.insert(id, route);
        }
    }
    table
}

/// Where `id` goes. `None`: refused.
pub(crate) fn route(id: i32) -> Option<Route> {
    if MENU_COMMAND.with(Cell::get) == Some(id) {
        return Some(Route::Allow);
    }
    TABLE.with(|t| t.borrow_mut().get_or_insert_with(table).get(&id).copied())
}

/// Runs `f` (which runs context menu command `id`) with that command allowed through.
pub(crate) fn running_menu_command(id: i32, f: impl FnOnce()) {
    let outer = MENU_COMMAND.with(|m| m.replace(Some(id)));
    f();
    MENU_COMMAND.with(|m| m.set(outer));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_routed_name_is_an_idc_name_once() {
        let mut names: Vec<&str> = routed().into_iter().map(|(n, _)| n).chain(ALLOWED.iter().copied()).collect();
        assert!(names.iter().all(|n| n.starts_with("IDC_")));
        let len = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), len, "a command is listed twice");
    }

    #[test]
    fn the_chrome_ui_commands_reach_the_app() {
        let routes: HashMap<&str, Route> = routed().into_iter().collect();
        for (name, command) in [
            ("IDC_SHOW_DOWNLOADS", BrowserCommand::Downloads),
            ("IDC_SHOW_HISTORY", BrowserCommand::History),
            ("IDC_SHOW_BOOKMARK_MANAGER", BrowserCommand::Bookmarks),
            ("IDC_TAB_SEARCH", BrowserCommand::TabSearch),
            ("IDC_NEW_TAB", BrowserCommand::NewTab),
        ] {
            assert_eq!(routes.get(name), Some(&Route::App(command)), "{name}");
        }
    }

    #[test]
    fn a_running_menu_command_is_allowed_once() {
        running_menu_command(-12345, || assert_eq!(route(-12345), Some(Route::Allow)));
        assert_eq!(MENU_COMMAND.with(Cell::get), None);
    }
}
