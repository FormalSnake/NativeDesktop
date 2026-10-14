//! `shelf`: a GPUI horizontal scroll area over the cards, taking only sideways wheels, with
//! paging arrows over its edges. The position lives in a GPUI `ScrollHandle` per node and is
//! reported to the app's [`ShelfHandle`] every frame; the handle's requests land through
//! [`scroll_target`], and a scroll that stopped settles on a card edge.

use nd_components::{ShelfArrows, ShelfProps, ShelfScrolled, scroll_target, snap_offset};
use nd_core::time::{Duration, Instant};
use nd_core::{NodeId, Rect};
use nd_gpui::gpui::{
    self, AnyElement, App, DispatchPhase, ElementId, InteractiveElement, IntoElement, MouseMoveEvent, ParentElement,
    ScrollHandle, ScrollWheelEvent, StatefulInteractiveElement, Styled, Window, canvas, div, point, px,
};

use crate::forms::glyph::{Glyph, glyph};
use crate::nav::Marks;
use crate::render::Build;

/// How long after the last sideways wheel a scroll counts as ended.
const SETTLE: Duration = Duration::from_millis(150);
/// Paging and settling glide over this long.
const GLIDE: Duration = Duration::from_millis(300);
const ARROW: f32 = 32.;
const ARROW_INSET: f32 = 8.;

struct Glide {
    from: f32,
    to: f32,
    start: Instant,
}

#[derive(Default)]
pub(crate) struct ShelfState {
    scroll: ScrollHandle,
    hovered: bool,
    flags: Option<(bool, bool)>,
    glide: Option<Glide>,
    wheeled: Option<Instant>,
}

pub(super) fn shelf(
    b: &Build,
    id: NodeId,
    p: &ShelfProps,
    children: Vec<AnyElement>,
    marks: &Marks,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let (ctl, state) = b.ctl(id, false, window, cx, ShelfState::default);
    let theme = b.theme();
    let now = Instant::now();
    let mut s = state.borrow_mut();
    let viewport = s.scroll.bounds();
    let width = viewport.size.width.as_f32();
    let max = s.scroll.max_offset().x.as_f32().abs();
    let mut offset = -s.scroll.offset().x.as_f32();
    let cards: Vec<Rect> = (0..s.scroll.children_count())
        .filter_map(|i| s.scroll.bounds_for_item(i))
        .map(|c| {
            let x = (c.origin.x - viewport.origin.x).as_f32();
            Rect::new(x, 0., c.size.width.as_f32(), c.size.height.as_f32())
        })
        .collect();
    let inset = cards.first().map_or(0., |c| c.origin.x);
    if width <= 0. {
        // Not laid out yet: the position, arrows and requests need the next frame's bounds.
        window.request_animation_frame();
    } else {
        if let Some((seq, request)) = p.handle.pending() {
            let to = scroll_target(request, offset, width, max, &cards, inset);
            s.glide = Some(Glide { from: offset, to, start: now });
            s.wheeled = None;
            p.handle.applied(seq);
        }
        if p.snap && s.glide.is_none() && s.wheeled.is_some_and(|t| now.saturating_duration_since(t) >= SETTLE) {
            s.wheeled = None;
            let to = snap_offset(offset, &cards, inset, max, 0);
            if (to - offset).abs() > 0.5 {
                s.glide = Some(Glide { from: offset, to, start: now });
            }
        } else if s.wheeled.is_some() {
            window.request_animation_frame();
        }
        if let Some(g) = &s.glide {
            let t = (now.saturating_duration_since(g.start).as_secs_f32() / GLIDE.as_secs_f32()).min(1.);
            let eased = 1. - (1. - t).powi(3);
            offset = (g.from + (g.to - g.from) * eased).clamp(0., max);
            s.scroll.set_offset(point(px(-offset), px(0.)));
            if t >= 1. {
                s.glide = None;
            } else {
                window.request_animation_frame();
            }
        }
        p.handle.report(offset, max, width);
        let flags = (p.handle.can_page_back(), p.handle.can_page_forward());
        if s.flags != Some(flags) {
            if s.flags.is_some() || flags != (false, false) {
                let event = ShelfScrolled { offset, can_page_back: flags.0, can_page_forward: flags.1 };
                ctl.sink.send(event, cx);
            }
            s.flags = Some(flags);
        }
    }
    let shows = match p.arrows {
        ShelfArrows::Always => true,
        ShelfArrows::Hover => s.hovered,
        ShelfArrows::Never => false,
    };
    let (back, forward) = s.flags.unwrap_or_default();
    let scroll = s.scroll.clone();
    drop(s);

    let mut scroller = div().id(ElementId::Integer(id.0)).track_scroll(&scroll);
    gpui::Refineable::refine(scroller.style(), &p.style);
    scroller.style().restrict_scroll_to_axis = Some(true);
    let wheel = {
        let state = state.clone();
        move |event: &ScrollWheelEvent, window: &mut Window, _: &mut App| {
            if event.delta.pixel_delta(px(20.)).x != px(0.) {
                let mut s = state.borrow_mut();
                s.wheeled = Some(Instant::now());
                s.glide = None;
                window.refresh();
            }
        }
    };
    let scroller = scroller.on_scroll_wheel(wheel).children(children);

    let fill = theme.surface.overlay;
    let ink = theme.text.primary;
    let arrow = |key: &'static str, g: Glyph, direction: i32, name: &'static str| {
        marks.fill("shelf arrow", fill, &[]);
        marks.fill("shelf arrow glyph", ink, &[fill]);
        let handle = p.handle.clone();
        let hover = theme.hover(fill);
        let strip = div().absolute().top_0().bottom_0().flex().items_center();
        let strip = if direction < 0 { strip.left(px(ARROW_INSET)) } else { strip.right(px(ARROW_INSET)) };
        strip.child(
            div()
                .id(key)
                .role(gpui::Role::Button)
                .aria_label(name)
                .size(px(ARROW))
                .rounded_full()
                .bg(fill)
                .border_1()
                .border_color(theme.border.subtle)
                .flex()
                .items_center()
                .justify_center()
                .occlude()
                .cursor_pointer()
                .hover(move |s| s.bg(hover))
                .child(glyph(g, px(16.), ink, ctl.look.glyph_stroke))
                .on_click(move |_, window, _| {
                    handle.page(direction);
                    window.refresh();
                }),
        )
    };
    // The pointer over the shelf, arrows included, from the window's moves: the arrows occlude
    // the shelf's own hitbox, so its hover state would drop under them.
    let tracker = {
        let state = state.clone();
        canvas(
            |_, _, _| (),
            move |bounds, _, window, _| {
                let state = state.clone();
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, _| {
                    if phase != DispatchPhase::Bubble {
                        return;
                    }
                    let inside = bounds.contains(&event.position);
                    let mut s = state.borrow_mut();
                    if s.hovered != inside {
                        s.hovered = inside;
                        window.refresh();
                    }
                });
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full()
    };
    div()
        .relative()
        .w_full()
        .min_w_0()
        .child(scroller)
        .child(tracker)
        .children((shows && back).then(|| arrow("back", Glyph::ChevronLeft, -1, "Scroll back")))
        .children((shows && forward).then(|| arrow("forward", Glyph::ChevronRight, 1, "Scroll forward")))
        .into_any_element()
}
