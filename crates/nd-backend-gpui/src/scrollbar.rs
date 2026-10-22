//! Drawn scroll bars over nd's scroll views, for the looks that show one: WinUI's ScrollBar under
//! the Fluent preset (ScrollBar_themeresources.xaml). A thin 2 px indicator shows while the view
//! scrolls or the pointer moves over it and fades 2 s after the last of either; under the pointer
//! the bar widens to its 12 px track with a 6 px thumb and caret buttons (0.4 s in, 167 ms on
//! the "fast out, slow in" curve), and contracts 0.5 s after the pointer leaves. The thumb drags,
//! the track pages by a viewport and the carets step.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use nd_core::{NodeId, Point, WindowId};
use nd_gpui::gpui::{
    self, AnyElement, App, DispatchPhase, ElementId, Hsla, InteractiveElement, IntoElement, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, StatefulInteractiveElement, Styled, Window, canvas,
    div, px,
};
use nd_theme::color::with_alpha;
use nd_theme::{Preset, Theme};

use crate::backend::GpuiBackend;
use crate::forms::look::cubic_bezier;

/// One look's scroll bar.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ScrollbarLook {
    /// Width of the expanded bar, and the height of each caret button (ScrollBarSize).
    pub size: f32,
    /// Visible width of the thumb collapsed and expanded: the thumb's width less its 6 px
    /// transparent stroke (ScrollBarVerticalThumbMinWidth 8 and ScrollBarSize 12).
    pub thin: f32,
    pub thick: f32,
    pub min_thumb: f32,
    pub thumb: Hsla,
    pub track: Hsla,
    pub caret: Hsla,
    /// Seconds the indicator stays after the last scroll or pointer move, and its fade.
    pub linger: f64,
    pub fade: f64,
    /// Delay and duration of the expansion under the pointer, and of the contraction after it.
    pub expand_delay: f64,
    pub contract_delay: f64,
    pub expand: f64,
    /// A caret press scrolls this far.
    pub step: f32,
}

impl ScrollbarLook {
    pub fn of(theme: &Theme) -> Option<Self> {
        if theme.preset != Preset::Fluent {
            return None;
        }
        let dark = theme.is_dark();
        // ControlStrongFillColorDefault for the thumb and carets, AcrylicInAppFillColorDefault
        // behind the expanded track.
        let strong = if dark { with_alpha(gpui::white(), 0.545) } else { with_alpha(gpui::black(), 0.447) };
        let track: Hsla = if dark { gpui::rgba(0x2c2c2cf2).into() } else { gpui::rgba(0xfcfcfcf2).into() };
        Some(Self {
            size: 12.,
            thin: 2.,
            thick: 6.,
            min_thumb: 30.,
            thumb: strong,
            track,
            caret: strong,
            linger: 2.,
            fade: 0.083,
            expand_delay: 0.4,
            contract_delay: 0.5,
            expand: 0.167,
            step: 16.,
        })
    }
}

/// One scroll bar's state between frames. Times are the frame clock's seconds.
#[derive(Clone, Copy, Debug, Default)]
struct Bar {
    offset: f32,
    /// Last scroll or pointer move over the view.
    active_at: f64,
    /// A pointer move arrived since the last frame; the next frame stamps it.
    moved: bool,
    over_bar: bool,
    expanded: bool,
    /// When `expanded` last flipped, and how far the bar was then.
    flipped_at: f64,
    from: f32,
    /// The pointer's distance from the thumb's top while dragging it.
    grab: Option<f32>,
    /// The bar's top in the window, as last painted.
    origin_y: Option<f32>,
    /// When a pointer move last asked for a frame.
    nudged: Option<nd_core::time::Instant>,
}

impl Bar {
    fn expansion(&self, look: &ScrollbarLook, now: f64) -> f32 {
        let (target, delay) = if self.expanded { (1., look.expand_delay) } else { (0., look.contract_delay) };
        let t = ((now - self.flipped_at - delay) / look.expand).clamp(0., 1.) as f32;
        self.from + (target - self.from) * cubic_bezier([0., 0., 0., 1.], t)
    }
}

/// Every scroll bar's state, by its scroll view, and the GPUI scroll handle of each view GPUI
/// scrolls itself.
#[derive(Clone, Default)]
pub(crate) struct Scrollbars {
    bars: Rc<RefCell<HashMap<NodeId, Bar>>>,
    handles: Rc<RefCell<HashMap<NodeId, gpui::ScrollHandle>>>,
}

