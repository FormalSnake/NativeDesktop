//! Drawn numeric controls: `slider` (drag, click, arrow keys) and `stepper` (minus and plus
//! buttons, arrow keys, and the value in a field when asked). Both report the normalised value
//! the props would hold, and nothing when it would not change.

use std::cell::RefCell;
use std::rc::Rc;

use nd_components::forms::*;
use nd_core::NodeId;
use nd_gpui::gpui::{
    self, App, CursorStyle, DispatchPhase, ElementId, InteractiveElement, IntoElement, KeyDownEvent, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, Point, StatefulInteractiveElement, Styled,
    Window, canvas, div, px, relative,
};

use super::glyph::{Glyph, glyph};
use super::text::value_field;
use super::toggle::{pressable, ring};
use super::{Sink, with_style};
use crate::render::Build;

/// A drag in progress and the last value it reported.
#[derive(Default)]
pub(crate) struct Drag {
    dragging: bool,
    reported: Option<f32>,
}

struct Track {
    node: NodeId,
    props: SliderProps,
    knob: Pixels,
}

impl Track {
    /// The normalised value under `at`, from the slider's laid-out bounds.
    fn value_at(
        &self,
        ctl_bounds: &RefCell<std::collections::HashMap<NodeId, nd_core::Rect>>,
        at: Point<Pixels>,
    ) -> Option<f32> {
        let b = *ctl_bounds.borrow().get(&self.node)?;
        let knob = self.knob.as_f32();
        let fraction = if self.props.vertical {
            let len = (b.size.height - knob).max(1.);
            1. - (at.y.as_f32() - b.origin.y - knob / 2.) / len
        } else {
            let len = (b.size.width - knob).max(1.);
            (at.x.as_f32() - b.origin.x - knob / 2.) / len
        };
        let p = &self.props;
        Some(p.normalise(p.min + fraction.clamp(0., 1.) * (p.max - p.min)))
    }
}

fn report(sink: &Sink, drag: &RefCell<Drag>, shown: f32, value: f32, cx: &mut App) {
    let mut d = drag.borrow_mut();
    if d.reported.unwrap_or(shown) == value {
        return;
    }
    d.reported = Some(value);
    drop(d);
    sink.send(SliderChanged { value }, cx);
}

