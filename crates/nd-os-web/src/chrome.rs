//! [`ChromeServices`] on the web. Menus are drawn by the single canvas (`GpuiBackend::
//! set_menu_bar`), alerts are drawn sheets, pickers are the browser's ([`crate::files`]).
//! Context menus are the GPUI backend's drawn ones, reached through the chrome this one wraps.

use std::cell::RefCell;
use std::rc::Rc;

use nd_backend_gpui::GpuiBackend;
use nd_components::services::{AlertCallback, OpenCallback, SaveCallback};
use nd_components::{Alert, AlertResponse, ButtonRole, ChromeServices, ContextMenu, Menu, OpenDialog, SaveDialog};
use nd_core::{NodeId, WindowId};
use nd_ui::WindowKind;
use nd_ui::prelude::*;

pub struct WebChrome {
    backend: GpuiBackend,
    /// The GPUI backend's own chrome, for what the web draws the same way (context menus).
    inner: Option<Rc<dyn ChromeServices>>,
}

impl WebChrome {
    pub fn new(backend: GpuiBackend, inner: Option<Rc<dyn ChromeServices>>) -> Rc<Self> {
        Rc::new(Self { backend, inner })
    }
}

impl ChromeServices for WebChrome {
    fn set_menus(&self, _cx: &mut App, menus: Vec<Menu>) {
        self.backend.set_menu_bar(menus);
    }

    fn show_context_menu(&self, cx: &mut App, window: WindowId, node: NodeId, x: f32, y: f32, menu: ContextMenu) {
        if let Some(inner) = &self.inner {
            inner.show_context_menu(cx, window, node, x, y, menu);
        }
    }

    fn alert(&self, cx: &mut App, window: WindowId, alert: Alert, done: AlertCallback) {
        let options = WindowOptions::main("nd.alert")
            .kind(WindowKind::Sheet { parent: window })
            .size(size(px(360.), px(if alert.message.is_some() { 180. } else { 140. })));
        cx.open_window(options, move |_, cx| cx.new(|_| AlertSheet { alert, done: RefCell::new(Some(done)) }));
    }

    fn open_dialog(&self, _cx: &mut App, _window: WindowId, dialog: OpenDialog, done: OpenCallback) {
        crate::files::open(&self.backend, &dialog, done);
    }

    fn save_dialog(&self, _cx: &mut App, _window: WindowId, dialog: SaveDialog, done: SaveCallback) {
        crate::files::save(&self.backend, &dialog, done);
    }
}

/// An alert drawn as a sheet: title, message, and its buttons trailing, the default last.
struct AlertSheet {
    alert: Alert,
    done: RefCell<Option<AlertCallback>>,
}

impl AlertSheet {
    fn answer(&self, index: usize, window: &mut Window, cx: &mut App) {
        if let Some(done) = self.done.borrow_mut().take() {
            done(AlertResponse(index), cx);
        }
        cx.close_window(window.id());
    }
}

impl Render for AlertSheet {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let labels: Vec<(usize, SharedString, ButtonRole)> = if self.alert.buttons.is_empty() {
            vec![(0, "OK".into(), ButtonRole::Default)]
        } else {
            self.alert.buttons.iter().enumerate().map(|(i, b)| (i, b.label.clone(), b.role)).collect()
        };
        let mut row = h_flex().gap_2().justify_end();
        for (index, label, _role) in labels.into_iter().rev() {
            row = row.child(button(format!("nd-alert-{index}"), label).on_click(
                cx.listener(move |this: &mut Self, _: &ClickEvent, window, cx| this.answer(index, window, cx)),
            ));
        }
        v_flex()
            .size_full()
            .p_4()
            .gap_2()
            .child(text(self.alert.title.clone()).font_weight(FontWeight::SEMIBOLD))
            .children(self.alert.message.clone().map(text))
            .child(div().flex_1())
            .child(row)
    }
}