impl Scrollbars {
    fn update(&self, id: NodeId, f: impl FnOnce(&mut Bar)) {
        f(self.bars.borrow_mut().entry(id).or_default());
    }

    fn handle(&self, id: NodeId) -> gpui::ScrollHandle {
        self.handles.borrow_mut().entry(id).or_default().clone()
    }

    /// The pointer moved over the scroll view: the indicator shows. Returns whether that needs
    /// a frame: at most two a second, which keeps a lingering indicator up without redrawing
    /// the window on every move.
    pub fn pointer_moved(&self, id: NodeId) -> bool {
        let now = nd_core::time::Instant::now();
        let mut redraw = false;
        self.update(id, |b| {
            b.moved = true;
            if b.nudged.is_none_or(|at| now.duration_since(at).as_secs_f64() > 0.5) {
                b.nudged = Some(now);
                redraw = true;
            }
        });
        redraw
    }

    pub fn forget(&self, id: NodeId) {
        self.bars.borrow_mut().remove(&id);
    }
}

/// Where a bar reads and moves its view's offset: nd's gesture router, or GPUI's own scrolling
/// (`overflow_y_scroll`), whose offset runs negative.
#[derive(Clone)]
pub(crate) enum Source {
    Gestures,
    Handle(gpui::ScrollHandle),
}

/// What the bar needs from the frame being built.
pub(crate) struct Frame<'a> {
    pub backend: &'a GpuiBackend,
    pub source: Source,
    pub window: WindowId,
    pub node: NodeId,
    pub time: f64,
    pub height: f32,
    pub reduce_motion: bool,
}

/// libadwaita's cues at a vertical scroll view's edges, for a view scrolled `offset` of `max`
/// points: a raised header bar's shadow along the top once content has scrolled under it
/// (`toolbarview > .top-bar.raised`), and the undershoot shade along the bottom while more
/// content waits below (`undershoot.bottom`). Nothing on other looks.
pub(crate) fn adwaita_edges(theme: &Theme, offset: f32, max: f32) -> Vec<AnyElement> {
    if theme.preset != Preset::Adwaita || max <= 0. {
        return Vec::new();
    }
    let dark = theme.is_dark();
    // --headerbar-shade-color and --shade-color: RGB(0 0 6) at 12% and 7% light, 36% and 25% dark.
    let tint = |alpha: f32| -> Hsla { with_alpha(gpui::rgb(0x000006).into(), alpha) };
    let raised = tint(if dark { 0.36 } else { 0.12 } * 0.5);
    let shade = tint(if dark { 0.25 } else { 0.07 } * 0.75);
    let edge = |color: Hsla, top: bool| {
        let line = div().h(px(1.)).w_full().bg(color);
        let fade = div().h(px(4.)).w_full().bg(gpui::linear_gradient(
            if top { 180. } else { 0. },
            gpui::linear_color_stop(color, 0.),
            gpui::linear_color_stop(with_alpha(color, 0.), 1.),
        ));
        let strip = div().absolute().left_0().right_0().flex().flex_col();
        if top { strip.top_0().child(line).child(fade) } else { strip.bottom_0().child(fade).child(line) }
            .into_any_element()
    };
    let mut edges = Vec::new();
    if offset > 0.5 {
        edges.push(edge(raised, true));
    }
    if offset < max - 0.5 {
        edges.push(edge(shade, false));
    }
    edges
}

