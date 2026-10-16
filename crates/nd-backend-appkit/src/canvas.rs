//! `canvas` on AppKit: the display list `nd-ui` recorded, as one `CAShapeLayer` per shape and
//! an image layer per picture, in the order recorded. `nd-ui` lays AppKit out itself, so a
//! resize repaints there and arrives here as new props.

use std::cell::RefCell;
use std::f32::consts::TAU;

use nd_components::media::ImageSource;
use nd_theme::Theme;
use nd_ui::prelude::Hsla;
use nd_ui::{CanvasPath, CanvasProps, CanvasRect, PaintOp, PathStep};
use objc2::rc::Retained;
use objc2_app_kit::NSView;
use objc2_core_foundation::CFRetained;
use objc2_core_graphics::CGMutablePath;
use objc2_foundation::{NSPoint, NSRect, NSSize};
use objc2_quartz_core::{CALayer, CAShapeLayer, CATransaction};

use crate::custom::{CustomCx, CustomNative, CustomPaint};
use crate::media::image::ImageSlot;
use crate::media::{NdMediaView, cg, scale_of};

pub(crate) fn create(cx: &CustomCx<'_>) -> Box<dyn CustomNative> {
    let view = NdMediaView::new(cx.mtm());
    let mut native = CanvasNative { view, layers: RefCell::new(Vec::new()), slots: Vec::new(), ops: Vec::new() };
    native.update(cx);
    Box::new(native)
}

struct CanvasNative {
    view: Retained<NdMediaView>,
    layers: RefCell<Vec<Retained<CALayer>>>,
    slots: Vec<ImageSlot>,
    ops: Vec<PaintOp>,
}

fn rect(r: &CanvasRect) -> NSRect {
    NSRect::new(NSPoint::new(r.x.into(), r.y.into()), NSSize::new(r.width.into(), r.height.into()))
}

fn rounded_rect(r: &CanvasRect, radius: f32) -> CFRetained<CGMutablePath> {
    let path = CGMutablePath::new();
    let radius = f64::from(radius).min(f64::from(r.width.min(r.height)) / 2.).max(0.);
    unsafe { CGMutablePath::add_rounded_rect(Some(&path), std::ptr::null(), rect(r), radius, radius) };
    path
}

fn cg_path(steps: &CanvasPath) -> CFRetained<CGMutablePath> {
    let path = CGMutablePath::new();
    let m = std::ptr::null();
    let p = Some(&*path);
    for step in &steps.0 {
        // SAFETY: `m` is null, which every CGPath call takes as no transform.
        unsafe {
            match *step {
                PathStep::MoveTo(x, y) => CGMutablePath::move_to_point(p, m, x.into(), y.into()),
                PathStep::LineTo(x, y) => CGMutablePath::add_line_to_point(p, m, x.into(), y.into()),
                PathStep::QuadTo { ctrl, to } => {
                    CGMutablePath::add_quad_curve_to_point(p, m, ctrl.0.into(), ctrl.1.into(), to.0.into(), to.1.into())
                }
                PathStep::CubicTo { ctrl1, ctrl2, to } => CGMutablePath::add_curve_to_point(
                    p,
                    m,
                    ctrl1.0.into(),
                    ctrl1.1.into(),
                    ctrl2.0.into(),
                    ctrl2.1.into(),
                    to.0.into(),
                    to.1.into(),
                ),
                PathStep::Close => CGMutablePath::close_subpath(p),
            }
        }
    }
    path
}

/// The arc as a polyline, matching the GPUI backend: from `start` to `end`, fractions of a turn
/// clockwise from 12 o'clock in the canvas's top-left coordinates.
fn arc(center: (f32, f32), radius: f32, start: f32, end: f32) -> CanvasPath {
    let steps = (((end - start).abs() * 96.).ceil() as usize).max(2);
    let mut out = CanvasPath(Vec::with_capacity(steps + 1));
    for i in 0..=steps {
        let a = (start + (end - start) * i as f32 / steps as f32) * TAU;
        let p = (center.0 + radius * a.sin(), center.1 - radius * a.cos());
        out.0.push(if i == 0 { PathStep::MoveTo(p.0, p.1) } else { PathStep::LineTo(p.0, p.1) });
    }
    out
}

