//! Drawn `select`: a pop-up button with a menu for `Automatic`/`PopUp`, a toggle group for
//! `Segmented`, radio buttons for `Radio`. Every pick reports [`SelectChanged`]; the selected
//! option shown is the prop's.

use nd_components::Mapping;
use nd_components::forms::*;
use nd_core::NodeId;
use nd_gpui::gpui::{
    self, App, CursorStyle, ElementId, InteractiveElement, IntoElement, KeyDownEvent, MouseDownEvent, ParentElement,
    StatefulInteractiveElement, Styled, Window, div, px,
};

use super::glyph::{Glyph, glyph};
use super::text::{menu_item, popover};
use super::toggle::{pressable, ring};
use super::{Sink, with_style};
use crate::render::Build;

/// The pop-up's menu while open.
#[derive(Default)]
pub(crate) struct Menu {
    open: bool,
    highlight: Option<usize>,
}

pub(crate) fn select(b: &Build, id: NodeId, p: &SelectProps, window: &mut Window, cx: &mut App) -> gpui::AnyElement {
    match p.mapping {
        Mapping::Automatic | Mapping::PopUp => popup(b, id, p, window, cx),
        Mapping::Segmented => segmented(b, id, p, window, cx),
        Mapping::Radio => radio(b, id, p, window, cx),
    }
}

/// Arrow keys step the selection, wrapping at the ends.
fn arrows(sink: Sink, selected: usize, count: usize) -> impl Fn(&KeyDownEvent, &mut Window, &mut App) + 'static {
    move |event: &KeyDownEvent, window: &mut Window, cx: &mut App| {
        sink.keyboard();
        window.refresh();
        let step = match event.keystroke.key.as_str() {
            "left" | "up" => count - 1,
            "right" | "down" => 1,
            _ => return,
        };
        if count == 0 || event.keystroke.modifiers.modified() {
            return;
        }
        sink.send(SelectChanged { index: (selected.min(count - 1) + step) % count }, cx);
        cx.stop_propagation();
    }
}

fn name(p: &SelectProps) -> gpui::SharedString {
    p.label.clone().or_else(|| p.tooltip.clone()).unwrap_or_default()
}

fn popup(b: &Build, id: NodeId, p: &SelectProps, window: &mut Window, cx: &mut App) -> gpui::AnyElement {
    let (ctl, menu) = b.ctl(id, p.enabled, window, cx, Menu::default);
    let look = ctl.look;
    if !ctl.focused {
        menu.borrow_mut().open = false;
    }
    let open = menu.borrow().open && p.enabled;
    let title = p.options.get(p.selected).cloned().unwrap_or_default();
    let mut button = div()
        .id(ElementId::Integer(id.0))
        .role(gpui::Role::ComboBox)
        .aria_label(name(p))
        .aria_value(title.clone())
        .aria_expanded(open)
        .flex()
        .flex_row()
        .items_center()
        .gap(look.gap)
        .h(look.control_height)
        .px(look.padding_x)
        .rounded(look.radius)
        .bg(look.fill)
        .border_2()
        .border_color(if ctl.ring() { look.focus_ring } else { gpui::transparent_black() })
        .text_size(look.text_size)
        .line_height(look.line_height)
        .text_color(look.text)
        .whitespace_nowrap()
        .child(div().flex_1().child(title))
        .child(glyph(Glyph::ChevronDown, look.icon_size, look.text, look.glyph_stroke));
    if !p.enabled {
        return with_style(button.opacity(look.disabled_opacity), &p.style).into_any_element();
    }
    let count = p.options.len();
    let pick = {
        let (menu, sink) = (menu.clone(), ctl.sink.clone());
        std::rc::Rc::new(move |index: usize, window: &mut Window, cx: &mut App| {
            *menu.borrow_mut() = Menu::default();
            sink.send(SelectChanged { index }, cx);
            window.refresh();
        })
    };
    let keys = {
        let (menu, sink, pick, selected) = (menu.clone(), ctl.sink.clone(), pick.clone(), p.selected);
        move |event: &KeyDownEvent, window: &mut Window, cx: &mut App| {
            sink.keyboard();
            window.refresh();
            let mut m = menu.borrow_mut();
            let key = event.keystroke.key.as_str();
            if !m.open {
                if key == "down" && count > 0 {
                    *m = Menu { open: true, highlight: Some(selected.min(count - 1)) };
                    cx.stop_propagation();
                }
                return;
            }
            match key {
                "up" | "down" if count > 0 => {
                    let at = m.highlight.unwrap_or(selected).min(count - 1);
                    m.highlight = Some(if key == "up" { at.saturating_sub(1) } else { (at + 1).min(count - 1) });
                }
                "enter" | "space" => {
                    let index = m.highlight.unwrap_or(selected);
                    drop(m);
                    window.prevent_default();
                    cx.stop_propagation();
                    pick(index, window, cx);
                    return;
                }
                "escape" => *m = Menu::default(),
                _ => return,
            }
            window.prevent_default();
            cx.stop_propagation();
        }
    };
    let toggle = {
        let (menu, sink, selected) = (menu.clone(), ctl.sink.clone(), p.selected);
        move |event: &gpui::ClickEvent, window: &mut Window, cx: &mut App| {
            if !matches!(event, gpui::ClickEvent::Keyboard(_)) {
                sink.take_focus(window, cx);
            }
            let mut m = menu.borrow_mut();
            *m = if m.open { Menu::default() } else { Menu { open: true, highlight: Some(selected) } };
            drop(m);
            window.refresh();
        }
    };
    button = button
        .track_focus(ctl.focus())
        .cursor(CursorStyle::PointingHand)
        .hover(move |s| s.bg(look.fill_hover))
        .active(move |s| s.bg(look.fill_active))
        .on_key_down(keys)
        .on_click(toggle);
    let mut root = div().flex().flex_col().child(with_style(button, &p.style));
    if open {
        let highlight = menu.borrow().highlight;
        let close = {
            let menu = menu.clone();
            move |_: &MouseDownEvent, window: &mut Window, _: &mut App| {
                *menu.borrow_mut() = Menu::default();
                window.refresh();
            }
        };
        let items = p.options.iter().enumerate().map(|(index, option)| {
            let pick = pick.clone();
            menu_item(look, index, option.clone(), highlight == Some(index), index == p.selected)
                .on_click(move |_, window, cx| pick(index, window, cx))
        });
        root = root.child(popover(b, id, window, look, close, items));
    }
    root.into_any_element()
}

