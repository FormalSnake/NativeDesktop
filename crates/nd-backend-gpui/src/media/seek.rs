//! `seek_bar`: the track, buffered range, played range and thumb, with elapsed and remaining
//! time under it. [`SeekModel`] decides what shows and what is reported; the bar extrapolates
//! the clock on its own frames while playing.

use std::sync::Arc;

use nd_components::{SEEK, SeekBarProps, SeekModel, TextStyle, format_clock};
use nd_core::NodeId;
use nd_core::time::Instant;
use nd_gpui::gpui::{
    self, AnyElement, App, DispatchPhase, ElementId, FontFeatures, InteractiveElement, IntoElement, KeyDownEvent,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, Point,
    StatefulInteractiveElement, Styled, Window, canvas, div, prelude::FluentBuilder, px, relative,
};

use crate::forms::{Sink, with_style};
use crate::nav::Marks;
use crate::render::Build;

#[derive(Default)]
pub(crate) struct Seek {
    pub model: SeekModel,
}

/// The position as automation reads it.
pub(crate) fn describe(props: &SeekBarProps, model: &mut SeekModel, now: Instant) -> String {
    let shown = model.shown(props, now);
    match props.duration {
        Some(d) => format!("{} of {}", format_clock(shown), format_clock(d)),
        None => format_clock(shown),
    }
}

/// The fraction along the track under `at`, from the node's last laid-out bounds.
fn fraction_at(
    bounds: &std::cell::RefCell<std::collections::HashMap<NodeId, nd_core::Rect>>,
    node: NodeId,
    at: Point<Pixels>,
) -> Option<f32> {
    let b = *bounds.borrow().get(&node)?;
    Some(((at.x.as_f32() - b.origin.x) / b.size.width.max(1.)).clamp(0., 1.))
}

