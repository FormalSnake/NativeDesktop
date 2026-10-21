//! Drawn on/off controls: `checkbox` (with its mixed state) and `switch`. A click or Space
//! reports [`Toggled`] with the state the control would move to; the shown state is the prop's.

use nd_components::forms::*;
use nd_core::NodeId;
use nd_gpui::gpui::prelude::FluentBuilder as _;
use nd_gpui::gpui::{
    self, AnimationExt as _, App, ElementId, InteractiveElement, IntoElement, KeyDownEvent, ParentElement, Pixels,
    StatefulInteractiveElement, Styled, Window, div, px,
};
use nd_theme::color::with_alpha;

use super::glyph::{Glyph, glyph};
use super::{Ctl, FormLook, Sink, with_style};
use crate::render::Build;

/// A transparent ring around `inner` that shows the focus colour while the ring is on. Under
/// a two-ring look (WinUI) the rings sit outside `inner` without taking layout space.
pub(crate) fn ring(look: FormLook, on: bool, radius: Pixels, inner: impl IntoElement) -> gpui::Div {
    if look.focus_inner.is_some() {
        return div().flex_none().relative().child(inner).when(on, |d| d.child(focus_visual(look, radius)));
    }
    if !look.ring_settles {
        return div()
            .flex_none()
            .p(px(1.))
            .rounded(radius + px(3.))
            .border_2()
            .border_color(if on { look.focus_ring } else { gpui::transparent_black() })
            .child(inner);
    }
    div()
        .relative()
        .flex_none()
        .p(px(3.))
        .child(inner)
        .when(on, |d| d.child(focus_outline(look.focus_ring, radius + px(3.), px(-2.), true)))
}

/// libadwaita's focus outline over an element rounded at `radius`: 2 px of `color`, `offset`
/// outside its edge (inside when negative), drawn over it so its layout never moves. When it
/// `settles`, it starts 4 px further out and closes onto the element over 200 ms, as GTK's
/// outline transition does.
pub(crate) fn focus_outline(color: gpui::Hsla, radius: Pixels, offset: Pixels, settles: bool) -> gpui::AnyElement {
    let edge = move |d: gpui::Div, grow: Pixels| {
        let out = (offset + px(2.) + grow).as_f32();
        let at = px(-out);
        d.top(at).left(at).right(at).bottom(at).rounded(px((radius.as_f32() + out).max(0.)))
    };
    let outline = edge(div().absolute().border_2().border_color(color), px(0.));
    if !settles {
        return outline.into_any_element();
    }
    let settle = gpui::Animation::new(std::time::Duration::from_millis(200))
        .with_easing(|t| super::look::cubic_bezier(super::look::EASE_OUT_QUAD, t));
    outline
        .with_animation("nd-focus-outline", settle, move |d, t| {
            edge(d, px(4. * (1. - t))).border_color(with_alpha(color, color.a * t))
        })
        .into_any_element()
}

/// WinUI's focus visual for a control of `radius` filling its parent (which must be
/// positioned): a 2 px ring in FocusStrokeColorOuter around a 2 px one in FocusStrokeColorInner,
/// touching the control (measured on a ComboBox in the WinUI Gallery).
pub(crate) fn focus_visual(look: FormLook, radius: Pixels) -> gpui::Div {
    let inner = look.focus_inner.unwrap_or(gpui::transparent_black());
    div()
        .absolute()
        .top(px(-4.))
        .left(px(-4.))
        .right(px(-4.))
        .bottom(px(-4.))
        .rounded(radius + px(4.))
        .border_2()
        .border_color(look.focus_ring)
        .child(div().size_full().rounded(radius + px(2.)).border_2().border_color(inner))
}

