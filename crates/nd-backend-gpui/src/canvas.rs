//! App windows drawn inside one GPUI window, for platforms with a single canvas (the web). Each
//! in-canvas window occludes what is under it, so a press on it never reaches the window below;
//! a sheet's scrim covers the whole canvas, as a modal dialog does.

use nd_core::Size;
use nd_gpui::gpui::prelude::FluentBuilder as _;
use nd_gpui::gpui::{
    self, AnyElement, InteractiveElement, IntoElement, ParentElement, SharedString, StatefulInteractiveElement, Styled,
    div, hsla, px,
};

use crate::render::Look;

/// Height of the drawn title row of an in-canvas window.
pub const TITLE_HEIGHT: f32 = 32.;
/// Room kept free around an in-canvas window.
const MARGIN: f32 = 24.;
/// How far each further window steps down and right from the one before.
const CASCADE: f32 = 28.;

/// `requested` shrunk to fit the canvas with a margin, leaving `chrome` for the title row.
pub fn fit(requested: Size, canvas: Size, chrome: f32) -> Size {
    let width = requested.width.min(canvas.width - 2. * MARGIN).max(1.);
    let height = requested.height.min(canvas.height - 2. * MARGIN - chrome).max(1.);
    Size { width, height }
}

/// A sheet: centred over a scrim that takes every press outside it.
pub fn sheet(size: Size, content: AnyElement, look: &Look) -> AnyElement {
    div()
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .bg(hsla(0., 0., 0., 0.35))
        .occlude()
        .child(
            div()
                .w(px(size.width))
                .h(px(size.height))
                .rounded(px(10.))
                .overflow_hidden()
                .bg(look.background)
                .border_1()
                .border_color(look.border)
                .shadow_lg()
                .occlude()
                .child(content),
        )
        .into_any_element()
}

/// A further window: a title row with the window's title and a close button over its content,
/// cascaded from the canvas's centre by `index`.
pub fn floating(
    index: usize,
    size: Size,
    canvas: Size,
    title: SharedString,
    content: AnyElement,
    look: &Look,
    close: impl Fn(&mut gpui::App) + 'static,
) -> AnyElement {
    let total = Size { width: size.width, height: size.height + TITLE_HEIGHT };
    let step = CASCADE * (index.saturating_sub(1)) as f32;
    let left = ((canvas.width - total.width) / 2. + step).clamp(0., (canvas.width - total.width).max(0.));
    let top = ((canvas.height - total.height) / 2. + step).clamp(0., (canvas.height - total.height).max(0.));
    let hover = look.control_hover;
    let title_row = div()
        .h(px(TITLE_HEIGHT))
        .px(px(12.))
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .border_b_1()
        .border_color(look.border)
        .text_color(look.text)
        .text_size(px(13.))
        .child(div().flex_1().truncate().child(title))
        .child(
            div()
                .id(("nd-canvas-close", index))
                .role(gpui::Role::Button)
                .aria_label("Close")
                .w(px(24.))
                .h(px(24.))
                .rounded(px(6.))
                .flex()
                .items_center()
                .justify_center()
                .hover(move |s| s.bg(hover))
                .child("\u{00d7}")
                .on_click(move |_, _, cx| close(cx)),
        );
    div()
        .absolute()
        .left(px(left))
        .top(px(top))
        .w(px(total.width))
        .h(px(total.height))
        .flex()
        .flex_col()
        .rounded(px(10.))
        .overflow_hidden()
        .bg(look.background)
        .border_1()
        .border_color(look.border)
        .shadow_lg()
        .occlude()
        .child(title_row)
        .child(div().w(px(size.width)).h(px(size.height)).child(content))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_keeps_a_small_window_and_shrinks_a_large_one() {
        let canvas = Size { width: 800., height: 600. };
        assert_eq!(fit(Size { width: 300., height: 200. }, canvas, TITLE_HEIGHT), Size { width: 300., height: 200. });
        let big = fit(Size { width: 2000., height: 2000. }, canvas, TITLE_HEIGHT);
        assert_eq!(big, Size { width: 800. - 2. * MARGIN, height: 600. - 2. * MARGIN - TITLE_HEIGHT });
    }
}

/// Height of the drawn menu bar above a single canvas's windows.
pub const MENU_BAR_HEIGHT: f32 = 28.;

/// One row of a drawn menu.
pub struct Entry {
    pub label: SharedString,
    pub action: Option<Box<dyn gpui::Action>>,
    pub enabled: bool,
    pub checked: bool,
}

/// The app's menus as a single canvas draws them: no OS menu bar exists there, so the bar is a
/// row above the windows and each menu a dropdown over them.
#[derive(Default)]
pub struct MenuBar {
    pub menus: Vec<(SharedString, Vec<Option<Entry>>)>,
    /// The menu whose dropdown is showing.
    pub open: Option<usize>,
}

