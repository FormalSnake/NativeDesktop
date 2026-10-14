//! The drawn `split` on GPUI: the two panes at the flex weights the component derives from the
//! ratio, a hairline divider between them, and a wider grab area over the divider that the user
//! drags. The ratio is controlled: every frame draws the ratio in the props, a drag only reports
//! [`SplitResized`], so a ratio the app refuses snaps back on the next frame.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use nd_components::split::Axis;
use nd_components::{SplitKind, SplitPaneKind, SplitPaneProps, SplitProps, SplitResized, StatusPageKind};
use nd_core::{NodeId, Rect};
use nd_gpui::gpui::{
    self, AnyElement, DispatchPhase, ElementId, InteractiveElement, IntoElement, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, Refineable, Styled, canvas, div, px,
};
use nd_ui::backend::WidgetKind;
use nd_ui::events::Emit;

use crate::backend::{Node, Wake};
use crate::render::Build;

/// A divider drag in progress, by split node; gone once the button is released.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SplitDrag {
    /// Where on the divider the press landed, from the first pane's trailing edge.
    offset: f32,
    /// The last ratio reported, or the one shown when the drag began.
    reported: f32,
}

pub(crate) type SplitDrags = Rc<RefCell<HashMap<NodeId, SplitDrag>>>;

/// The divider's thickness in the layout.
const HAIRLINE: f32 = 1.;

fn is<W: 'static>(kind: WidgetKind) -> bool {
    kind.type_id == std::any::TypeId::of::<W>()
}

/// What a drag needs to turn a pointer position into a ratio.
#[derive(Clone, Copy, Debug)]
struct Geometry {
    node: NodeId,
    horizontal: bool,
    ratio: f32,
    first: (Option<f32>, Option<f32>),
    second: (Option<f32>, Option<f32>),
}

impl Geometry {
    /// The split's start and length along its axis, in window coordinates.
    fn main(&self, b: Rect) -> (f32, f32) {
        if self.horizontal { (b.origin.x, b.size.width) } else { (b.origin.y, b.size.height) }
    }

    fn along(&self, p: gpui::Point<Pixels>) -> f32 {
        if self.horizontal { p.x.as_f32() } else { p.y.as_f32() }
    }

    /// The first pane's size as last laid out, else the one the ratio gives.
    fn first_size(&self, bounds: &HashMap<NodeId, Rect>, first: Option<NodeId>, available: f32) -> f32 {
        first
            .and_then(|f| bounds.get(&f))
            .map(|b| if self.horizontal { b.size.width } else { b.size.height })
            .unwrap_or(self.ratio * available)
    }

    /// The ratio a first pane of `size` stands for, inside both panes' minimum and maximum.
    fn ratio_for(&self, size: f32, available: f32) -> f32 {
        let lo = self.first.0.unwrap_or(0.).max(available - self.second.1.unwrap_or(f32::INFINITY));
        let hi = self.first.1.unwrap_or(f32::INFINITY).min(available - self.second.0.unwrap_or(0.));
        // Constraints that cannot both hold leave the first pane its minimum.
        let size = if lo > hi { lo.min(available) } else { size.clamp(lo, hi) };
        (size / available.max(1.)).clamp(0., 1.)
    }
}

fn pane_props(build: &Build, id: Option<&NodeId>) -> SplitPaneProps {
    id.and_then(|c| build.nodes.get(c))
        .and_then(|n| n.props.downcast_ref::<SplitPaneProps>())
        .cloned()
        .unwrap_or_default()
}

struct Drag {
    geometry: Geometry,
    first: Option<NodeId>,
    drags: SplitDrags,
    bounds: Rc<RefCell<HashMap<NodeId, Rect>>>,
    emit: Emit,
    wake: Wake,
}

impl Drag {
    fn press(&self, at: gpui::Point<Pixels>) {
        let g = &self.geometry;
        let bounds = self.bounds.borrow();
        let Some(split) = bounds.get(&g.node).copied() else { return };
        let (start, length) = g.main(split);
        let available = (length - HAIRLINE).max(1.);
        let first = g.first_size(&bounds, self.first, available);
        let offset = g.along(at) - start - first;
        self.drags.borrow_mut().insert(g.node, SplitDrag { offset, reported: g.ratio });
    }

    fn moved(&self, at: gpui::Point<Pixels>, cx: &mut gpui::App) {
        let g = &self.geometry;
        let Some(drag) = self.drags.borrow().get(&g.node).copied() else { return };
        let Some(split) = self.bounds.borrow().get(&g.node).copied() else { return };
        let (start, length) = g.main(split);
        let available = (length - HAIRLINE).max(1.);
        let ratio = g.ratio_for(g.along(at) - start - drag.offset, available);
        if ratio == drag.reported {
            return;
        }
        if let Some(d) = self.drags.borrow_mut().get_mut(&g.node) {
            d.reported = ratio;
        }
        self.emit.emit(SplitResized { ratio });
        (self.wake)(cx);
    }
}

