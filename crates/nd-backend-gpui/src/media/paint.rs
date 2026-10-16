//! `canvas` on GPUI: the display list `nd-ui` recorded, painted as quads and paths at the
//! canvas's bounds, images through the shared pipeline. GPUI lays the canvas out, so a size it
//! was not painted for is reported back and `nd-ui` paints it again.

use std::cell::RefCell;
use std::f32::consts::TAU;
use std::rc::Rc;

use nd_components::{DecodeRequest, ImageFit, ImageSource};
use nd_core::NodeId;
use nd_gpui::gpui::{
    self, AnyElement, App, Bounds, Hsla, IntoElement, ParentElement, PathBuilder, Pixels, Point, Refineable,
    StyleRefinement, Styled, Window, div, fill, point, px, quad, size, transparent_black,
};
use nd_ui::{CanvasPath, CanvasProps, CanvasRect, CanvasResized, PaintOp, PathStep};

use super::image::{Picture, picture};
use crate::nav::Marks;
use crate::render::Build;

fn at(origin: Point<Pixels>, (x, y): (f32, f32)) -> Point<Pixels> {
    point(origin.x + px(x), origin.y + px(y))
}

fn rect(origin: Point<Pixels>, r: CanvasRect) -> Bounds<Pixels> {
    Bounds::new(at(origin, (r.x, r.y)), size(px(r.width), px(r.height)))
}

fn path(origin: Point<Pixels>, steps: &CanvasPath, mut builder: PathBuilder) -> Option<gpui::Path<Pixels>> {
    for step in &steps.0 {
        match *step {
            PathStep::MoveTo(x, y) => builder.move_to(at(origin, (x, y))),
            PathStep::LineTo(x, y) => builder.line_to(at(origin, (x, y))),
            PathStep::QuadTo { ctrl, to } => builder.curve_to(at(origin, to), at(origin, ctrl)),
            PathStep::CubicTo { ctrl1, ctrl2, to } => {
                builder.cubic_bezier_to(at(origin, to), at(origin, ctrl1), at(origin, ctrl2))
            }
            PathStep::Close => builder.close(),
        }
    }
    builder.build().ok()
}

/// The arc as a polyline: from `start` to `end`, fractions of a turn clockwise from 12 o'clock.
fn arc(center: (f32, f32), radius: f32, start: f32, end: f32) -> CanvasPath {
    let steps = (((end - start).abs() * 96.).ceil() as usize).max(2);
    let mut out = CanvasPath::new();
    for i in 0..=steps {
        let t = start + (end - start) * i as f32 / steps as f32;
        let a = t * TAU;
        let p = point(px(center.0 + radius * a.sin()), px(center.1 - radius * a.cos()));
        out = if i == 0 { out.move_to(p) } else { out.line_to(p) };
    }
    out
}

fn paint_ops(ops: &[PaintOp], origin: Point<Pixels>, window: &mut Window) {
    for op in ops {
        match op {
            PaintOp::FillRect { rect: r, color, radius } => {
                window.paint_quad(fill(rect(origin, *r), *color).corner_radii(px(*radius)));
            }
            PaintOp::StrokeRect { rect: r, color, width, radius } => {
                window.paint_quad(quad(
                    rect(origin, *r),
                    px(*radius),
                    transparent_black(),
                    px(*width),
                    *color,
                    Default::default(),
                ));
            }
            PaintOp::FillCircle { center, radius, color } => {
                let r =
                    CanvasRect { x: center.0 - radius, y: center.1 - radius, width: 2. * radius, height: 2. * radius };
                window.paint_quad(fill(rect(origin, r), *color).corner_radii(px(*radius)));
            }
            PaintOp::StrokeArc { center, radius, start, end, width, color } => {
                if let Some(p) = path(origin, &arc(*center, *radius, *start, *end), PathBuilder::stroke(px(*width))) {
                    window.paint_path(p, *color);
                }
            }
            PaintOp::Line { from, to, width, color } => {
                let line = CanvasPath::new().move_to(point(px(from.0), px(from.1))).line_to(point(px(to.0), px(to.1)));
                if let Some(p) = path(origin, &line, PathBuilder::stroke(px(*width))) {
                    window.paint_path(p, *color);
                }
            }
            PaintOp::FillPath { path: steps, color } => {
                if let Some(p) = path(origin, steps, PathBuilder::fill()) {
                    window.paint_path(p, *color);
                }
            }
            PaintOp::StrokePath { path: steps, width, color } => {
                if let Some(p) = path(origin, steps, PathBuilder::stroke(px(*width))) {
                    window.paint_path(p, *color);
                }
            }
            // Drawn as children, over the shapes.
            PaintOp::Image { .. } => {}
        }
    }
}

/// The colour each fill op paints, for the appearance audit.
fn fills(ops: &[PaintOp]) -> Vec<Hsla> {
    ops.iter()
        .filter_map(|op| match op {
            PaintOp::FillRect { color, .. } | PaintOp::FillCircle { color, .. } | PaintOp::FillPath { color, .. } => {
                Some(*color)
            }
            _ => None,
        })
        .collect()
}

pub(crate) fn canvas(b: &Build, id: NodeId, p: &CanvasProps, window: &mut Window, cx: &mut App) -> AnyElement {
    let marks = Marks::default();
    for color in fills(&p.ops) {
        marks.fill("canvas", color, &[]);
    }
    b.record_marks(id, marks.take());
    let images: Vec<(ImageSource, CanvasRect)> = p
        .ops
        .iter()
        .filter_map(|op| match op {
            PaintOp::Image { source, rect } => Some((source.get::<ImageSource>()?.clone(), *rect)),
            _ => None,
        })
        .collect();
    let (_, pictures) = b.ctl(id, false, window, cx, Vec::<Rc<RefCell<Picture>>>::new);
    let app = b.backend.state.borrow().app.clone();
    let slots: Vec<Rc<RefCell<Picture>>> = {
        let mut slots = pictures.borrow_mut();
        slots.resize_with(images.len(), Rc::default);
        slots.truncate(images.len());
        for slot in slots.iter() {
            slot.borrow_mut().attach(None, app.clone());
        }
        slots.clone()
    };
    let mut children = Vec::new();
    for ((source, r), slot) in images.into_iter().zip(slots) {
        let request = DecodeRequest::at(source, r.width, r.height, window.scale_factor(), ImageFit::Cover);
        Picture::want(&slot, Some(request), window, cx);
        let crop = Some(r.width / r.height.max(1.));
        if let Some(image) = Picture::current(&slot, true, crop, window, cx) {
            children.push(
                div()
                    .absolute()
                    .left(px(r.x))
                    .top(px(r.y))
                    .w(px(r.width))
                    .h(px(r.height))
                    .child(picture(image, ImageFit::Cover, &StyleRefinement::default()))
                    .into_any_element(),
            );
        }
    }
    let ops = p.ops.clone();
    let painted = (p.width, p.height);
    let emit = b.backend.state.borrow().events.emitter(id);
    let wake = b.wake.clone();
    let shapes = gpui::canvas(
        move |bounds, _, cx| {
            let laid = (bounds.size.width.as_f32(), bounds.size.height.as_f32());
            if laid != painted {
                emit.emit(CanvasResized { width: laid.0, height: laid.1 });
                wake(cx);
            }
        },
        move |bounds, (), window, _| paint_ops(&ops, bounds.origin, window),
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full();
    let mut d = div().relative();
    d.style().refine(&p.style);
    d.child(shapes).children(children).into_any_element()
}
