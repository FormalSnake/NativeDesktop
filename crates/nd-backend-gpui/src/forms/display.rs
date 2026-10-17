//! Drawn display controls: `progress` (bar, spinner, circular; indeterminate ones animate on the
//! window's frame clock), `label` (text styles on the skin's type scale, tone colours) and
//! `link` (accent text, underlined under the pointer, reporting [`LinkActivated`]).

use std::f32::consts::PI;

use nd_components::forms::*;
use nd_core::NodeId;
use nd_gpui::gpui::{
    self, App, CursorStyle, ElementId, InteractiveElement, IntoElement, ParentElement, StatefulInteractiveElement,
    Styled, Window, div, relative,
};

use super::glyph::arc;
use super::toggle::{pressable, ring};
use super::{FormLook, with_style};
use crate::render::Build;

/// Position in a repeating cycle of `seconds`, 0 to 1.
fn phase(time: f64, seconds: f64) -> f32 {
    (time / seconds).rem_euclid(1.) as f32
}

pub(crate) fn progress(b: &Build, id: NodeId, p: &ProgressProps, window: &mut Window) -> gpui::AnyElement {
    let look = FormLook::of(&b.theme());
    let value = p.value.map(|v| v.clamp(0., 1.));
    let animated = value.is_none() || p.presentation == ProgressStyle::Spinner;
    if animated && p.enabled {
        window.request_animation_frame();
    }
    let indicator = match p.presentation {
        ProgressStyle::Bar => {
            let fill = div().absolute().top_0().bottom_0().rounded_full().bg(look.accent);
            let fill = match value {
                Some(v) => fill.left_0().w(relative(v)),
                None => {
                    let segment = look.indeterminate_segment;
                    let at = phase(b.time, look.indeterminate_seconds) * (1. + segment) - segment;
                    fill.left(relative(at)).w(relative(segment))
                }
            };
            div()
                .relative()
                .overflow_hidden()
                .h(look.track_thickness)
                .min_w(look.slider_min_length)
                .rounded_full()
                .bg(look.track)
                .child(fill)
                .into_any_element()
        }
        ProgressStyle::Spinner => {
            let start = phase(b.time, look.spinner_seconds) * 2. * PI;
            arc(look.spinner_size, start, 1.5 * PI, look.secondary_text, look.spinner_stroke).into_any_element()
        }
        ProgressStyle::Circular => {
            let size = look.spinner_size;
            let (start, sweep) = match value {
                Some(v) => (-PI / 2., v * 2. * PI),
                None => (phase(b.time, look.spinner_seconds) * 2. * PI, 0.5 * PI),
            };
            div()
                .relative()
                .size(size)
                .child(div().absolute().child(arc(size, 0., 2. * PI, look.track, look.spinner_stroke)))
                .child(div().absolute().child(arc(size, start, sweep, look.accent, look.spinner_stroke)))
                .into_any_element()
        }
    };
    let name = p.label.clone().or_else(|| p.tooltip.clone()).unwrap_or_default();
    let mut element = div()
        .id(ElementId::Integer(id.0))
        .role(gpui::Role::ProgressIndicator)
        .aria_label(name)
        .flex()
        .items_center()
        .child(indicator);
    if let (Some(v), false) = (value, p.presentation == ProgressStyle::Spinner) {
        element =
            element.aria_numeric_value(f64::from(v) * 100.).aria_min_numeric_value(0.).aria_max_numeric_value(100.);
    }
    if !p.enabled {
        element = element.opacity(look.disabled_opacity);
    }
    with_style(element, &p.style).into_any_element()
}

pub(crate) fn label(b: &Build, id: NodeId, p: &LabelProps) -> gpui::AnyElement {
    let look = FormLook::of(&b.theme());
    let (size, weight, line_height) = look.type_of(p.text_style);
    let color = match p.tone {
        LabelTone::Primary => look.text,
        LabelTone::Secondary => look.secondary_text,
        LabelTone::Danger => look.danger,
    };
    let color = p.style.text.color.unwrap_or(color);
    if let Some(rich) = &p.rich {
        let theme = b.theme();
        let base = crate::rich::Base {
            theme: &theme,
            size,
            leading: line_height.as_f32() / size.as_f32(),
            weight: weight.0,
            color,
            ink: None,
        };
        b.rich_runs(id, rich, &base, None);
        let on_link = rich.has_links().then(|| b.link_handler(id, p.opens_links));
        let mut text =
            div().font_weight(weight).child(crate::rich::element(ElementId::Integer(id.0), rich, &base, on_link));
        if !p.enabled {
            text = text.opacity(look.disabled_opacity);
        }
        return with_style(text, &p.style).into_any_element();
    }
    b.text_run(id, &p.text, crate::render::Run { size: size.as_f32(), weight: weight.0 }, Some(color), None);
    let mut text =
        div().text_size(size).line_height(line_height).font_weight(weight).text_color(color).child(p.text.clone());
    match (p.wraps, p.max_lines) {
        (false, _) => text = text.truncate(),
        (true, Some(lines)) => text = text.line_clamp(usize::from(lines)),
        (true, None) => {}
    }
    if !p.enabled {
        text = text.opacity(look.disabled_opacity);
    }
    with_style(text, &p.style).into_any_element()
}

pub(crate) fn link(b: &Build, id: NodeId, p: &LinkProps, window: &mut Window, cx: &mut App) -> gpui::AnyElement {
    let (ctl, _) = b.ctl(id, p.enabled, window, cx, || ());
    let look = ctl.look;
    let mut text = div()
        .id(ElementId::Integer(id.0))
        .role(gpui::Role::Link)
        .aria_label(p.title.clone())
        .text_size(look.text_size)
        .line_height(look.line_height)
        .text_color(look.accent_text)
        .whitespace_nowrap()
        .child(p.title.clone());
    if let Some(url) = &p.url {
        text = text.aria_value(url.clone());
    }
    if !p.enabled {
        return with_style(text.opacity(look.disabled_opacity), &p.style).into_any_element();
    }
    let (sink, url, external) = (ctl.sink.clone(), p.url.clone(), p.open_external);
    text = pressable(text, &ctl)
        .cursor(CursorStyle::PointingHand)
        .hover(move |s| s.underline().text_decoration_color(look.accent_text))
        .on_click(move |event, window, cx| {
            if !matches!(event, gpui::ClickEvent::Keyboard(_)) {
                sink.take_focus(window, cx);
            }
            sink.send(LinkActivated { url: url.clone() }, cx);
            if external && let Some(url) = &url {
                cx.open_url(url);
            }
        });
    let link = div().flex_none().child(ring(look, ctl.ring(), look.radius, with_style(text, &p.style)));
    link.into_any_element()
}
