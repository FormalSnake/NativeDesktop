//! What the chrome intents need from the platform beyond the retained tree: menus, alerts and
//! pickers, which are imperative. Each backend installs one [`ChromeServices`] as an app
//! global; without one (tests, a backend that has none yet) every call reports `false` or
//! answers with the cancel result.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::rc::Rc;

use nd_core::{NodeId, WindowId};
use nd_ui::App;

use crate::dialogs::{AboutPanel, Alert, AlertPresentation, AlertResponse, OpenDialog, SaveDialog};
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
    /// Shows `alert` attached to `window` (a sheet on macOS), or in front of the app when its
    /// presentation is modal. Called with at most one alert showing per window and one modal
    /// alert per app; [`alert`] queues the rest.
    fn alert(&self, cx: &mut App, window: WindowId, alert: Alert, done: AlertCallback);
    /// Shows the app's about panel, over `window` where the platform attaches it to one.
    fn about(&self, _cx: &mut App, _window: Option<WindowId>, _about: AboutPanel) {}
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
    let menus: Vec<Menu> = menus.into_iter().collect();
    crate::menu::remember_app_menus(&menus);
    if let Some(s) = services(cx) {
        s.set_menus(cx, menus);
    }
}

pub fn show_context_menu(cx: &mut App, window: WindowId, node: NodeId, x: f32, y: f32, menu: ContextMenu) {
    if let Some(s) = services(cx) {
        s.show_context_menu(cx, window, node, x, y, menu);
    }
}

/// Alerts waiting for the one showing on the same window (or, modal, on the app) to close, so
/// none is lost and each answer reaches its own callback.
#[derive(Default)]
struct AlertQueue {
    showing: HashSet<Option<WindowId>>,
    waiting: HashMap<Option<WindowId>, VecDeque<(WindowId, Alert, AlertCallback)>>,
}

#[derive(Clone, Default)]
struct Alerts(Rc<RefCell<AlertQueue>>);

fn alerts(cx: &mut App) -> Rc<RefCell<AlertQueue>> {
    if !cx.has_global::<Alerts>() {
        cx.set_global(Alerts::default());
    }
    cx.global::<Alerts>().0.clone()
}

/// Shows `alert` on `window`. While another alert shows there (or, for a modal alert, anywhere in
/// the app) it waits its turn; `done` runs with the button the user picked.
pub fn alert(cx: &mut App, window: WindowId, alert: Alert, done: impl FnOnce(AlertResponse, &mut App) + 'static) {
    let Some(s) = services(cx) else { return done(AlertResponse(0), cx) };
    let slot = match alert.presentation {
        AlertPresentation::Sheet => Some(window),
        AlertPresentation::Modal => None,
    };
    let queue = alerts(cx);
    if queue.borrow().showing.contains(&slot) {
        queue.borrow_mut().waiting.entry(slot).or_default().push_back((window, alert, Box::new(done)));
        return;
    }
    show_alert(cx, s, slot, window, alert, Box::new(done));
}

fn show_alert(
    cx: &mut App,
    s: Rc<dyn ChromeServices>,
    slot: Option<WindowId>,
    window: WindowId,
    alert: Alert,
    done: AlertCallback,
) {
    let queue = alerts(cx);
    queue.borrow_mut().showing.insert(slot);
    let next = s.clone();
    let finish: AlertCallback = Box::new(move |response, cx| {
        done(response, cx);
        queue.borrow_mut().showing.remove(&slot);
        loop {
            let Some((window, alert, done)) = queue.borrow_mut().waiting.get_mut(&slot).and_then(VecDeque::pop_front)
            else {
                return;
            };
            // A window that closed while its alert waited answers it as dismissed.
            if cx.window(window).is_none() {
                let cancel = alert.cancel_index();
                done(AlertResponse(cancel), cx);
                continue;
            }
            show_alert(cx, next, slot, window, alert, done);
            return;
        }
    });
    s.alert(cx, window, alert, finish);
}

/// Shows the app's about panel. On macOS it is the standard panel, filled from the bundle where
/// `about` leaves a field empty.
pub fn about(cx: &mut App, window: Option<WindowId>, about: AboutPanel) {
    if let Some(s) = services(cx) {
        s.about(cx, window, about);
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
