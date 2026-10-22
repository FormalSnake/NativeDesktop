//! `lyrics_view`: rows from [`LyricsModel`], the anchor row held at [`ANCHOR_AT`] of the view by a
//! tween over [`SCROLL`], rows fading and blurring with their distance from it, the anchor row
//! filling word by word. A wheel over the view stops following, with every row sharp, until the
//! resync button, new lyrics or `resync_after`.
//!
//! GPUI has no blur filter for text, so a blurred row is drawn as several copies offset on a
//! ring around it, each faint, which sums to the row's opacity with soft edges: a box blur of
//! the row's radius. Only rows on screen get the copies.

use std::cell::RefCell;
use std::rc::Rc;

use nd_components::{
    LyricClicked, LyricRowKind, LyricsFollowChanged, LyricsModel, LyricsProps, SCROLL, SCROLL_CURVE, TextStyle,
    UNLIT_SCALE, anchor_y, bezier, follow_offset, interlude_fill, row_depth, word_fill,
};
use nd_core::NodeId;
use nd_core::time::{Duration, Instant};
use nd_gpui::gpui::{
    self, AnyElement, App, ElementId, FontWeight, Hsla, InteractiveElement, IntoElement, ParentElement, Pixels,
    ScrollWheelEvent, SharedString, StatefulInteractiveElement, Styled, Window, canvas, div, prelude::FluentBuilder,
    px, relative,
};
use nd_theme::color::with_alpha;

use crate::forms::glyph::{Glyph, glyph};
use crate::forms::{Sink, with_style};
use crate::nav::Marks;
use crate::render::Build;

/// Rows as last laid out: each one's top and height relative to the column.
#[derive(Default)]
struct Measured {
    column: Option<f32>,
    rows: Vec<Option<(f32, f32)>>,
}

#[derive(Clone, Copy)]
struct Tween {
    from: f32,
    to: f32,
    start: Instant,
}

/// A row's opacity and blur, easing to `to` from where it was when the target changed.
#[derive(Clone, Copy)]
struct Depth {
    from: (f32, f32),
    to: (f32, f32),
    start: Instant,
}

impl Depth {
    fn at(&self, now: Instant, duration: f32, curve: [f32; 4]) -> (f32, f32) {
        let x = if duration <= 0. { 1. } else { now.saturating_duration_since(self.start).as_secs_f32() / duration };
        if x >= 1. {
            return self.to;
        }
        let p = bezier(curve, x);
        (self.from.0 + (self.to.0 - self.from.0) * p, self.from.1 + (self.to.1 - self.from.1) * p)
    }
}

/// Where the copies of a row blurred by `radius` points go, centre first.
fn taps(radius: f32) -> Vec<(f32, f32)> {
    let ring = if radius < 2. { 4 } else { 8 };
    let r = radius * 0.75;
    let mut out = vec![(0., 0.)];
    out.extend((0..ring).map(|k| {
        let a = k as f32 / ring as f32 * std::f32::consts::TAU;
        (r * a.cos(), r * a.sin())
    }));
    out
}

#[derive(Default)]
pub(crate) struct LyricsState {
    depth: Vec<Depth>,
    model: LyricsModel,
    offset: f32,
    tween: Option<Tween>,
    /// The next frame with measured rows jumps straight to the anchor.
    snap: bool,
    /// The scroll range: the first and last rows at the anchor line.
    range: (f32, f32),
    reported: Option<bool>,
    measured: Rc<RefCell<Measured>>,
}

impl LyricsState {
    /// Reports a follow change the app has not heard.
    fn report(&mut self, sink: &Sink, cx: &mut App) {
        let following = self.model.follow.following();
        if self.reported.is_some_and(|r| r != following) {
            sink.send(LyricsFollowChanged { following }, cx);
        }
        self.reported = Some(following);
    }
}

/// Records the bounds of the element it sits in (absolute, filling it).
fn measure(f: impl Fn(gpui::Bounds<Pixels>) + 'static) -> impl IntoElement {
    canvas(move |bounds, _, _| f(bounds), |_, _, _, _| ()).absolute().top_0().left_0().size_full()
}