impl Build<'_> {
    /// The drawn split, split pane and status page; `None` for any other kind.
    pub(crate) fn layout_widget(
        &self,
        id: NodeId,
        kind: WidgetKind,
        node: &Node,
        window: &mut gpui::Window,
        cx: &mut gpui::App,
    ) -> Option<AnyElement> {
        if is::<SplitKind>(kind) {
            return self.split(id, node, window, cx);
        }
        if is::<SplitPaneKind>(kind) {
            return self.split_pane(id, node, window, cx);
        }
        if is::<StatusPageKind>(kind) {
            return self.status_page(id, node);
        }
        None
    }

    fn split(&self, id: NodeId, node: &Node, window: &mut gpui::Window, cx: &mut gpui::App) -> Option<AnyElement> {
        let p = node.props.downcast_ref::<SplitProps>()?;
        let horizontal = p.axis == Axis::Horizontal;
        let theme = self.theme();
        let (first, second) = (node.children.first().copied(), node.children.get(1).copied());
        let (a, b) = (pane_props(self, first.as_ref()), pane_props(self, second.as_ref()));
        let geometry = Geometry {
            node: id,
            horizontal,
            ratio: p.ratio.clamp(0., 1.),
            first: (a.min, a.max),
            second: (b.min, b.max),
        };
        let drag = Rc::new(Drag {
            geometry,
            first,
            drags: self.backend.splits.clone(),
            bounds: self.backend.bounds.clone(),
            emit: self.backend.state.borrow().events.emitter(id),
            wake: self.wake.clone(),
        });

        let line = theme.separator;
        self.painted(id, Some(line), None, "split divider", || ());
        let divider = div().flex_none().bg(line);
        let divider = if horizontal { divider.w(px(HAIRLINE)).h_full() } else { divider.h(px(HAIRLINE)).w_full() };

        // The grab area is the container's last child, so it sits above both panes' content.
        let grab = theme.spacing.sm.as_f32();
        let at = {
            let bounds = self.backend.bounds.borrow();
            bounds.get(&id).map(|s| geometry.first_size(&bounds, first, (geometry.main(*s).1 - HAIRLINE).max(1.)))
        };
        let mut handle = div()
            .id(ElementId::Name(format!("nd-split-divider-{}", id.0).into()))
            .absolute()
            .occlude()
            .on_mouse_down(MouseButton::Left, {
                let drag = drag.clone();
                move |event: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    drag.press(event.position);
                }
            });
        let extent = px(grab * 2. + HAIRLINE);
        handle = match (horizontal, at) {
            (true, Some(x)) => handle.top_0().bottom_0().left(px(x - grab)).w(extent).cursor_col_resize(),
            (true, None) => {
                handle.top_0().bottom_0().left(gpui::relative(geometry.ratio)).w(extent).cursor_col_resize()
            }
            (false, Some(y)) => handle.left_0().right_0().top(px(y - grab)).h(extent).cursor_row_resize(),
            (false, None) => {
                handle.left_0().right_0().top(gpui::relative(geometry.ratio)).h(extent).cursor_row_resize()
            }
        };

        // Window-wide while dragging, so the divider follows the pointer past the split.
        let follow = canvas(
            |_, _, _| (),
            move |_, _, window, _| {
                let moved = drag.clone();
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                    if phase == DispatchPhase::Bubble && event.dragging() {
                        moved.moved(event.position, cx);
                    }
                });
                let released = drag.clone();
                window.on_mouse_event(move |_: &MouseUpEvent, phase, _, _| {
                    if phase == DispatchPhase::Bubble {
                        released.drags.borrow_mut().remove(&released.geometry.node);
                    }
                });
            },
        )
        .absolute()
        .size_0();

        let panes: Vec<AnyElement> = [first, second].into_iter().flatten().map(|c| self.node(c, window, cx)).collect();
        let mut panes = panes.into_iter();
        let mut container = div().size_full().relative().flex().min_w_0().min_h_0();
        container.style().refine(&p.style);
        let container = if horizontal { container.flex_row() } else { container.flex_col() };
        Some(
            container
                .children(panes.next())
                .child(divider)
                .children(panes)
                .child(handle)
                .child(follow)
                .into_any_element(),
        )
    }

    fn split_pane(
        &self,
        _id: NodeId,
        node: &Node,
        window: &mut gpui::Window,
        cx: &mut gpui::App,
    ) -> Option<AnyElement> {
        let p = node.props.downcast_ref::<SplitPaneProps>()?;
        let horizontal = node
            .parent
            .and_then(|s| self.nodes.get(&s))
            .and_then(|s| s.props.downcast_ref::<SplitProps>())
            .is_none_or(|s| s.axis == Axis::Horizontal);
        let mut pane = div();
        pane.style().refine(&p.style);
        if let Some(min) = p.min {
            pane = if horizontal { pane.min_w(px(min)) } else { pane.min_h(px(min)) };
        }
        if let Some(max) = p.max {
            pane = if horizontal { pane.max_w(px(max)) } else { pane.max_h(px(max)) };
        }
        let children: Vec<AnyElement> = node.children.iter().map(|c| self.node(*c, window, cx)).collect();
        Some(pane.children(children).into_any_element())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn geometry(first: (Option<f32>, Option<f32>), second: (Option<f32>, Option<f32>)) -> Geometry {
        Geometry { node: NodeId(1), horizontal: true, ratio: 0.5, first, second }
    }

    #[test]
    fn ratio_respects_both_panes_limits() {
        let g = geometry((Some(100.), None), (Some(200.), None));
        assert_eq!(g.ratio_for(50., 1000.), 0.1);
        assert_eq!(g.ratio_for(900., 1000.), 0.8);
        assert_eq!(g.ratio_for(400., 1000.), 0.4);
        let capped = geometry((None, Some(300.)), (None, None));
        assert_eq!(capped.ratio_for(600., 1000.), 0.3);
    }
}
