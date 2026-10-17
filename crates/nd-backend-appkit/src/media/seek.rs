//! `seek_bar` on AppKit: a layer-drawn track with the buffered and played ranges, a thumb shown
//! while hovered or dragged, and the elapsed and remaining times in monospaced digits. Input
//! goes through the shared `SeekModel`; while playing, a ticker moves it between the app's
//! position reports without a render.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use nd_components::media::{SEEK, SeekBarProps, SeekModel, Seeked};
use nd_core::Size;
use nd_core::time::Instant;
use nd_theme::Theme;
use nd_ui::events::Emit;
use nd_ui::semantics::{ActionError, SemanticAction};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::{NSEvent, NSFont, NSFontWeightRegular, NSView};
use objc2_core_foundation::CFType;
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use objc2_quartz_core::{CALayer, CATextLayer, CATransaction, kCAAlignmentLeft, kCAAlignmentRight};

use super::{Input, NdMediaView, Ticker, cg, scale_of};
use crate::custom::{CustomCx, CustomNative, CustomPaint, CustomWidget, Waker};

pub(crate) fn text_layer(right: bool) -> Retained<CATextLayer> {
    let layer = CATextLayer::new();
    unsafe { layer.setAlignmentMode(if right { kCAAlignmentRight } else { kCAAlignmentLeft }) };
    layer
}

pub(crate) fn set_font(layer: &CATextLayer, font: &NSFont) {
    // SAFETY: NSFont is toll-free bridged with CTFont, a CF type.
    let cf = unsafe { &*(font as *const NSFont).cast::<CFType>() };
    unsafe { layer.setFont(Some(cf)) };
    layer.setFontSize(font.pointSize());
}

pub(crate) fn set_string(layer: &CATextLayer, text: &str) {
    let s = NSString::from_str(text);
    let object: &AnyObject = &s;
    unsafe { layer.setString(Some(object)) };
}

pub(crate) struct SeekWidget;

impl CustomWidget for SeekWidget {
    fn create(&self, cx: &CustomCx<'_>) -> Box<dyn CustomNative> {
        let props = props(cx).clone();
        let view = NdMediaView::new(cx.mtm());
        let root = view.layer().expect("layer-backed");
        let make = || {
            let l = CALayer::new();
            root.addSublayer(&l);
            l
        };
        let (track, buffered, played, thumb) = (make(), make(), make(), make());
        let elapsed = text_layer(false);
        let remaining = text_layer(true);
        root.addSublayer(&elapsed);
        root.addSublayer(&remaining);
        let inner = Rc::new(SeekInner {
            view,
            track,
            buffered,
            played,
            thumb,
            elapsed,
            remaining,
            props: RefCell::new(props),
            theme: RefCell::new(cx.theme().clone()),
            model: RefCell::new(SeekModel::default()),
            ticker: Rc::default(),
            emit: cx.emitter(),
            wake: cx.waker(),
        });
        inner.view.set_input(Rc::downgrade(&inner) as Weak<dyn Input>);
        inner.restyle();
        inner.run();
        Box::new(SeekNative { inner })
    }
}

fn props<'a>(cx: &'a CustomCx<'_>) -> &'a SeekBarProps {
    cx.props().downcast_ref::<SeekBarProps>().expect("seek bar node needs SeekBarProps")
}

struct SeekInner {
    view: Retained<NdMediaView>,
    track: Retained<CALayer>,
    buffered: Retained<CALayer>,
    played: Retained<CALayer>,
    thumb: Retained<CALayer>,
    elapsed: Retained<CATextLayer>,
    remaining: Retained<CATextLayer>,
    props: RefCell<SeekBarProps>,
    theme: RefCell<Theme>,
    model: RefCell<SeekModel>,
    ticker: Rc<Ticker>,
    emit: Emit,
    wake: Waker,
}

fn times_font(theme: &Theme) -> Retained<NSFont> {
    NSFont::monospacedDigitSystemFontOfSize_weight(f64::from(theme.type_scale.caption.as_f32()), unsafe {
        NSFontWeightRegular
    })
}

impl SeekInner {
    fn send(&self, event: Option<Seeked>) {
        if let Some(event) = event {
            self.emit.emit(event);
            self.wake.wake();
        }
    }

    fn restyle(&self) {
        let theme = self.theme.borrow();
        self.track.setBackgroundColor(Some(&cg(theme.border.subtle)));
        self.buffered.setBackgroundColor(Some(&cg(theme.text.tertiary)));
        self.played.setBackgroundColor(Some(&cg(theme.accent)));
        self.thumb.setBackgroundColor(Some(&cg(theme.accent)));
        let font = times_font(&theme);
        for t in [&self.elapsed, &self.remaining] {
            set_font(t, &font);
            t.setForegroundColor(Some(&cg(theme.text.secondary)));
        }
        let tip = self.props.borrow().tooltip.clone();
        self.view.setToolTip(tip.as_deref().map(NSString::from_str).as_deref());
        drop(theme);
        self.paint();
    }

    /// Ticks while playing, so the bar moves between position reports.
    fn run(self: &Rc<Self>) {
        let playing = self.props.borrow().clock.playing;
        if !playing {
            self.ticker.stop();
            return;
        }
        let me = Rc::downgrade(self);
        self.ticker.start(30., move || {
            let Some(me) = me.upgrade() else { return false };
            if me.view.window().is_none() {
                return false;
            }
            me.paint();
            me.props.borrow().clock.playing
        });
    }

    fn track_rect(&self) -> NSRect {
        let width = self.view.bounds().size.width;
        NSRect::new(NSPoint::new(0., 0.), NSSize::new(width, f64::from(SEEK.hit)))
    }

