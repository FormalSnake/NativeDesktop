//! Drawn on/off controls: `checkbox` (with its mixed state) and `switch`. A click or Space
//! reports [`Toggled`] with the state the control would move to; the shown state is the prop's.

use nd_components::forms::*;
use nd_core::NodeId;
use nd_gpui::gpui::{
    self, App, CursorStyle, ElementId, InteractiveElement, IntoElement, KeyDownEvent, ParentElement, Pixels,
    StatefulInteractiveElement, Styled, Window, div, px,
};

use super::glyph::{Glyph, glyph};
use super::{Ctl, FormLook, Sink, with_style};
use crate::render::Build;

/// A transparent ring around `inner` that shows the focus colour while the ring is on.
pub(crate) fn ring(look: FormLook, on: bool, radius: Pixels, inner: impl IntoElement) -> gpui::Div {
    div()
        .flex_none()
        .p(px(1.))
        .rounded(radius + px(3.))
        .border_2()
        .border_color(if on { look.focus_ring } else { gpui::transparent_black() })
        .child(inner)
}

/// Toggles on click, and on Space or Return through GPUI's keyboard click.
fn toggles(element: gpui::Stateful<gpui::Div>, ctl: &Ctl, next: bool) -> gpui::Stateful<gpui::Div> {
    let sink = ctl.sink.clone();
    pressable(element, ctl).on_click(move |event, window, cx| {
        if !matches!(event, gpui::ClickEvent::Keyboard(_)) {
            sink.take_focus(window, cx);
        }
        sink.send(Toggled { checked: next }, cx);
    })
}

/// Focusable, with the focus ring turning on at the first key.
pub(crate) fn pressable(element: gpui::Stateful<gpui::Div>, ctl: &Ctl) -> gpui::Stateful<gpui::Div> {
    let key: Sink = ctl.sink.clone();
    element.track_focus(ctl.focus()).cursor(CursorStyle::PointingHand).on_key_down(
        move |_: &KeyDownEvent, window: &mut Window, _: &mut App| {
            key.keyboard();
            window.refresh();
        },
    )
}

pub(crate) fn checkbox(
    b: &Build,
    id: NodeId,
    p: &CheckboxProps,
    window: &mut Window,
    cx: &mut App,
) -> gpui::AnyElement {
    let (ctl, _) = b.ctl(id, p.enabled, window, cx, || ());
    let look = ctl.look;
    let on = p.checked || p.mixed;
    let mark = if p.mixed { Glyph::Dash } else { Glyph::Check };
    let indicator = div()
        .size(look.check_size)
        .flex()
        .items_center()
        .justify_center()
        .rounded(look.check_radius)
        .border_2()
        .border_color(if on { look.accent } else { look.track })
        .when_bg(on, look.accent)
        .children(on.then(|| glyph(mark, look.check_size - px(4.), look.on_accent, look.glyph_stroke + px(0.25))));
    let toggled = if p.mixed {
        gpui::Toggled::Mixed
    } else if p.checked {
        gpui::Toggled::True
    } else {
        gpui::Toggled::False
    };
    let row = div()
        .id(ElementId::Integer(id.0))
        .role(gpui::Role::CheckBox)
        .aria_label(p.label.clone())
        .aria_toggled(toggled)
        .flex()
        .flex_row()
        .items_center()
        .gap(look.gap)
        .text_size(look.text_size)
        .line_height(look.line_height)
        .child(ring(look, ctl.ring(), look.check_radius, indicator))
        .children((!p.label.is_empty()).then(|| p.label.clone()));
    // A mixed checkbox checks, as AppKit's cycle does from mixed.
    let next = p.mixed || !p.checked;
    let row = if p.enabled { toggles(row, &ctl, next) } else { row.opacity(look.disabled_opacity) };
    with_style(row, &p.style).into_any_element()
}

/// Where the knob was last drawn, so a prop change slides it from there.
#[derive(Default)]
pub(crate) struct Slide {
    shown: Option<bool>,
    from: f32,
    start: f64,
    at: f32,
}

pub(crate) fn switch(b: &Build, id: NodeId, p: &SwitchProps, window: &mut Window, cx: &mut App) -> gpui::AnyElement {
    let (ctl, slide) = b.ctl(id, p.enabled, window, cx, Slide::default);
    let look = ctl.look;
    let target = if p.checked { 1. } else { 0. };
    let at = {
        let mut s = slide.borrow_mut();
        if s.shown != Some(p.checked) {
            // No slide on the first frame: a switch appears in its state.
            (s.from, s.start) = if s.shown.is_some() { (s.at, ctl.time) } else { (target, f64::NEG_INFINITY) };
            s.shown = Some(p.checked);
        }
        let t = ((ctl.time - s.start) / look.toggle_seconds) as f32;
        s.at = s.from + (target - s.from) * look.ease(t);
        if t < 1. {
            window.request_animation_frame();
        }
        s.at
    };
    let knob = look.switch_height - look.switch_inset * 2.;
    let travel = look.switch_width - look.switch_inset * 2. - knob;
    let track = div()
        .relative()
        .w(look.switch_width)
        .h(look.switch_height)
        .rounded_full()
        .bg(if p.checked { look.accent } else { look.track })
        .child(
            div()
                .absolute()
                .top(look.switch_inset)
                .left(look.switch_inset + travel * at)
                .size(knob)
                .rounded_full()
                .bg(look.knob)
                .border_1()
                .border_color(look.knob_edge),
        );
    let name = p.label.clone().or_else(|| p.tooltip.clone()).unwrap_or_default();
    let control = div()
        .id(ElementId::Integer(id.0))
        .role(gpui::Role::Switch)
        .aria_label(name)
        .aria_toggled(if p.checked { gpui::Toggled::True } else { gpui::Toggled::False })
        .flex_none()
        .child(ring(look, ctl.ring(), look.switch_height / 2., track));
    let control = if p.enabled { toggles(control, &ctl, !p.checked) } else { control.opacity(look.disabled_opacity) };
    with_style(control, &p.style).into_any_element()
}

use crate::drawn::WhenBg;
