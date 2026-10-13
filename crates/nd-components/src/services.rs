//! What the chrome intents need from the platform beyond the retained tree: menus, alerts and
//! pickers, which are imperative. Each backend installs one [`ChromeServices`] as an app
//! global; without one (tests, a backend that has none yet) every call reports `false` or
//! answers with the cancel result.

use std::path::PathBuf;
use std::rc::Rc;

use nd_core::{NodeId, WindowId};
use nd_ui::App;

use crate::dialogs::{Alert, AlertResponse, OpenDialog, SaveDialog};
use crate::menu::{ContextMenu, Menu};

pub type AlertCallback = Box<dyn FnOnce(AlertResponse, &mut App)>;
pub type OpenCallback = Box<dyn FnOnce(Option<Vec<PathBuf>>, &mut App)>;
pub type SaveCallback = Box<dyn FnOnce(Option<PathBuf>, &mut App)>;

pub trait ChromeServices {
    /// Replaces the app's menu bar. Platforms without one fold it into the command palette.
    fn set_menus(&self, cx: &mut App, menus: Vec<Menu>);
    /// Pops `menu` up over `node` at `(x, y)` in the node's coordinates. Its items dispatch
    /// their actions from the window's focused node, as a key binding would.
    fn show_context_menu(&self, cx: &mut App, window: WindowId, node: NodeId, x: f32, y: f32, menu: ContextMenu);
    /// Shows `alert` attached to `window` (a sheet on macOS).
    fn alert(&self, cx: &mut App, window: WindowId, alert: Alert, done: AlertCallback);
    fn open_dialog(&self, cx: &mut App, window: WindowId, dialog: OpenDialog, done: OpenCallback);
    fn save_dialog(&self, cx: &mut App, window: WindowId, dialog: SaveDialog, done: SaveCallback);
}

/// The app's [`ChromeServices`], as a global.
#[derive(Clone)]
pub struct Chrome(pub Rc<dyn ChromeServices>);

fn services(cx: &App) -> Option<Rc<dyn ChromeServices>> {
    cx.try_global::<Chrome>().map(|c| c.0.clone())
}

pub fn set_menus(cx: &mut App, menus: impl IntoIterator<Item = Menu>) {
    if let Some(s) = services(cx) {
        s.set_menus(cx, menus.into_iter().collect());
    }
}

pub fn show_context_menu(cx: &mut App, window: WindowId, node: NodeId, x: f32, y: f32, menu: ContextMenu) {
    if let Some(s) = services(cx) {
        s.show_context_menu(cx, window, node, x, y, menu);
    }
}

pub fn alert(cx: &mut App, window: WindowId, alert: Alert, done: impl FnOnce(AlertResponse, &mut App) + 'static) {
    match services(cx) {
        Some(s) => s.alert(cx, window, alert, Box::new(done)),
        None => done(AlertResponse(0), cx),
    }
}

pub fn open_dialog(
    cx: &mut App,
    window: WindowId,
    dialog: OpenDialog,
    done: impl FnOnce(Option<Vec<PathBuf>>, &mut App) + 'static,
) {
    match services(cx) {
        Some(s) => s.open_dialog(cx, window, dialog, Box::new(done)),
        None => done(None, cx),
    }
}

pub fn save_dialog(
    cx: &mut App,
    window: WindowId,
    dialog: SaveDialog,
    done: impl FnOnce(Option<PathBuf>, &mut App) + 'static,
) {
    match services(cx) {
        Some(s) => s.save_dialog(cx, window, dialog, Box::new(done)),
        None => done(None, cx),
    }
}
