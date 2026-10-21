//! `canvas`: custom drawing as a display list. The app's paint closure runs in `nd-ui` with the
//! canvas's laid-out bounds (origin at the canvas's top left) and records [`PaintOp`]s through
//! a [`Painter`]; the list reaches the backend as props, so an unchanged drawing costs no
//! backend work, and AppKit draws it with CoreGraphics while GPUI paints it as quads and paths.
//! When the canvas's size changes the closure runs again for the new bounds.

use std::panic::Location;
use std::rc::Rc;

use nd_gpui::gpui::{Bounds, Hsla, Pixels, Point, StyleRefinement, Styled, point, px, size};

use crate::app::App;
use crate::backend::{AnyProps, NodeKind};
use crate::element::{AnyElement, DispatchProps, IntoElement, Key, NodeElement};

/// One step of a path, in the canvas's coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PathStep {
    MoveTo(f32, f32),
    LineTo(f32, f32),
    /// A quadratic curve through the control point to the end point.
    QuadTo {
        ctrl: (f32, f32),
        to: (f32, f32),
    },
    CubicTo {
        ctrl1: (f32, f32),
        ctrl2: (f32, f32),
        to: (f32, f32),
    },
    Close,
}

/// An outline built step by step, filled or stroked by the painter.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CanvasPath(pub Vec<PathStep>);

impl CanvasPath {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn move_to(mut self, p: Point<Pixels>) -> Self {
        self.0.push(PathStep::MoveTo(p.x.as_f32(), p.y.as_f32()));
        self
    }

    pub fn line_to(mut self, p: Point<Pixels>) -> Self {
        self.0.push(PathStep::LineTo(p.x.as_f32(), p.y.as_f32()));
        self
    }

    pub fn quad_to(mut self, ctrl: Point<Pixels>, to: Point<Pixels>) -> Self {
        self.0.push(PathStep::QuadTo { ctrl: (ctrl.x.as_f32(), ctrl.y.as_f32()), to: (to.x.as_f32(), to.y.as_f32()) });
        self
    }

    pub fn cubic_to(mut self, ctrl1: Point<Pixels>, ctrl2: Point<Pixels>, to: Point<Pixels>) -> Self {
        let p = |p: Point<Pixels>| (p.x.as_f32(), p.y.as_f32());
        self.0.push(PathStep::CubicTo { ctrl1: p(ctrl1), ctrl2: p(ctrl2), to: p(to) });
        self
    }

    pub fn close(mut self) -> Self {
        self.0.push(PathStep::Close);
        self
    }
}

/// A rectangle in the canvas's coordinates.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CanvasRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl From<Bounds<Pixels>> for CanvasRect {
    fn from(b: Bounds<Pixels>) -> Self {
        Self {
            x: b.origin.x.as_f32(),
            y: b.origin.y.as_f32(),
            width: b.size.width.as_f32(),
            height: b.size.height.as_f32(),
        }
    }
}

/// What a canvas draws, in order.
#[derive(Clone, Debug, PartialEq)]
pub enum PaintOp {
    FillRect {
        rect: CanvasRect,
        color: Hsla,
        radius: f32,
    },
    StrokeRect {
        rect: CanvasRect,
        color: Hsla,
        width: f32,
        radius: f32,
    },
    FillCircle {
        center: (f32, f32),
        radius: f32,
        color: Hsla,
    },
    /// From `start` to `end`, fractions of a turn clockwise from 12 o'clock.
    StrokeArc {
        center: (f32, f32),
        radius: f32,
        start: f32,
        end: f32,
        width: f32,
        color: Hsla,
    },
    Line {
        from: (f32, f32),
        to: (f32, f32),
        width: f32,
        color: Hsla,
    },
    FillPath {
        path: CanvasPath,
        color: Hsla,
    },
    StrokePath {
        path: CanvasPath,
        width: f32,
        color: Hsla,
    },
    /// An image (the backend's image source type, `nd::ImageSource`) drawn into `rect`.
    Image {
        source: crate::Scope,
        rect: CanvasRect,
    },
}

/// Records what a canvas draws. Colours come from the theme like any other element's.
#[derive(Debug, Default)]
pub struct Painter {
    ops: Vec<PaintOp>,
}

fn pt(p: Point<Pixels>) -> (f32, f32) {
    (p.x.as_f32(), p.y.as_f32())
}

impl Painter {
    pub fn fill_rect(&mut self, rect: Bounds<Pixels>, color: Hsla) {
        self.ops.push(PaintOp::FillRect { rect: rect.into(), color, radius: 0. });
    }