fn segmented(b: &Build, id: NodeId, p: &SelectProps, window: &mut Window, cx: &mut App) -> gpui::AnyElement {
    let (ctl, _) = b.ctl(id, p.enabled, window, cx, || ());
    let look = ctl.look;
    let inset = px(3.);
    let mut group = div()
        .id(ElementId::Integer(id.0))
        .role(gpui::Role::RadioGroup)
        .aria_label(name(p))
        .flex()
        .flex_row()
        .h(look.control_height)
        .p(inset - px(2.))
        .rounded(look.radius + inset)
        .bg(look.fill)
        .border_2()
        .border_color(if ctl.ring() { look.focus_ring } else { gpui::transparent_black() })
        .text_size(look.text_size)
        .line_height(look.line_height)
        .text_color(look.text);
    for (index, option) in p.options.iter().enumerate() {
        let selected = index == p.selected;
        let mut segment = div()
            .id(ElementId::Integer(index as u64))
            .role(gpui::Role::RadioButton)
            .aria_label(option.clone())
            .aria_toggled(if selected { gpui::Toggled::True } else { gpui::Toggled::False })
            .flex()
            .flex_1()
            .items_center()
            .justify_center()
            .px(look.padding_x)
            .rounded(look.radius)
            .whitespace_nowrap()
            .child(option.clone());
        if selected {
            segment = segment.bg(look.raised).shadow(vec![gpui::BoxShadow {
                color: gpui::hsla(0., 0., 0., 0.12),
                offset: gpui::point(px(0.), px(1.)),
                blur_radius: px(2.),
                spread_radius: px(0.),
                inset: false,
            }]);
        }
        if p.enabled {
            let sink = ctl.sink.clone();
            segment = segment.cursor(CursorStyle::PointingHand).when_hover_bg(!selected, look.fill_hover).on_click(
                move |_, window, cx| {
                    sink.take_focus(window, cx);
                    sink.send(SelectChanged { index }, cx);
                },
            );
        }
        group = group.child(segment);
    }
    let group = if p.enabled {
        group.track_focus(ctl.focus()).on_key_down(arrows(ctl.sink.clone(), p.selected, p.options.len()))
    } else {
        group.opacity(look.disabled_opacity)
    };
    with_style(group, &p.style).into_any_element()
}

fn radio(b: &Build, id: NodeId, p: &SelectProps, window: &mut Window, cx: &mut App) -> gpui::AnyElement {
    let (ctl, _) = b.ctl(id, p.enabled, window, cx, || ());
    let look = ctl.look;
    let mut group = div()
        .id(ElementId::Integer(id.0))
        .role(gpui::Role::RadioGroup)
        .aria_label(name(p))
        .flex()
        .gap(if p.horizontal { look.gap * 2. } else { look.gap / 2. })
        .text_size(look.text_size)
        .line_height(look.line_height)
        .text_color(look.text);
    group = if p.horizontal { group.flex_row().flex_wrap() } else { group.flex_col() };
    for (index, option) in p.options.iter().enumerate() {
        let selected = index == p.selected;
        let dot = div()
            .size(look.check_size)
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .border_2()
            .border_color(if selected { look.accent } else { look.track })
            .bg(if selected { look.accent } else { gpui::transparent_black() })
            .children(selected.then(|| div().size(px(6.)).rounded_full().bg(look.on_accent)));
        let mut item = div()
            .id(ElementId::Integer(index as u64))
            .role(gpui::Role::RadioButton)
            .aria_label(option.clone())
            .aria_toggled(if selected { gpui::Toggled::True } else { gpui::Toggled::False })
            .flex()
            .flex_row()
            .items_center()
            .gap(look.gap)
            .child(ring(look, selected && ctl.ring(), look.check_size / 2., dot))
            .child(option.clone());
        if p.enabled {
            let sink = ctl.sink.clone();
            item = item.cursor(CursorStyle::PointingHand).on_click(move |_, window, cx| {
                sink.take_focus(window, cx);
                sink.send(SelectChanged { index }, cx);
            });
        }
        group = group.child(item);
    }
    let group = if p.enabled {
        pressable(group, &ctl).on_key_down(arrows(ctl.sink.clone(), p.selected, p.options.len()))
    } else {
        group.opacity(look.disabled_opacity)
    };
    with_style(group, &p.style).into_any_element()
}

/// A hover fill only where `on`, for segments other than the selected one.
trait WhenHoverBg: StatefulInteractiveElement + Sized {
    fn when_hover_bg(self, on: bool, color: gpui::Hsla) -> Self {
        if on { self.hover(move |s| s.bg(color)) } else { self }
    }
}

impl<T: StatefulInteractiveElement> WhenHoverBg for T {}