pub(super) fn lyrics(
    b: &Build,
    id: NodeId,
    p: &LyricsProps,
    marks: &Marks,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let (ctl, state) = b.ctl(id, false, window, cx, LyricsState::default);
    let theme = b.theme();
    let look = ctl.look;
    let now = Instant::now();
    let height = b.backend.bounds(id).map_or(0., |r| r.size.height);
    let mut s = state.borrow_mut();
    if s.model.set_lyrics(&p.lyrics) {
        s.snap = true;
        s.tween = None;
        s.offset = 0.;
        s.depth.clear();
        *s.measured.borrow_mut() = Measured { column: None, rows: vec![None; s.model.rows.len()] };
    }
    s.model.advance(p, now);
    if s.model.follow.tick(now, p.resync_after) {
        s.tween = None;
    }
    s.report(&ctl.sink, cx);
    let synced = s.model.synced();
    let rows = s.model.rows.clone();
    let lit = s.model.lit.clone();
    let time = LyricsModel::time(p, now);

    let measured: Vec<Option<(f32, f32)>> = s.measured.borrow().rows.clone();
    let complete =
        height > 0. && !rows.is_empty() && measured.len() == rows.len() && measured.iter().all(Option::is_some);
    if complete {
        let top = |i: usize| measured[i].map_or(0., |m| m.0);
        s.range = (
            follow_offset(top(0), height),
            follow_offset(top(rows.len() - 1), height).max(follow_offset(top(0), height)),
        );
        if synced && s.model.follow.following() {
            let target = follow_offset(top(lit.anchor.min(rows.len() - 1)), height);
            if s.snap {
                s.offset = target;
                s.tween = None;
                s.snap = false;
            } else if s.tween.as_ref().map_or((s.offset - target).abs() > 0.5, |t| (t.to - target).abs() > 0.5) {
                s.tween = Some(Tween { from: s.offset, to: target, start: now });
            }
        } else if !synced {
            s.range = (0., (top(rows.len() - 1) - height / 2.).max(0.));
            s.offset = s.offset.clamp(s.range.0, s.range.1);
        }
    } else if !rows.is_empty() {
        window.request_animation_frame();
    }
    if let Some(&Tween { from, to, start }) = s.tween.as_ref() {
        let x = now.saturating_duration_since(start).as_secs_f32() / SCROLL.as_secs_f32();
        s.offset = from + (to - from) * bezier(SCROLL_CURVE, x.min(1.));
        if x >= 1. {
            s.offset = to;
            s.tween = None;
        } else {
            window.request_animation_frame();
        }
    }
    if p.clock.playing {
        window.request_animation_frame();
    }
    let offset = s.offset;
    let following = s.model.follow.following();
    let measured_rows = s.measured.clone();
    drop(s);

    let (size, weight, line) = look.type_of(TextStyle::Title2);
    let mut root =
        div().id(ElementId::Integer(id.0)).role(gpui::Role::List).aria_label("Lyrics").relative().overflow_hidden();
    if rows.is_empty() {
        let ink = theme.text.secondary;
        marks.text(&p.empty_text, ink, &[], look.text_size, FontWeight::NORMAL);
        root = root
            .flex()
            .items_center()
            .justify_center()
            .text_size(look.text_size)
            .text_color(ink)
            .child(p.empty_text.clone());
        return with_style(root, &p.style).into_any_element();
    }

    // Rows filling each side of the anchor line, for the depth fade.
    let avg = {
        let m = measured_rows.borrow();
        let known: Vec<f32> = m.rows.iter().flatten().map(|r| r.1).collect();
        if known.is_empty() { line.as_f32() + 12. } else { known.iter().sum::<f32>() / known.len() as f32 }
    };
    let above = (anchor_y(height) / avg).max(1.);
    let below = ((height - anchor_y(height)) / avg).max(1.);

    let (primary, secondary) = (theme.text.primary, theme.text.secondary);
    let column_measure = {
        let m = measured_rows.clone();
        measure(move |b| m.borrow_mut().column = Some(b.origin.y.as_f32()))
    };
    let mut column = div().absolute().left_0().right_0().top(px(-offset)).flex().flex_col().child(column_measure);
    let motion = theme.motion;
    let duration = motion.slow.as_secs_f32();
    let mut depth = std::mem::take(&mut state.borrow_mut().depth);
    let mut easing = false;
    for (i, row) in rows.iter().enumerate() {
        let is_lit = synced && lit.contains(i);
        let target = if synced {
            let span = if i < lit.anchor { above } else { below };
            row_depth(is_lit, i.abs_diff(lit.anchor), span, following)
        } else {
            (1., 0.)
        };
        if depth.len() <= i {
            depth.push(Depth { from: target, to: target, start: now });
        } else if depth[i].to != target {
            depth[i] = Depth { from: depth[i].at(now, duration, motion.ease), to: target, start: now };
        }
        let (fade, blur) = depth[i].at(now, duration, motion.ease);
        easing |= (fade, blur) != target;
        let on_screen = measured_rows.borrow().rows.get(i).copied().flatten().is_none_or(|(top, h)| {
            let y = top - offset;
            y + h > 0. && y < height
        });
        let ink = |c: Hsla| with_alpha(c, c.a * fade);
        let (row_size, row_weight) = match (row.kind, synced, is_lit) {
            (_, false, _) => (look.type_of(TextStyle::Title3).0, FontWeight::BOLD),
            (LyricRowKind::Background, _, _) => (size * 0.7, FontWeight::MEDIUM),
            (_, true, true) => (size, weight),
            (_, true, false) => (size * UNLIT_SCALE, weight),
        };
        let mut el = div()
            .id(ElementId::Integer(i as u64))
            .relative()
            .w_full()
            .py(px(6.))
            .text_size(row_size)
            .font_weight(row_weight);
        el = if row.opposite { el.flex().flex_row().justify_end() } else { el.flex().flex_row() };
        let rest = if synced && !is_lit { secondary } else { primary };
        match row.kind {
            LyricRowKind::Interlude => {
                let fill = interlude_fill(row, time);
                let mut dots = div().flex().flex_row().gap(px(6.)).py(px(6.));
                for d in 0..3 {
                    let on = is_lit && fill >= (d + 1) as f32 / 3.;
                    let rest = if on { primary } else { secondary };
                    let c = ink(rest);
                    marks.fill("lyrics interlude", rest, &[]);
                    dots = dots.child(div().size(px(8.)).rounded_full().bg(c));
                }
                el = el.child(dots);
            }
            _ if is_lit && i == lit.anchor && !row.words.is_empty() => {
                let mut words = div().flex().flex_row().flex_wrap().when(row.opposite, |w| w.justify_end());
                for word in &row.words {
                    let f = word_fill(word, time);
                    let text: SharedString = word.text.clone();
                    let gap = if word.space_after { row_size * 0.28 } else { px(0.) };
                    let w = if f >= 1. || f <= 0. {
                        let at_rest = if f >= 1. { primary } else { secondary };
                        let c = ink(at_rest);
                        marks.text(&text, at_rest, &[], row_size, row_weight);
                        div().text_color(c).child(text)
                    } else {
                        marks.text(&text, secondary, &[], row_size, row_weight);
                        marks.text(&text, primary, &[], row_size, row_weight);
                        div().relative().text_color(ink(secondary)).child(text.clone()).child(
                            div()
                                .absolute()
                                .top_0()
                                .left_0()
                                .h_full()
                                .w(relative(f))
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_color(ink(primary))
                                .child(text),
                        )
                    };
                    words = words.child(w.mr(gap));
                }
                el = el.child(words);
            }
            _ => {
                // Recorded at rest: the depth fade is decoration around the lit line, as the
                // AppKit view reports it.
                marks.text(&row.text, rest, &[], row_size, row_weight);
                let line = |c: Hsla| div().text_color(c).when(row.opposite, |d| d.text_right()).child(row.text.clone());
                if blur > 0.25 && on_screen {
                    let taps = taps(blur);
                    // Each copy's share, so the copies together cover a glyph at the row's opacity.
                    let alpha = 1. - (1. - (rest.a * fade).min(0.999)).powf(1. / taps.len() as f32);
                    let mut stack = div().relative().w_full().child(line(gpui::transparent_black()));
                    for (dx, dy) in taps {
                        stack = stack.child(line(with_alpha(rest, alpha)).absolute().top(px(dy)).left(px(dx)).w_full());
                    }
                    el = el.child(stack);
                } else {
                    el = el.child(line(ink(rest)));
                }
            }
        }
        let m = measured_rows.clone();
        el = el.child(measure(move |b| {
            let mut m = m.borrow_mut();
            let Some(column) = m.column else { return };
            if let Some(slot) = m.rows.get_mut(i) {
                *slot = Some((b.origin.y.as_f32() - column, b.size.height.as_f32()));
            }
        }));
        if synced {
            let (sink, line, start) = (ctl.sink.clone(), row.line, Duration::from_secs_f64(row.start.max(0.)));
            el = el.cursor_pointer().on_click(move |_, _, cx| sink.send(LyricClicked { line, start }, cx));
        }
        column = column.child(el);
    }
    state.borrow_mut().depth = depth;
    if easing {
        window.request_animation_frame();
    }

    let wheel = {
        let (sink, state) = (ctl.sink.clone(), state.clone());
        move |event: &ScrollWheelEvent, window: &mut Window, cx: &mut App| {
            let delta = event.delta.pixel_delta(px(20.)).y.as_f32();
            if delta == 0. {
                return;
            }
            cx.stop_propagation();
            let mut s = state.borrow_mut();
            s.model.follow.user_scrolled(Instant::now());
            s.tween = None;
            s.offset = (s.offset - delta).clamp(s.range.0, s.range.1);
            s.report(&sink, cx);
            window.refresh();
        }
    };
    root = root.child(column).on_scroll_wheel(wheel);
    if synced && !following {
        let fill = theme.surface.overlay;
        marks.fill("lyrics resync", fill, &[]);
        marks.fill("lyrics resync glyph", primary, &[fill]);
        let (sink, state) = (ctl.sink.clone(), state.clone());
        root = root.child(
            div()
                .id("resync")
                .role(gpui::Role::Button)
                .aria_label("Follow playback")
                .absolute()
                .right(px(12.))
                .bottom(px(12.))
                .h(px(32.))
                .w(px(44.))
                .rounded_full()
                .bg(fill)
                .border_1()
                .border_color(theme.border.subtle)
                .flex()
                .items_center()
                .justify_center()
                .occlude()
                .cursor_pointer()
                .hover(move |s| s.bg(theme.hover(fill)))
                .child(glyph(Glyph::LocateFixed, px(16.), primary, look.glyph_stroke))
                .on_click(move |_, window, cx| {
                    let mut s = state.borrow_mut();
                    s.model.follow.resync();
                    s.report(&sink, cx);
                    window.refresh();
                }),
        );
    }
    with_style(root, &p.style).into_any_element()
}