fn shape(path: &CGMutablePath, fill: Option<Hsla>, stroke: Option<(Hsla, f32)>) -> Retained<CALayer> {
    let layer = CAShapeLayer::new();
    layer.setPath(Some(path));
    layer.setFillColor(fill.map(cg).as_deref());
    if let Some((color, width)) = stroke {
        layer.setStrokeColor(Some(&cg(color)));
        layer.setLineWidth(width.into());
    }
    Retained::into_super(layer)
}

/// The colour each fill op paints, for the appearance audit.
pub(crate) fn fills(ops: &[PaintOp]) -> Vec<Hsla> {
    ops.iter()
        .filter_map(|op| match op {
            PaintOp::FillRect { color, .. } | PaintOp::FillCircle { color, .. } | PaintOp::FillPath { color, .. } => {
                Some(*color)
            }
            _ => None,
        })
        .collect()
}

impl CanvasNative {
    fn rebuild(&mut self) {
        let root = self.view.layer().expect("layer-backed");
        let scale = scale_of(&self.view);
        CATransaction::begin();
        CATransaction::setDisableActions(true);
        for layer in self.layers.borrow_mut().drain(..) {
            layer.removeFromSuperlayer();
        }
        let mut slots = std::mem::take(&mut self.slots).into_iter();
        let mut layers = Vec::with_capacity(self.ops.len());
        let mut kept = Vec::new();
        for op in &self.ops {
            let layer = match op {
                PaintOp::FillRect { rect: r, color, radius } => shape(&rounded_rect(r, *radius), Some(*color), None),
                PaintOp::StrokeRect { rect: r, color, width, radius } => {
                    shape(&rounded_rect(r, *radius), None, Some((*color, *width)))
                }
                PaintOp::FillCircle { center, radius, color } => {
                    let r = CanvasRect {
                        x: center.0 - radius,
                        y: center.1 - radius,
                        width: 2. * radius,
                        height: 2. * radius,
                    };
                    shape(&rounded_rect(&r, *radius), Some(*color), None)
                }
                PaintOp::StrokeArc { center, radius, start, end, width, color } => {
                    shape(&cg_path(&arc(*center, *radius, *start, *end)), None, Some((*color, *width)))
                }
                PaintOp::Line { from, to, width, color } => {
                    let line = CanvasPath(vec![PathStep::MoveTo(from.0, from.1), PathStep::LineTo(to.0, to.1)]);
                    shape(&cg_path(&line), None, Some((*color, *width)))
                }
                PaintOp::FillPath { path, color } => shape(&cg_path(path), Some(*color), None),
                PaintOp::StrokePath { path, width, color } => shape(&cg_path(path), None, Some((*color, *width))),
                PaintOp::Image { source, rect: r } => {
                    let Some(source) = source.get::<ImageSource>() else { continue };
                    let slot = slots.next().unwrap_or_else(ImageSlot::new);
                    slot.set_source(Some(source.clone()), nd_components::media::ImageFit::Cover, true);
                    slot.place(rect(r), scale);
                    let layer = slot.0.layer.clone();
                    kept.push(slot);
                    layer
                }
            };
            root.addSublayer(&layer);
            layers.push(layer);
        }
        CATransaction::commit();
        self.slots = kept;
        *self.layers.borrow_mut() = layers;
    }
}

impl CustomNative for CanvasNative {
    fn view(&self) -> &NSView {
        &self.view
    }

    fn update(&mut self, cx: &CustomCx<'_>) {
        let props = cx.props().downcast_ref::<CanvasProps>().expect("canvas node needs CanvasProps");
        if *props.ops == *self.ops {
            return;
        }
        self.ops = props.ops.to_vec();
        self.rebuild();
    }

    fn set_theme(&mut self, _theme: &Theme) {}

    fn paints(&self) -> Vec<CustomPaint> {
        fills(&self.ops)
            .into_iter()
            .map(|color| CustomPaint { label: "canvas".into(), color, under: Vec::new(), text: None })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arcs_start_at_twelve_and_run_clockwise() {
        let quarter = arc((10., 10.), 5., 0., 0.25);
        let PathStep::MoveTo(x, y) = quarter.0[0] else { panic!("starts with a move") };
        assert!((x - 10.).abs() < 1e-4 && (y - 5.).abs() < 1e-4, "{x},{y}");
        let PathStep::LineTo(x, y) = *quarter.0.last().expect("steps") else { panic!("ends with a line") };
        assert!((x - 15.).abs() < 1e-4 && (y - 10.).abs() < 1e-4, "{x},{y}");
    }
}