/// The vertical bar for a scroll view `height` tall, or `None` while it shows nothing.
pub(crate) fn vertical(look: ScrollbarLook, f: Frame<'_>, window: &mut Window) -> Option<AnyElement> {
    let (offset, max) = match &f.source {
        Source::Gestures => f.backend.gestures.scroll_metrics(f.window, f.node)?,
        Source::Handle(h) => {
            let (o, m) = (h.offset(), h.max_offset());
            (Point { x: -o.x.as_f32(), y: -o.y.as_f32() }, Point { x: m.x.as_f32(), y: m.y.as_f32() })
        }
    };
    let bars = f.backend.scrollbars.clone();
    if max.y <= 0. || f.height < look.size * 2. + look.min_thumb {
        bars.forget(f.node);
        return None;
    }
    let now = f.time;
    let mut bar = {
        let mut map = bars.bars.borrow_mut();
        let bar = map.entry(f.node).or_insert_with(|| Bar {
            offset: offset.y,
            active_at: f64::NEG_INFINITY,
            ..Bar::default()
        });
        if bar.moved || (bar.offset - offset.y).abs() > 0.01 {
            bar.active_at = now;
            bar.moved = false;
        }
        bar.offset = offset.y;
        let hot = bar.over_bar || bar.grab.is_some();
        if hot != bar.expanded {
            bar.from = bar.expansion(&look, now);
            bar.expanded = hot;
            bar.flipped_at = now;
        }
        *bar
    };
    if f.reduce_motion {
        bar.from = if bar.expanded { 1. } else { 0. };
        bar.flipped_at = f64::NEG_INFINITY;
    }

    // Shown while hot, and for `linger` after the last activity, then fading.
    let idle = now - bar.active_at;
    let hot = bar.expanded || bar.grab.is_some();
    let opacity = if hot || idle < look.linger {
        1.
    } else {
        (1. - (idle - look.linger) / look.fade.max(1e-6)).clamp(0., 1.) as f32
    };
    let e = bar.expansion(&look, now);
    // Frames while something moves; a timer for the next change that waits.
    let settle = bar.flipped_at + if bar.expanded { look.expand_delay } else { look.contract_delay };
    let animating = (now >= settle && now < settle + look.expand) || (opacity > 0. && opacity < 1.);
    if animating {
        window.request_animation_frame();
    } else {
        let wake = if now < settle {
            Some(settle - now)
        } else if !hot && opacity >= 1. {
            Some(look.linger - idle)
        } else {
            None
        };
        if let Some(after) = wake {
            f.backend.refresh_after(f.window, Duration::from_secs_f64(after.max(0.) + 0.001));
        }
    }
    if opacity <= 0. && e <= 0. {
        return None;
    }

    let size = look.size;
    let track_len = f.height - size * 2.;
    let viewport = f.height;
    let thumb_len = (track_len * viewport / (viewport + max.y)).clamp(look.min_thumb.min(track_len), track_len);
    let travel = (track_len - thumb_len).max(0.);
    let at = (offset.y / max.y).clamp(0., 1.);
    let thumb_top = size + travel * at;
    let width = look.thin + (look.thick - look.thin) * e;

    let (node, window_id) = (f.node, f.window);
    let (gestures, source) = (f.backend.gestures.clone(), f.source.clone());
    let scroll_to = move |y: f32| {
        let y = y.clamp(0., max.y);
        match &source {
            Source::Gestures => gestures.scroll_to(window_id, node, Point { x: offset.x, y }),
            Source::Handle(h) => h.set_offset(gpui::point(px(-offset.x), px(-y))),
        }
    };

    let thumb = div()
        .absolute()
        .top(px(thumb_top))
        // Measured on the WinUI Gallery: the 2 px indicator sits 6 px into the bar, 4 px from its
        // outer edge; the 6 px thumb is centred.
        .left(px(6. + ((size - look.thick) / 2. - 6.) * e))
        .w(px(width))
        .h(px(thumb_len))
        .rounded(px(width / 2.))
        .bg(look.thumb);
    let caret_to = scroll_to.clone();
    let caret = move |name: &'static str, up: bool| {
        let to = caret_to.clone();
        let step = if up { -look.step } else { look.step };
        let mut b = div()
            .id(name)
            .absolute()
            .left_0()
            .w(px(size))
            .h(px(size))
            .flex()
            .items_center()
            .justify_center()
            .opacity(e)
            .child(caret_glyph(up, look.caret))
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                cx.stop_propagation();
                to(offset.y + step);
                window.refresh();
            });
        b = if up { b.top_0() } else { b.bottom_0() };
        b
    };

    let page = {
        let to = scroll_to.clone();
        let bars = bars.clone();
        move |event: &MouseDownEvent, window: &mut Window, cx: &mut App| {
            cx.stop_propagation();
            let Some(bounds) = window_bounds(&bars, node) else { return };
            let y = event.position.y.as_f32() - bounds;
            if y >= thumb_top && y <= thumb_top + thumb_len {
                bars.update(node, |b| b.grab = Some(y - thumb_top));
            } else if y < thumb_top {
                to(offset.y - viewport);
            } else {
                to(offset.y + viewport);
            }
            window.refresh();
        }
    };
    // Window-wide while the thumb is held, so it follows the pointer off the bar.
    let follow = {
        let (bars, to) = (bars.clone(), scroll_to);
        canvas(
            |_, _, _| (),
            move |bounds, _, window, _| {
                bars.update(node, |b| b.origin_y = Some(bounds.origin.y.as_f32()));
                let (moving, released) = (bars.clone(), bars.clone());
                let to = to.clone();
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, _| {
                    let Some(grab) = moving.bars.borrow().get(&node).and_then(|b| b.grab) else { return };
                    if phase != DispatchPhase::Bubble || !event.dragging() {
                        return;
                    }
                    let origin = moving.bars.borrow().get(&node).and_then(|b| b.origin_y).unwrap_or(0.);
                    let top = event.position.y.as_f32() - origin - grab - size;
                    let fraction = if travel > 0. { (top / travel).clamp(0., 1.) } else { 0. };
                    to(fraction * max.y);
                    window.refresh();
                });
                window.on_mouse_event(move |_: &MouseUpEvent, phase, window, _| {
                    if phase == DispatchPhase::Bubble
                        && released.bars.borrow().get(&node).is_some_and(|b| b.grab.is_some())
                    {
                        released.update(node, |b| b.grab = None);
                        window.refresh();
                    }
                });
            },
        )
        .absolute()
        .size_0()
    };

    let hover = bars.clone();
    Some(
        div()
            .id(ElementId::NamedInteger("nd-scrollbar".into(), node.0))
            .absolute()
            .top_0()
            .right_0()
            .w(px(size))
            .h(px(f.height))
            .opacity(opacity)
            .rounded(px(size / 2.))
            .bg(with_alpha(look.track, look.track.a * e))
            .occlude()
            .on_hover(move |over: &bool, window, _| {
                hover.update(node, |b| b.over_bar = *over);
                window.refresh();
            })
            .on_mouse_down(MouseButton::Left, page)
            .child(follow)
            .child(caret("nd-scrollbar-up", true))
            .child(thumb)
            .child(caret("nd-scrollbar-down", false))
            .into_any_element(),
    )
}