    pub fn fill_rounded_rect(&mut self, rect: Bounds<Pixels>, radius: Pixels, color: Hsla) {
        self.ops.push(PaintOp::FillRect { rect: rect.into(), color, radius: radius.as_f32() });
    }

    pub fn stroke_rect(&mut self, rect: Bounds<Pixels>, radius: Pixels, width: Pixels, color: Hsla) {
        self.ops.push(PaintOp::StrokeRect { rect: rect.into(), color, width: width.as_f32(), radius: radius.as_f32() });
    }

    pub fn fill_circle(&mut self, center: Point<Pixels>, radius: Pixels, color: Hsla) {
        self.ops.push(PaintOp::FillCircle { center: pt(center), radius: radius.as_f32(), color });
    }

    /// A stroked arc around `center`, from `start` to `end` as fractions of a turn from 12 o'clock.
    pub fn stroke_arc(
        &mut self,
        center: Point<Pixels>,
        radius: Pixels,
        start: f32,
        end: f32,
        width: Pixels,
        color: Hsla,
    ) {
        self.ops.push(PaintOp::StrokeArc {
            center: pt(center),
            radius: radius.as_f32(),
            start,
            end,
            width: width.as_f32(),
            color,
        });
    }

    pub fn line(&mut self, from: Point<Pixels>, to: Point<Pixels>, width: Pixels, color: Hsla) {
        self.ops.push(PaintOp::Line { from: pt(from), to: pt(to), width: width.as_f32(), color });
    }

    pub fn fill_path(&mut self, path: CanvasPath, color: Hsla) {
        self.ops.push(PaintOp::FillPath { path, color });
    }

    pub fn stroke_path(&mut self, path: CanvasPath, width: Pixels, color: Hsla) {
        self.ops.push(PaintOp::StrokePath { path, width: width.as_f32(), color });
    }

    /// Draws `source` (an `nd::ImageSource`) into `rect`, cropped to fill it.
    pub fn image(&mut self, source: impl AnyProps, rect: Bounds<Pixels>) {
        self.ops.push(PaintOp::Image { source: crate::Scope::new(source), rect: rect.into() });
    }

    pub fn ops(&self) -> &[PaintOp] {
        &self.ops
    }
}

/// Props of a canvas node: the display list, the size it was painted for, and the element's
/// style (backends that lay out themselves size the canvas from it).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CanvasProps {
    pub ops: Rc<[PaintOp]>,
    pub width: f32,
    pub height: f32,
    pub style: StyleRefinement,
}

/// A backend laid the canvas out at a size it was not painted for; `nd-ui` paints it again.
/// Retained backends never send it: `nd-ui` sees their frames itself.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanvasResized {
    pub width: f32,
    pub height: f32,
}

pub(crate) type PaintFn = Rc<dyn Fn(Bounds<Pixels>, &mut Painter, &mut App)>;

/// Runs `paint` for a canvas `width` x `height` styled `style`.
pub(crate) fn record(paint: &PaintFn, style: StyleRefinement, width: f32, height: f32, app: &mut App) -> CanvasProps {
    let mut painter = Painter::default();
    if width > 0. && height > 0. {
        paint(Bounds::new(point(px(0.), px(0.)), size(px(width), px(height))), &mut painter, app);
    }
    CanvasProps { ops: painter.ops.into(), width, height, style }
}

/// Custom drawing. `paint` gets the canvas's bounds (its own top left at zero) and runs again
/// whenever the canvas renders or its size changes.
pub struct Canvas {
    key: Key,
    style: StyleRefinement,
    paint: PaintFn,
}

#[track_caller]
pub fn canvas(paint: impl Fn(Bounds<Pixels>, &mut Painter, &mut App) + 'static) -> Canvas {
    Canvas { key: Key::Site(Location::caller()), style: StyleRefinement::default(), paint: Rc::new(paint) }
}

impl Canvas {
    pub fn id(mut self, id: impl Into<nd_gpui::gpui::SharedString>) -> Self {
        self.key = Key::Id(id.into());
        self
    }
}

impl Styled for Canvas {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl nd_gpui::gpui::prelude::FluentBuilder for Canvas {}

impl IntoElement for Canvas {
    fn into_any_element(self) -> AnyElement {
        AnyElement::node(NodeElement {
            kind: NodeKind::Canvas,
            key: self.key,
            props: Rc::new(CanvasProps { style: self.style.clone(), ..CanvasProps::default() }),
            layout: self.style,
            measured: false,
            children: Vec::new(),
            handlers: Vec::new(),
            dispatch: DispatchProps::default(),
            motion: None,
            paint: Some(self.paint),
            drag: crate::drag::DragSpec::default(),
        })
    }
}