impl MenuBar {
    /// Rows from `menus`: actions, separators as `None`, submenus inline after a separator. Items
    /// the OS implements (`MenuRole`) have no meaning in a canvas and are left out.
    pub fn set(&mut self, menus: Vec<nd_components::Menu>) {
        fn rows(items: Vec<nd_components::MenuItem>, out: &mut Vec<Option<Entry>>) {
            for item in items {
                match item {
                    nd_components::MenuItem::Action { label, action, enabled, checked, .. } => {
                        out.push(Some(Entry { label, action: Some(action), enabled, checked }));
                    }
                    nd_components::MenuItem::Separator => out.push(None),
                    nd_components::MenuItem::Submenu(menu) => {
                        out.push(None);
                        out.push(Some(Entry { label: menu.title, action: None, enabled: false, checked: false }));
                        rows(menu.items, out);
                    }
                    nd_components::MenuItem::Role(_) => {}
                }
            }
        }
        self.menus = menus
            .into_iter()
            .filter_map(|menu| {
                let mut out = Vec::new();
                rows(menu.items, &mut out);
                out.iter().any(Option::is_some).then_some((menu.title, out))
            })
            .collect();
        self.open = None;
    }

    pub fn is_empty(&self) -> bool {
        self.menus.is_empty()
    }
}

/// The bar, and the open menu's dropdown over a backdrop that closes it. `choose` gets the chosen
/// row's action; `toggle` the menu to open, or `None` to close.
pub fn menu_bar(
    bar: &MenuBar,
    look: &Look,
    toggle: impl Fn(Option<usize>, &mut gpui::Window) + Clone + 'static,
    choose: impl Fn(Box<dyn gpui::Action>, &mut gpui::Window) + Clone + 'static,
) -> (AnyElement, Option<AnyElement>) {
    const TITLE_WIDTH: f32 = 72.;
    let hover = look.control_hover;
    let mut row = div()
        .h(px(MENU_BAR_HEIGHT))
        .w_full()
        .flex()
        .flex_row()
        .items_center()
        .px(px(6.))
        .bg(look.control)
        .border_b_1()
        .border_color(look.border)
        .text_color(look.text)
        .text_size(px(13.));
    for (index, (title, _)) in bar.menus.iter().enumerate() {
        let toggle = toggle.clone();
        let open = bar.open == Some(index);
        row = row.child(
            div()
                .id(("nd-menu", index))
                .role(gpui::Role::MenuItem)
                .aria_label(title.clone())
                .px(px(10.))
                .h(px(22.))
                .rounded(px(5.))
                .flex()
                .items_center()
                .when_some(open.then_some(look.control_active), |d, bg| d.bg(bg))
                .hover(move |s| s.bg(hover))
                .child(title.clone())
                .on_click(move |_, window, _| toggle((!open).then_some(index), window)),
        );
    }
    let dropdown = bar.open.and_then(|index| bar.menus.get(index)).map(|(_, rows)| {
        let index = bar.open.unwrap_or_default();
        let mut list = div()
            .absolute()
            .top(px(MENU_BAR_HEIGHT))
            .left(px(6. + TITLE_WIDTH * index as f32))
            .min_w(px(200.))
            .py(px(4.))
            .flex()
            .flex_col()
            .rounded(px(8.))
            .bg(look.background)
            .border_1()
            .border_color(look.border)
            .shadow_lg()
            .text_color(look.text)
            .text_size(px(13.))
            .occlude();
        for (row_index, row) in rows.iter().enumerate() {
            let Some(entry) = row else {
                list = list.child(div().my(px(4.)).h(px(1.)).bg(look.border));
                continue;
            };
            let action = entry.action.as_ref().filter(|_| entry.enabled).map(|a| a.boxed_clone());
            let choose = choose.clone();
            let mark = if entry.checked { "\u{2713}" } else { "" };
            list = list.child(
                div()
                    .id(("nd-menu-item", row_index))
                    .role(gpui::Role::MenuItem)
                    .aria_label(entry.label.clone())
                    .h(px(26.))
                    .mx(px(4.))
                    .px(px(8.))
                    .rounded(px(5.))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.))
                    .when(action.is_none(), |d| d.opacity(0.5))
                    .when(action.is_some(), move |d| d.hover(move |s| s.bg(hover)))
                    .child(div().w(px(12.)).child(mark))
                    .child(entry.label.clone())
                    .on_click(move |_, window, _| {
                        if let Some(action) = &action {
                            choose(action.boxed_clone(), window);
                        }
                    }),
            );
        }
        let close = toggle.clone();
        div()
            .absolute()
            .inset_0()
            .child(
                div()
                    .id("nd-menu-backdrop")
                    .absolute()
                    .inset_0()
                    .occlude()
                    .on_click(move |_, window, _| close(None, window)),
            )
            .child(list)
            .into_any_element()
    });
    (row.into_any_element(), dropdown)
}