/// The bottom edge of WinUI's elevation border for a control of `radius` with a 1 px border,
/// drawn over that border; `None` for a look without one.
pub(crate) fn elevation_edge(look: FormLook, radius: Pixels) -> Option<gpui::Div> {
    let edge = look.stroke_bottom?;
    // Absolute insets start inside the border: -1 puts the edge on its bottom row.
    Some(
        div()
            .absolute()
            .top(px(-1.))
            .left(px(-1.))
            .right(px(-1.))
            .bottom(px(-1.))
            .rounded(radius)
            .border_b_1()
            .border_color(edge),
    )
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
    element.track_focus(ctl.focus()).cursor(ctl.look.cursor).on_key_down(
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
    let (rest, hover, active) = if on {
        (look.accent, look.accent_hover, look.accent_active)
    } else {
        (look.check_fill[0], look.check_fill[1], look.check_fill[2])
    };
    let mut indicator = div()
        .id("box")
        .size(look.check_size)
        .flex()
        .items_center()
        .justify_center()
        .rounded(look.check_radius)
        .border(look.check_stroke_width)
        .border_color(if on { look.accent } else { look.check_stroke })
        .bg(rest)
        .children(on.then(|| glyph(mark, look.check_glyph, look.on_accent, look.glyph_stroke + px(0.25))));
    if p.enabled && look.trough_states && !on {
        // libadwaita: the ring darkens hovered; pressed, the box fills and the ring goes.
        let (hover, active) = (look.track_hover, look.track_active);
        indicator = indicator
            .group_hover(CHECK_GROUP, move |s| s.border_color(hover))
            .group_active(CHECK_GROUP, move |s| s.bg(active).border_color(gpui::transparent_black()));
    } else if p.enabled {
        indicator = indicator
            .group_hover(CHECK_GROUP, move |s| s.bg(hover).border_color(if on { hover } else { look.check_stroke }))
            .group_active(CHECK_GROUP, move |s| s.bg(active).border_color(if on { active } else { look.check_stroke }));
    }
    let toggled = if p.mixed {
        gpui::Toggled::Mixed
    } else if p.checked {
        gpui::Toggled::True
    } else {
        gpui::Toggled::False
    };
    let row = div()
        .id(ElementId::Integer(id.0))
        .group(CHECK_GROUP)
        .role(gpui::Role::CheckBox)
        .aria_label(p.label.clone())
        .aria_toggled(toggled)
        .flex()
        .flex_row()
        .items_center()
        .min_h(look.toggle_row)
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

/// The checkbox row, whose hover and press the box shows.
const CHECK_GROUP: &str = "nd-check";

/// The switch, whose hover and press its knob and track show.
const SWITCH_GROUP: &str = "nd-switch";

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
    let (top, left) = (look.switch_inset, look.switch_inset + travel * at);
    let (grow, press) = (look.knob_hover_grow, look.knob_press);
    // WinUI's off track: ControlAltFillColorSecondary, Tertiary and Quarternary, as a checkbox.
    let [off_fill, off_hover, off_active] =
        if look.switch_outline.is_some() { look.check_fill } else { [look.fill, look.fill_hover, look.fill_active] };
    let (on_hover, on_active) = (look.accent_hover, look.accent_active);
    let mut knob_el = div()
        .id("knob")
        .absolute()
        .top(top)
        .left(left)
        .size(knob)
        .rounded_full()
        .bg(if p.checked { look.knob_hover } else { look.knob_off })
        .border_1()
        .border_color(look.knob_edge);
    if let Some(shadow) = look.knob_shadow {
        knob_el = knob_el.shadow(knob_shadow(shadow));
    }
    if p.enabled && look.trough_states {
        let lit = look.knob_hover;
        knob_el = knob_el.group_hover(SWITCH_GROUP, move |s| s.bg(lit));
    }
    let mut track = div()
        .id("track")
        .relative()
        .w(look.switch_width)
        .h(look.switch_height)
        .rounded_full()
        .bg(match (p.checked, look.switch_outline) {
            (true, _) => look.accent,
            (false, Some(_)) => off_fill,
            (false, None) => look.track,
        })
        .when_some(look.switch_outline.filter(|_| !p.checked), |track, outline| track.border_1().border_color(outline));
    if p.enabled && look.trough_states {
        // libadwaita: the trough darkens hovered and pressed; checked, a 10% veil and a 20% shade.
        let (hover, active) = if p.checked { (on_hover, on_active) } else { (look.track_hover, look.track_active) };
        track = track.group_hover(SWITCH_GROUP, move |s| s.bg(hover)).group_active(SWITCH_GROUP, move |s| s.bg(active));
    } else if p.enabled && look.switch_outline.is_some() {
        // WinUI's knob grows under the pointer and stretches towards the middle while pressed,
        // and the track takes the hovered and pressed fills.
        let half = grow / 2.;
        let stretch_left = if p.checked { left - half - press } else { left - half };
        knob_el = knob_el
            .group_hover(SWITCH_GROUP, move |s| s.size(knob + grow).top(top - half).left(left - half))
            .group_active(SWITCH_GROUP, move |s| {
                s.w(knob + grow + press).h(knob + grow).top(top - half).left(stretch_left)
            });
        track = if p.checked {
            track
                .group_hover(SWITCH_GROUP, move |s| s.bg(on_hover))
                .group_active(SWITCH_GROUP, move |s| s.bg(on_active))
        } else {
            track
                .group_hover(SWITCH_GROUP, move |s| s.bg(off_hover))
                .group_active(SWITCH_GROUP, move |s| s.bg(off_active))
        };
    }
    let track = track.child(knob_el);
    let name = p.label.clone().or_else(|| p.tooltip.clone()).unwrap_or_default();
    let control = div()
        .id(ElementId::Integer(id.0))
        .group(SWITCH_GROUP)
        .role(gpui::Role::Switch)
        .aria_label(name)
        .aria_toggled(if p.checked { gpui::Toggled::True } else { gpui::Toggled::False })
        .flex_none()
        .child(ring(look, ctl.ring(), look.switch_height / 2., track));
    let control = if p.enabled { toggles(control, &ctl, !p.checked) } else { control.opacity(look.disabled_opacity) };
    with_style(control, &p.style).into_any_element()
}

/// libadwaita's knob shadow: `0 2px 4px` of `color`.
pub(crate) fn knob_shadow(color: gpui::Hsla) -> Vec<gpui::BoxShadow> {
    vec![gpui::BoxShadow {
        color,
        offset: gpui::point(px(0.), px(2.)),
        blur_radius: px(4.),
        spread_radius: px(0.),
        inset: false,
    }]
}
