//! `equalizer` on AppKit: four bar layers whose heights Core Animation moves on the render
//! server, each on its own period from `EQUALIZER`, so a playing row costs no main thread work.
//! Paused, or under Reduce Motion, the bars hold `EQUALIZER.still`.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use nd_components::media::{EQUALIZER, EqualizerProps, equalizer_bars};
use nd_core::Size;
use nd_theme::Theme;
use nd_ui::prelude::Hsla;
use objc2::rc::Retained;
use objc2_app_kit::NSView;
use objc2_foundation::{NSNumber, NSPoint, NSRect, NSSize, NSString};
use objc2_quartz_core::{
    CABasicAnimation, CALayer, CAMediaTiming, CAMediaTimingFunction, CATransaction, kCAMediaTimingFunctionEaseInEaseOut,
};

use super::{Input, NdMediaView, cg, reduce_motion};
use crate::custom::{CustomCx, CustomNative, CustomPaint, CustomWidget};

const MOTION: &str = "nd-equalizer";

pub(crate) struct EqualizerWidget;

impl CustomWidget for EqualizerWidget {
    fn create(&self, cx: &CustomCx<'_>) -> Box<dyn CustomNative> {
        let view = NdMediaView::new(cx.mtm());
        let root = view.layer().expect("layer-backed");
        let bars = (0..EQUALIZER.bars)
            .map(|_| {
                let bar = CALayer::new();
                // Grows up from its foot: the view is flipped, so the foot is the bottom edge.
                bar.setAnchorPoint(NSPoint::new(0.5, 1.));
                root.addSublayer(&bar);
                bar
            })
            .collect();
        let inner = Rc::new(EqualizerInner {
            view,
            bars,
            props: RefCell::new(props(cx).clone()),
            theme: RefCell::new(cx.theme().clone()),
            shown: RefCell::new(None),
        });
        inner.view.set_input(Rc::downgrade(&inner) as Weak<dyn Input>);
        inner.paint();
        Box::new(EqualizerNative { inner })
    }
}

fn props<'a>(cx: &'a CustomCx<'_>) -> &'a EqualizerProps {
    cx.props().downcast_ref::<EqualizerProps>().expect("equalizer node needs EqualizerProps")
}

struct EqualizerInner {
    view: Retained<NdMediaView>,
    bars: Vec<Retained<CALayer>>,
    props: RefCell<EqualizerProps>,
    theme: RefCell<Theme>,
    /// The side, colour and motion the bars were last set up for.
    shown: RefCell<Option<(f64, Hsla, bool)>>,
}

impl EqualizerInner {
    fn color(&self) -> Hsla {
        self.props.borrow().style.text.color.unwrap_or(self.theme.borrow().accent)
    }

    fn paint(&self) {
        let side = self.view.bounds().size.width.min(self.view.bounds().size.height);
        let color = self.color();
        let moving = self.props.borrow().playing && !reduce_motion();
        if side <= 0. || *self.shown.borrow() == Some((side, color, moving)) {
            return;
        }
        *self.shown.borrow_mut() = Some((side, color, moving));
        let frames = equalizer_bars(side as f32, |i| EQUALIZER.still[i]);
        CATransaction::begin();
        CATransaction::setDisableActions(true);
        for (i, (bar, (x, _, w, h))) in self.bars.iter().zip(frames).enumerate() {
            let (x, w, h) = (f64::from(x), f64::from(w), f64::from(h));
            bar.setBounds(NSRect::new(NSPoint::ZERO, NSSize::new(w, h)));
            bar.setPosition(NSPoint::new(x + w / 2., side));
            bar.setCornerRadius(w / 2.);
            bar.setBackgroundColor(Some(&cg(color)));
            bar.removeAnimationForKey(&NSString::from_str(MOTION));
            if moving {
                bar.addAnimation_forKey(&motion(i, side, w), Some(&NSString::from_str(MOTION)));
            }
        }
        CATransaction::commit();
    }
}

/// Bar `i` rising from `EQUALIZER.low` to `.high` of `side` and back, forever: an ease in and
/// out each way is the shared cosine wave, started at the bar's phase.
fn motion(i: usize, side: f64, width: f64) -> Retained<CABasicAnimation> {
    let m = EQUALIZER;
    let period = f64::from(m.periods[i]);
    let anim = CABasicAnimation::animationWithKeyPath(Some(&NSString::from_str("bounds.size.height")));
    let (from, to) = ((side * f64::from(m.low)).max(width), side * f64::from(m.high));
    unsafe {
        anim.setFromValue(Some(&NSNumber::new_f64(from)));
        anim.setToValue(Some(&NSNumber::new_f64(to)));
        anim.setTimingFunction(Some(&CAMediaTimingFunction::functionWithName(kCAMediaTimingFunctionEaseInEaseOut)));
    }
    anim.setDuration(period / 2.);
    anim.setAutoreverses(true);
    anim.setRepeatCount(f32::INFINITY);
    anim.setTimeOffset(f64::from(m.phases[i]) * period);
    anim
}

impl Input for EqualizerInner {
    fn layout(&self) {
        self.paint();
    }
}

struct EqualizerNative {
    inner: Rc<EqualizerInner>,
}

impl CustomNative for EqualizerNative {
    fn view(&self) -> &NSView {
        &self.inner.view
    }

    fn update(&mut self, cx: &CustomCx<'_>) {
        *self.inner.props.borrow_mut() = props(cx).clone();
        self.inner.paint();
    }

    fn set_theme(&mut self, theme: &Theme) {
        *self.inner.theme.borrow_mut() = theme.clone();
        self.inner.paint();
    }

    fn measure(&self, _width: Option<f32>) -> Size {
        let side = self.inner.props.borrow().size;
        Size { width: side, height: side }
    }

    fn paints(&self) -> Vec<CustomPaint> {
        vec![CustomPaint { label: "equalizer".into(), color: self.inner.color(), under: Vec::new(), text: None }]
    }
}