    fn fraction_at(&self, at: NSPoint) -> f32 {
        let r = self.track_rect();
        if r.size.width <= 0. { 0. } else { ((at.x - r.origin.x) / r.size.width).clamp(0., 1.) as f32 }
    }

    fn paint(&self) {
        let props = self.props.borrow();
        let mut model = self.model.borrow_mut();
        let now = Instant::now();
        let shown = model.shown(&props, now);
        let played = f64::from(props.fraction(shown));
        let buffered = f64::from(props.buffered_fraction(props.fraction(shown)));
        let active = model.active() && props.seekable();
        let r = self.track_rect();
        let thick = f64::from(if active { SEEK.track_active } else { SEEK.track });
        let y = ((r.size.height - thick) / 2.).round();
        let w = r.size.width;
        CATransaction::begin();
        CATransaction::setDisableActions(true);
        for (layer, fraction) in [(&self.track, 1.), (&self.buffered, buffered), (&self.played, played)] {
            layer.setFrame(NSRect::new(NSPoint::new(0., y), NSSize::new((w * fraction).max(0.), thick)));
            layer.setCornerRadius(thick / 2.);
        }
        let t = f64::from(SEEK.thumb);
        self.thumb.setFrame(NSRect::new(
            NSPoint::new((w * played - t / 2.).clamp(0., (w - t).max(0.)), ((r.size.height - t) / 2.).round()),
            NSSize::new(t, t),
        ));
        self.thumb.setCornerRadius(t / 2.);
        self.thumb.setHidden(!active);
        let scale = scale_of(&self.view);
        let show_times = props.show_times;
        let (elapsed, remaining) = model.times(&props, now);
        for (layer, text) in [(&self.elapsed, elapsed), (&self.remaining, remaining)] {
            layer.setHidden(!show_times);
            layer.setContentsScale(scale);
            set_string(layer, &text);
            let line = self.view.bounds().size.height - r.size.height - f64::from(SEEK.label_gap);
            layer.setFrame(NSRect::new(
                NSPoint::new(0., r.size.height + f64::from(SEEK.label_gap)),
                NSSize::new(w, line.max(0.)),
            ));
        }
        CATransaction::commit();
    }
}

impl Input for SeekInner {
    fn takes_mouse(&self) -> bool {
        true
    }

    fn mouse_down(&self, at: NSPoint, _event: &NSEvent) {
        if let Some(w) = self.view.window() {
            w.makeFirstResponder(Some(&self.view));
        }
        let event = self.model.borrow_mut().press(&self.props.borrow(), self.fraction_at(at));
        self.send(event);
        self.paint();
    }

    fn mouse_dragged(&self, at: NSPoint, _event: &NSEvent) {
        let event = self.model.borrow_mut().moved(&self.props.borrow(), self.fraction_at(at));
        self.send(event);
        self.paint();
    }

    fn mouse_up(&self, at: NSPoint, _event: &NSEvent) {
        let props = self.props.borrow();
        let mut model = self.model.borrow_mut();
        let moved = model.moved(&props, self.fraction_at(at));
        let release = model.release(&props, Instant::now());
        drop((model, props));
        self.send(moved);
        self.send(release);
        self.paint();
    }

    fn hover(&self, inside: bool, _at: NSPoint) {
        if self.model.borrow_mut().set_hovered(inside) {
            self.paint();
        }
    }

    fn key_down(&self, event: &NSEvent) -> bool {
        let direction = match event.keyCode() {
            123 => -1,
            124 => 1,
            _ => return false,
        };
        let step = self.model.borrow_mut().step(&self.props.borrow(), direction, Instant::now());
        self.send(step);
        self.paint();
        true
    }

    fn layout(&self) {
        self.paint();
    }

    fn takes_focus(&self) -> bool {
        self.props.borrow().seekable()
    }
}

struct SeekNative {
    inner: Rc<SeekInner>,
}

impl CustomNative for SeekNative {
    fn view(&self) -> &NSView {
        &self.inner.view
    }

    fn update(&mut self, cx: &CustomCx<'_>) {
        *self.inner.props.borrow_mut() = props(cx).clone();
        self.inner.restyle();
        self.inner.run();
    }

    fn set_theme(&mut self, theme: &Theme) {
        *self.inner.theme.borrow_mut() = theme.clone();
        self.inner.restyle();
    }

    fn measure(&self, width: Option<f32>) -> Size {
        let mut height = SEEK.hit;
        if self.inner.props.borrow().show_times {
            let font = times_font(&self.inner.theme.borrow());
            height += SEEK.label_gap + (font.ascender() - font.descender() + font.leading()).ceil() as f32;
        }
        Size { width: width.unwrap_or(160.), height }
    }

    fn perform(&self, action: &SemanticAction) -> Option<Result<(), ActionError>> {
        use nd_ui::semantics::WidgetSemantics;
        let response = self.inner.props.borrow().respond(action)?;
        Some(response.map(|event| {
            self.inner.emit.emit_boxed(event);
            self.inner.wake.wake();
        }))
    }

    fn paints(&self) -> Vec<CustomPaint> {
        let theme = self.inner.theme.borrow();
        let mut out = vec![
            CustomPaint { label: "seek track".into(), color: theme.border.subtle, under: Vec::new(), text: None },
            CustomPaint { label: "seek played".into(), color: theme.accent, under: Vec::new(), text: None },
        ];
        if self.inner.props.borrow().show_times {
            let size = theme.type_scale.caption.as_f32();
            out.push(CustomPaint {
                label: "seek times".into(),
                color: theme.text.secondary,
                under: Vec::new(),
                text: Some((size, 400.)),
            });
        }
        out
    }
}