fn window_bounds(bars: &Scrollbars, node: NodeId) -> Option<f32> {
    bars.bars.borrow().get(&node).and_then(|b| b.origin_y)
}

/// CaretUpSolid8 or CaretDownSolid8 at 8 px (ScrollBarButtonArrowIconFontSize), from Segoe
/// Fluent Icons where Windows has it, a small chevron elsewhere.
fn caret_glyph(up: bool, color: Hsla) -> AnyElement {
    if cfg!(target_os = "windows") {
        return crate::fluent_icons::glyph(if up { '\u{EDDB}' } else { '\u{EDDC}' }, px(8.), color).into_any_element();
    }
    let g = if up { crate::forms::glyph::Glyph::ChevronUp } else { crate::forms::glyph::Glyph::ChevronDown };
    crate::forms::glyph::glyph(g, px(8.), color, px(1.5))
}

/// How a GPUI-scrolled view sits in its parent, which its wrapper takes over.
#[derive(Clone, Copy)]
pub(crate) enum Fit {
    Fill,
    /// The rest of a flex column.
    Flex,
}

impl crate::render::Build<'_> {
    /// A view GPUI scrolls (`overflow_y_scroll`, keyed by its node) with the look's drawn bar over
    /// it. The bar sits in a wrapper beside the view so it does not scroll with the content.
    pub(crate) fn scrolled(
        &self,
        id: NodeId,
        view: gpui::Stateful<gpui::Div>,
        fit: Fit,
        window: &mut Window,
    ) -> AnyElement {
        fn place<E: Styled>(e: E, fit: Fit) -> E {
            match fit {
                Fit::Fill => e.size_full(),
                Fit::Flex => e.flex_1().min_h_0(),
            }
        }
        let theme = self.theme();
        let Some(look) = ScrollbarLook::of(&theme) else {
            let handle = self.backend.scrollbars.handle(id);
            let edges = adwaita_edges(&theme, -handle.offset().y.as_f32(), handle.max_offset().y.as_f32());
            if edges.is_empty() && theme.preset != Preset::Adwaita {
                return place(view, fit).into_any_element();
            }
            return place(div(), fit)
                .relative()
                .child(view.track_scroll(&handle).size_full())
                .children(edges)
                .into_any_element();
        };
        let bars = self.backend.scrollbars.clone();
        let handle = bars.handle(id);
        let height = handle.bounds().size.height.as_f32();
        let frame = Frame {
            backend: self.backend,
            source: Source::Handle(handle.clone()),
            window: self.window_id,
            node: id,
            time: self.time,
            height,
            reduce_motion: self.backend.platform_reduce_motion() == Some(true),
        };
        let bar = vertical(look, frame, window);
        let moved = bars.clone();
        place(div(), fit)
            .relative()
            .on_mouse_move(move |_: &MouseMoveEvent, window, _| {
                if moved.pointer_moved(id) {
                    window.refresh();
                }
            })
            .child(view.track_scroll(&handle).size_full())
            .children(bar)
            .into_any_element()
    }
}