pub(crate) fn slider(b: &Build, id: NodeId, p: &SliderProps, window: &mut Window, cx: &mut App) -> gpui::AnyElement {
    let (ctl, drag) = b.ctl(id, p.enabled, window, cx, Drag::default);
    let look = ctl.look;
    let knob = look.slider_knob;
    let span = p.max - p.min;
    let fraction = if span.abs() < f32::EPSILON { 0. } else { ((p.value - p.min) / span).clamp(0., 1.) };
    let thickness = look.track_thickness;
    let cross = knob + if p.ticks > 0 { look.tick_length + px(3.) } else { px(0.) };

    // The rail spans the knob centre's travel; the track, fill, ticks and knob sit on it.
    let mut rail = div().absolute();
    rail = if p.vertical {
        rail.top(knob / 2.).bottom(knob / 2.).left_0().w(knob)
    } else {
        rail.left(knob / 2.).right(knob / 2.).top_0().h(knob)
    };
    let bar = |d: gpui::Div| {
        if p.vertical {
            d.w(thickness).left((knob - thickness) / 2.)
        } else {
            d.h(thickness).top((knob - thickness) / 2.)
        }
    };
    let track = bar(div().absolute().rounded_full().bg(look.track));
    let track = if p.vertical { track.top_0().bottom_0() } else { track.left_0().right_0() };
    let filled = bar(div().absolute().rounded_full().bg(look.accent));
    let filled =
        if p.vertical { filled.bottom_0().h(relative(fraction)) } else { filled.left_0().w(relative(fraction)) };
    rail = rail.child(track).child(filled);
    if p.ticks > 0 {
        for i in 0..p.ticks {
            let at = if p.ticks == 1 { 0.5 } else { i as f32 / (p.ticks - 1) as f32 };
            let tick = div().absolute().bg(look.secondary_text);
            let tick = if p.vertical {
                tick.w(look.tick_length).h(px(1.)).left(knob + px(3.)).bottom(relative(at))
            } else {
                tick.h(look.tick_length).w(px(1.)).top(knob + px(3.)).left(relative(at))
            };
            rail = rail.child(tick);
        }
    }
    let thumb = div().absolute().size(knob + px(6.)).child(ring(
        look,
        ctl.ring(),
        knob / 2.,
        div().size(knob - px(2.)).rounded_full().bg(look.knob).border_1().border_color(look.knob_edge),
    ));
    let thumb = if p.vertical {
        thumb.left(px(-3.)).bottom(relative(fraction)).mb(-(knob / 2.) - px(3.))
    } else {
        thumb.top(px(-3.)).left(relative(fraction)).ml(-(knob / 2.) - px(3.))
    };
    rail = rail.child(thumb);

    let name = p.label.clone().or_else(|| p.tooltip.clone()).unwrap_or_default();
    let mut slider = div()
        .id(ElementId::Integer(id.0))
        .role(gpui::Role::Slider)
        .aria_label(name)
        .aria_numeric_value(p.value.into())
        .aria_min_numeric_value(p.min.into())
        .aria_max_numeric_value(p.max.into())
        .aria_orientation(if p.vertical { gpui::Orientation::Vertical } else { gpui::Orientation::Horizontal })
        .relative()
        .flex_none()
        .child(rail);
    slider = if p.vertical {
        slider.w(cross).min_h(look.slider_min_length)
    } else {
        slider.h(cross).min_w(look.slider_min_length)
    };
    if let Some(step) = p.step {
        slider = slider.aria_numeric_value_step(step.into());
    }
    if !p.enabled {
        return with_style(slider.opacity(look.disabled_opacity), &p.style).into_any_element();
    }

    let track = Rc::new(Track { node: id, props: p.clone(), knob });
    let shown = p.value;
    let down = {
        let (sink, drag, track, bounds) = (ctl.sink.clone(), drag.clone(), track.clone(), ctl.bounds.clone());
        move |event: &MouseDownEvent, window: &mut Window, cx: &mut App| {
            sink.take_focus(window, cx);
            *drag.borrow_mut() = Drag { dragging: true, reported: None };
            if let Some(v) = track.value_at(&bounds, event.position) {
                report(&sink, &drag, shown, v, cx);
            }
        }
    };
    // Window-wide while dragging, so the knob follows the pointer off the track.
    let follow = {
        let (sink, drag, track, bounds) = (ctl.sink.clone(), drag.clone(), track.clone(), ctl.bounds.clone());
        canvas(
            |_, _, _| (),
            move |_, _, window, _| {
                let (sink, drag2, track2, bounds2) = (sink.clone(), drag.clone(), track.clone(), bounds.clone());
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                    if phase == DispatchPhase::Bubble
                        && drag2.borrow().dragging
                        && event.dragging()
                        && let Some(v) = track2.value_at(&bounds2, event.position)
                    {
                        report(&sink, &drag2, shown, v, cx);
                    }
                });
                let drag = drag.clone();
                window.on_mouse_event(move |_: &MouseUpEvent, phase, _, _| {
                    if phase == DispatchPhase::Bubble {
                        drag.borrow_mut().dragging = false;
                    }
                });
            },
        )
        .absolute()
        .size_0()
    };
    let keys = {
        let (sink, props) = (ctl.sink.clone(), p.clone());
        move |event: &KeyDownEvent, window: &mut Window, cx: &mut App| {
            sink.keyboard();
            window.refresh();
            let step = props.step.filter(|s| *s > 0.).unwrap_or((props.max - props.min).abs() / 20.);
            let next = match event.keystroke.key.as_str() {
                "left" | "down" => props.value - step,
                "right" | "up" => props.value + step,
                "home" => props.min,
                "end" => props.max,
                _ => return,
            };
            let next = props.normalise(next);
            cx.stop_propagation();
            if next != props.value {
                sink.send(SliderChanged { value: next }, cx);
            }
        }
    };
    slider = pressable(slider, &ctl)
        .cursor(CursorStyle::Arrow)
        .on_mouse_down(MouseButton::Left, down)
        .on_key_down(keys)
        .child(follow);
    with_style(slider, &p.style).into_any_element()
}