pub(super) fn seek_bar(
    b: &Build,
    id: NodeId,
    p: &SeekBarProps,
    marks: &Marks,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let seekable = p.seekable();
    let (ctl, state) = b.ctl(id, seekable, window, cx, Seek::default);
    let theme = b.theme();
    let now = Instant::now();
    let (shown, times, active, dragging) = {
        let mut s = state.borrow_mut();
        let shown = s.model.shown(p, now);
        let times = p.show_times.then(|| s.model.times(p, now));
        (shown, times, seekable && (s.model.active() || ctl.ring()), s.model.dragging())
    };
    if p.clock.playing && !dragging {
        window.request_animation_frame();
    }
    let played = p.fraction(shown);
    let buffered = p.buffered_fraction(played);
    let thickness = if active { SEEK.track_active } else { SEEK.track };
    let accent = if p.enabled { theme.accent } else { theme.disabled(theme.accent) };
    let (track, loaded) = (theme.border.subtle, theme.border.strong);
    marks.fill("seek track", track, &[]);
    if buffered > played {
        marks.fill("seek buffered", loaded, &[track]);
    }
    if played > 0. {
        marks.fill("seek played", accent, &[track]);
    }

    let bar_fill = |fraction: f32, color| {
        div().absolute().left_0().top_0().bottom_0().w(relative(fraction)).rounded_full().bg(color)
    };
    let mut bar = div().id("track").relative().w_full().h(px(SEEK.hit)).child(
        div()
            .absolute()
            .left_0()
            .right_0()
            .top(px((SEEK.hit - thickness) / 2.))
            .h(px(thickness))
            .rounded_full()
            .bg(track)
            .when(buffered > played, |t| t.child(bar_fill(buffered, loaded)))
            .when(played > 0., |t| t.child(bar_fill(played, accent))),
    );
    if active {
        marks.fill("seek thumb", accent, &[]);
        bar = bar.child(
            div()
                .absolute()
                .top(px((SEEK.hit - SEEK.thumb) / 2.))
                .left(relative(played))
                .ml(px(-SEEK.thumb / 2.))
                .size(px(SEEK.thumb))
                .rounded_full()
                .bg(accent),
        );
    }

    let mut root = div()
        .id(ElementId::Integer(id.0))
        .role(gpui::Role::Slider)
        .aria_label(p.label.clone())
        .flex()
        .flex_col()
        .w_full();
    if seekable {
        let props = Arc::new(p.clone());
        let send = |sink: &Sink, event: Option<nd_components::Seeked>, window: &mut Window, cx: &mut App| {
            if let Some(event) = event {
                sink.send(event, cx);
            }
            window.refresh();
        };
        let down = {
            let (sink, state, props, bounds) = (ctl.sink.clone(), state.clone(), props.clone(), ctl.bounds.clone());
            move |event: &MouseDownEvent, window: &mut Window, cx: &mut App| {
                sink.take_focus(window, cx);
                let Some(f) = fraction_at(&bounds, id, event.position) else { return };
                let seeked = state.borrow_mut().model.press(&props, f);
                send(&sink, seeked, window, cx);
            }
        };
        let hover = {
            let state = state.clone();
            move |hovered: &bool, window: &mut Window, _: &mut App| {
                if state.borrow_mut().model.set_hovered(*hovered) {
                    window.refresh();
                }
            }
        };
        // Window-wide while dragging, so the thumb follows the pointer off the track.
        let follow = {
            let (sink, state, props, bounds) = (ctl.sink.clone(), state.clone(), props.clone(), ctl.bounds.clone());
            canvas(
                |_, _, _| (),
                move |_, _, window, _| {
                    let (sink2, state2, props2, bounds2) = (sink.clone(), state.clone(), props.clone(), bounds.clone());
                    window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                        if phase != DispatchPhase::Bubble || !state2.borrow().model.dragging() {
                            return;
                        }
                        let Some(f) = fraction_at(&bounds2, id, event.position) else { return };
                        let seeked = state2.borrow_mut().model.moved(&props2, f);
                        send(&sink2, seeked, window, cx);
                    });
                    let (sink, state, props) = (sink.clone(), state.clone(), props.clone());
                    window.on_mouse_event(move |_: &MouseUpEvent, phase, window, cx| {
                        if phase != DispatchPhase::Bubble {
                            return;
                        }
                        let seeked = state.borrow_mut().model.release(&props, Instant::now());
                        if seeked.is_some() {
                            send(&sink, seeked, window, cx);
                        }
                    });
                },
            )
            .absolute()
            .size_0()
        };
        bar = bar.on_mouse_down(MouseButton::Left, down).on_hover(hover).child(follow);
        let keys = {
            let (sink, state, props) = (ctl.sink.clone(), state.clone(), props.clone());
            move |event: &KeyDownEvent, window: &mut Window, cx: &mut App| {
                sink.keyboard();
                let direction = match event.keystroke.key.as_str() {
                    "left" => -1,
                    "right" => 1,
                    _ => {
                        window.refresh();
                        return;
                    }
                };
                cx.stop_propagation();
                let seeked = state.borrow_mut().model.step(&props, direction, Instant::now());
                send(&sink, seeked, window, cx);
            }
        };
        root = root.track_focus(ctl.focus()).on_key_down(keys);
    }
    root = root.child(bar);
    if let Some((elapsed, remaining)) = times {
        let (size, weight, line) = ctl.look.type_of(TextStyle::Caption1);
        let ink = theme.text.secondary;
        marks.text(&elapsed, ink, &[], size, weight);
        marks.text(&remaining, ink, &[], size, weight);
        let mut row = div()
            .flex()
            .flex_row()
            .justify_between()
            .mt(px(SEEK.label_gap))
            .text_size(size)
            .line_height(line)
            .text_color(ink)
            .child(elapsed)
            .child(remaining);
        row.style().text.font_features = Some(FontFeatures(Arc::new(vec![("tnum".into(), 1)])));
        root = root.child(row);
    }
    with_style(root, &p.style).into_any_element()
}