pub(crate) fn stepper(b: &Build, id: NodeId, p: &StepperProps, window: &mut Window, cx: &mut App) -> gpui::AnyElement {
    let (ctl, field) = match p.field {
        Some(digits) => {
            let props = p.clone();
            let parse = move |text: &str| -> Option<Box<dyn std::any::Any>> {
                let v = text.trim().parse::<f64>().ok().filter(|v| v.is_finite())?;
                Some(Box::new(StepperChanged { value: props.normalise(v) }))
            };
            let text = number_text(p.value, Some(digits)).into();
            let (ctl, field) = value_field(b, id, text, p.enabled, parse, window, cx);
            let width = ctl.look.field_min_width / 2.;
            (ctl, Some(field.min_w(width).rounded_r(px(0.))))
        }
        None => (b.ctl(id, p.enabled, window, cx, || ()).0, None),
    };
    let look = ctl.look;
    let button = |key: &'static str, g: Glyph, label: &'static str, next: f64, last: bool| {
        let available = p.enabled && p.normalise(next) != p.value;
        let mut b = div()
            .id(key)
            .role(gpui::Role::Button)
            .aria_label(label)
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(look.control_height)
            .bg(look.fill)
            .child(glyph(
                g,
                look.icon_size,
                if available { look.text } else { look.secondary_text },
                look.glyph_stroke,
            ));
        b = if last { b.rounded_r(look.radius) } else { b };
        if available {
            let (sink, value) = (ctl.sink.clone(), p.normalise(next));
            b = b
                .cursor(CursorStyle::PointingHand)
                .hover(move |s| s.bg(look.fill_hover))
                .active(move |s| s.bg(look.fill_active))
                .on_click(move |_, window, cx| {
                    sink.take_focus(window, cx);
                    sink.send(StepperChanged { value }, cx);
                });
        }
        b
    };
    let name = p.label.clone().or_else(|| p.tooltip.clone()).unwrap_or_default();
    let has_field = field.is_some();
    let mut group = div()
        .id(ElementId::Integer(id.0))
        .role(gpui::Role::SpinButton)
        .aria_label(name)
        .aria_numeric_value(p.value)
        .aria_min_numeric_value(p.min)
        .aria_max_numeric_value(p.max)
        .aria_numeric_value_step(p.step)
        .flex()
        .flex_row()
        .items_center()
        .gap(px(1.))
        .text_color(look.text)
        .children(field)
        .child({
            let minus = button("down", Glyph::Minus, "Decrement", p.value - p.step, false);
            if has_field { minus } else { minus.rounded_l(look.radius) }
        })
        .child(button("up", Glyph::Plus, "Increment", p.value + p.step, true));
    if !has_field {
        group = div().id(ElementId::Integer(id.0)).child(ring(look, ctl.ring(), look.radius, group)).flex_none();
    }
    if !p.enabled {
        return with_style(group.opacity(look.disabled_opacity), &p.style).into_any_element();
    }
    let keys = {
        let (sink, props) = (ctl.sink.clone(), p.clone());
        move |event: &KeyDownEvent, window: &mut Window, cx: &mut App| {
            sink.keyboard();
            window.refresh();
            let next = match event.keystroke.key.as_str() {
                "up" => props.value + props.step,
                "down" => props.value - props.step,
                _ => return,
            };
            cx.stop_propagation();
            let next = props.normalise(next);
            if next != props.value {
                sink.send(StepperChanged { value: next }, cx);
            }
        }
    };
    group = if has_field { group.on_key_down(keys) } else { pressable(group, &ctl).on_key_down(keys) };
    with_style(group, &p.style).into_any_element()
}
