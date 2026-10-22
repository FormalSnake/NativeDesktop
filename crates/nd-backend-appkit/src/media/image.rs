//! `image()` on AppKit: the decoded frames as a layer's contents (ARCHITECTURE.md 3.7), with
//! the placeholder fill until the first arrive. [`ImageSlot`] is the loader the avatar reuses.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::sync::Arc;

use dispatch2::DispatchQueue;
use nd_components::media::{
    DecodeRequest, Decoded, ImageFailed, ImageFit, ImageLoaded, ImagePipeline, ImageProps, ImageSource,
};
use nd_core::Size;
use nd_core::time::Instant;
use nd_theme::Theme;
use nd_ui::events::Emit;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::NSView;
use objc2_core_foundation::CFRetained;
use objc2_core_graphics::{
    CGBitmapContextCreate, CGBitmapContextCreateImage, CGBitmapContextGetBytesPerRow, CGBitmapContextGetData,
    CGColorSpace, CGImage, CGImageAlphaInfo,
};
use objc2_quartz_core::{CALayer, kCAGravityResizeAspect, kCAGravityResizeAspectFill};

use super::{Input, NdMediaView, Ticker, cg, reduce_motion, scale_of, style_radius};
use crate::custom::{CustomCx, CustomNative, CustomPaint, CustomWidget, Waker};

/// Straight RGBA to a premultiplied sRGB `CGImage`.
pub(crate) fn cg_image(width: u32, height: u32, rgba: &[u8]) -> Option<CFRetained<CGImage>> {
    let (w, h) = (width as usize, height as usize);
    if w == 0 || h == 0 || rgba.len() < w * h * 4 {
        return None;
    }
    let space = CGColorSpace::with_name(Some(unsafe { objc2_core_graphics::kCGColorSpaceSRGB }))
        .or_else(CGColorSpace::new_device_rgb)?;
    let info = CGImageAlphaInfo::PremultipliedLast.0;
    let ctx = unsafe { CGBitmapContextCreate(std::ptr::null_mut(), w, h, 8, 0, Some(&space), info) }?;
    let data = CGBitmapContextGetData(Some(&ctx)).cast::<u8>();
    let stride = CGBitmapContextGetBytesPerRow(Some(&ctx));
    if data.is_null() {
        return None;
    }
    for y in 0..h {
        // SAFETY: the context owns `stride * h` bytes and nothing else touches them yet.
        let row = unsafe { std::slice::from_raw_parts_mut(data.add(y * stride), w * 4) };
        let src = &rgba[y * w * 4..(y + 1) * w * 4];
        for (d, s) in row.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
            let a = u16::from(s[3]);
            d[0] = ((u16::from(s[0]) * a + 127) / 255) as u8;
            d[1] = ((u16::from(s[1]) * a + 127) / 255) as u8;
            d[2] = ((u16::from(s[2]) * a + 127) / 255) as u8;
            d[3] = s[3];
        }
    }
    CGBitmapContextCreateImage(Some(&ctx))
}

pub(crate) fn set_contents(layer: &CALayer, image: Option<&CGImage>) {
    let object = image.map(|i| {
        // SAFETY: a CGImage is a CF type, which is toll-free an Objective-C object.
        unsafe { &*(i as *const CGImage).cast::<AnyObject>() }
    });
    unsafe { layer.setContents(object) };
}

thread_local! {
    static SLOTS: RefCell<HashMap<u64, Weak<SlotState>>> = RefCell::default();
    static NEXT: Cell<u64> = const { Cell::new(1) };
}

/// What happened to the slot's load, for the widget to report.
pub(crate) enum Loaded {
    Ready(Arc<Decoded>),
    Failed(String),
}

type OnLoad = Box<dyn Fn(&Loaded)>;

/// Loads one source into one layer at the layer's size, through the shared pipeline.
pub(crate) struct SlotState {
    id: u64,
    pub layer: Retained<CALayer>,
    source: RefCell<Option<ImageSource>>,
    fit: Cell<ImageFit>,
    tint: Cell<Option<nd_ui::prelude::Hsla>>,
    /// The blur's sigma in decoded pixels.
    blur: Cell<f32>,
    animate: Cell<bool>,
    request: RefCell<Option<DecodeRequest>>,
    decoded: RefCell<Option<Arc<Decoded>>>,
    frames: RefCell<Vec<Option<CFRetained<CGImage>>>>,
    shown: Cell<Option<usize>>,
    started: Cell<Instant>,
    ticker: Rc<Ticker>,
    on_load: RefCell<Option<OnLoad>>,
    placeholder: Cell<Option<nd_ui::prelude::Hsla>>,
}

#[derive(Clone)]
pub(crate) struct ImageSlot(pub Rc<SlotState>);

impl ImageSlot {
    pub fn new() -> Self {
        let id = NEXT.with(|n| n.replace(n.get() + 1));
        let layer = CALayer::new();
        layer.setMasksToBounds(true);
        let state = Rc::new(SlotState {
            id,
            layer,
            source: RefCell::new(None),
            fit: Cell::new(ImageFit::Cover),
            tint: Cell::new(None),
            blur: Cell::new(0.),
            animate: Cell::new(true),
            request: RefCell::new(None),
            decoded: RefCell::new(None),
            frames: RefCell::new(Vec::new()),
            shown: Cell::new(None),
            started: Cell::new(Instant::now()),
            ticker: Rc::default(),
            on_load: RefCell::new(None),
            placeholder: Cell::new(None),
        });
        SLOTS.with(|s| s.borrow_mut().insert(id, Rc::downgrade(&state)));
        Self(state)
    }

    /// Draws the picture in `tint`, keeping its alpha; takes effect with the next request.
    pub fn set_tint(&self, tint: Option<nd_ui::prelude::Hsla>) {
        if self.0.tint.replace(tint) != tint {
            *self.0.request.borrow_mut() = None;
        }
    }

    /// Blurs the picture by `sigma` decoded pixels; takes effect with the next request.
    pub fn set_blur(&self, sigma: f32) {
        if self.0.blur.replace(sigma) != sigma {
            *self.0.request.borrow_mut() = None;
        }
    }

    pub fn on_load(&self, f: impl Fn(&Loaded) + 'static) {
        *self.0.on_load.borrow_mut() = Some(Box::new(f));
    }

    pub fn set_placeholder(&self, color: nd_ui::prelude::Hsla) {
        self.0.placeholder.set(Some(color));
        if self.0.shown.get().is_none() {
            self.0.layer.setBackgroundColor(Some(&cg(color)));
        }
    }

    pub fn set_source(&self, source: Option<ImageSource>, fit: ImageFit, animate: bool) {
        let s = &self.0;
        s.fit.set(fit);
        unsafe {
            s.layer.setContentsGravity(match fit {
                ImageFit::Cover => kCAGravityResizeAspectFill,
                ImageFit::Contain => kCAGravityResizeAspect,
            })
        };
        s.animate.set(animate);
        if *s.source.borrow() != source {
            // A new source keeps the old picture up until its own decode lands, so a stream of
            // frames (motion artwork) never flashes the placeholder.
            let keep = source.is_some() && s.shown.get().is_some();
            *s.source.borrow_mut() = source;
            *s.request.borrow_mut() = None;
            if keep {
                s.replace();
            } else {
                s.clear();
            }
        }
    }

    /// A picture is on screen, not the placeholder.
    pub fn is_showing(&self) -> bool {
        self.0.shown.get().is_some()
    }

    /// Lays the layer out at `frame` (in its superlayer) and loads at that size.
    pub fn place(&self, frame: objc2_foundation::NSRect, scale: f64) {
        let s = &self.0;
        if s.layer.frame() != frame {
            s.layer.setFrame(frame);
        }
        s.layer.setContentsScale(scale);
        let Some(source) = s.source.borrow().clone() else { return };
        let (w, h) = (frame.size.width as f32, frame.size.height as f32);
        if w <= 0. || h <= 0. {
            return;
        }
        let request =
            DecodeRequest::at(source, w, h, scale as f32, s.fit.get()).tinted(s.tint.get()).blurred(s.blur.get());
        if s.request.borrow().as_ref() == Some(&request) {
            return;
        }
        *s.request.borrow_mut() = Some(request.clone());
        let pipeline = ImagePipeline::global();
        if let Some(decoded) = pipeline.get(&request) {
            s.show(&request, Ok(decoded));
            return;
        }
        let id = s.id;
        let req = request.clone();
        pipeline.load(request, move |result| {
            DispatchQueue::main().exec_async(move || {
                let Some(slot) = SLOTS.with(|s| s.borrow().get(&id).and_then(Weak::upgrade)) else { return };
                slot.show(&req, result);
            });
        });
    }
}

impl SlotState {
    fn clear(&self) {
        *self.decoded.borrow_mut() = None;
        self.frames.borrow_mut().clear();
        self.shown.set(None);
        self.ticker.stop();
        set_contents(&self.layer, None);
        if let Some(color) = self.placeholder.get() {
            self.layer.setBackgroundColor(Some(&cg(color)));
        }
    }

    /// Drops the old source's frames and clock but leaves its picture on the layer.
    fn replace(&self) {
        *self.decoded.borrow_mut() = None;
        self.frames.borrow_mut().clear();
        self.shown.set(None);
        self.ticker.stop();
    }

    fn show(self: &Rc<Self>, request: &DecodeRequest, result: Result<Arc<Decoded>, String>) {
        // A result for a size or source the slot moved on from.
        if self.request.borrow().as_ref() != Some(request) {
            return;
        }
        let loaded = match result {
            Ok(decoded) => {
                let same = self.decoded.borrow().as_ref().is_some_and(|d| Arc::ptr_eq(d, &decoded));
                if !same {
                    *self.frames.borrow_mut() = vec![None; decoded.frames.len()];
                    *self.decoded.borrow_mut() = Some(decoded.clone());
                    self.shown.set(None);
                    self.started.set(Instant::now());
                }
                self.show_frame(0);
                self.layer.setBackgroundColor(None);
                self.animate_if_needed();
                Loaded::Ready(decoded)
            }
            Err(e) => Loaded::Failed(e),
        };
        if let Some(on_load) = self.on_load.borrow().as_ref() {
            on_load(&loaded);
        }
    }

    fn show_frame(&self, index: usize) {
        if self.shown.get() == Some(index) {
            return;
        }
        let Some(decoded) = self.decoded.borrow().clone() else { return };
        let Some(frame) = decoded.frames.get(index) else { return };
        let mut frames = self.frames.borrow_mut();
        if frames[index].is_none() {
            frames[index] = cg_image(decoded.width, decoded.height, &frame.rgba);
        }
        objc2_quartz_core::CATransaction::begin();
        objc2_quartz_core::CATransaction::setDisableActions(true);
        set_contents(&self.layer, frames[index].as_deref());
        objc2_quartz_core::CATransaction::commit();
        self.shown.set(Some(index));
    }

    fn animate_if_needed(self: &Rc<Self>) {
        let animated = self.decoded.borrow().as_ref().is_some_and(|d| d.is_animated());
        if !animated || !self.animate.get() || reduce_motion() {
            self.ticker.stop();
            return;
        }
        let me = Rc::downgrade(self);
        self.ticker.start(60., move || {
            let Some(me) = me.upgrade() else { return false };
            let Some(decoded) = me.decoded.borrow().clone() else { return false };
            let index = decoded.frame_at(me.started.get().elapsed());
            me.show_frame(index);
            if let Some(request) = me.request.borrow().as_ref() {
                ImagePipeline::global().touch(request);
            }
            true
        });
    }
}

impl Drop for SlotState {
    fn drop(&mut self) {
        let id = self.id;
        let _ = SLOTS.try_with(|s| s.borrow_mut().remove(&id));
    }
}

pub(crate) struct ImageWidget;

impl CustomWidget for ImageWidget {
    fn create(&self, cx: &CustomCx<'_>) -> Box<dyn CustomNative> {
        let props = props(cx).clone();
        let view = NdMediaView::new(cx.mtm());
        let slot = ImageSlot::new();
        view.layer().expect("layer-backed").addSublayer(&slot.0.layer);
        let inner = Rc::new(ImageInner {
            view: view.clone(),
            slot: slot.clone(),
            props: RefCell::new(props.clone()),
            theme: RefCell::new(cx.theme().clone()),
            reported: Cell::new(false),
        });
        let emit = cx.emitter();
        let wake = cx.waker();
        let me = Rc::downgrade(&inner);
        slot.on_load(move |loaded| report(&me, &emit, &wake, loaded));
        view.set_input(Rc::downgrade(&inner) as Weak<dyn Input>);
        inner.apply();
        Box::new(ImageNative { inner })
    }
}

fn props<'a>(cx: &'a CustomCx<'_>) -> &'a ImageProps {
    cx.props().downcast_ref::<ImageProps>().expect("image node needs ImageProps")
}

/// One `ImageLoaded` or `ImageFailed` per source the node shows.
fn report(me: &Weak<ImageInner>, emit: &Emit, wake: &Waker, loaded: &Loaded) {
    let Some(me) = me.upgrade() else { return };
    if me.reported.replace(true) {
        return;
    }
    match loaded {
        Loaded::Ready(d) => {
            emit.emit(ImageLoaded { width: d.source_width, height: d.source_height, animated: d.is_animated() })
        }
        Loaded::Failed(error) => emit.emit(ImageFailed { error: error.clone() }),
    }
    wake.wake();
}

struct ImageInner {
    view: Retained<NdMediaView>,
    slot: ImageSlot,
    props: RefCell<ImageProps>,
    theme: RefCell<Theme>,
    reported: Cell<bool>,
}

impl ImageInner {
    fn apply(&self) {
        let props = self.props.borrow();
        self.slot.set_placeholder(self.theme.borrow().surface.raised);
        let before = self.slot.0.source.borrow().clone();
        self.slot.set_tint(props.tint);
        self.slot.set_source(props.source.clone(), props.fit, props.animate);
        if before != props.source {
            self.reported.set(false);
        }
        self.view.setToolTip(props.tooltip.as_deref().map(objc2_foundation::NSString::from_str).as_deref());
        drop(props);
        self.layout();
    }
}

impl Input for ImageInner {
    fn layout(&self) {
        let bounds = self.view.bounds();
        let tail = self.props.borrow().tail;
        let Some(side) = tail else {
            let radius = style_radius(&self.props.borrow().style, bounds.size.width, bounds.size.height);
            self.slot.0.layer.setCornerRadius(radius);
            // SAFETY: the layer is ours and on the main thread; clearing the mask is plain KVC.
            unsafe { self.slot.0.layer.setMask(None) };
            let screen = scale_of(&self.view) as f32;
            let props = self.props.borrow();
            self.slot.set_blur(props.blur_sigma(screen));
            let bleed = f64::from(props.bleed(screen));
            let frame = objc2_foundation::NSRect::new(
                objc2_foundation::NSPoint::new(bounds.origin.x - bleed, bounds.origin.y - bleed),
                objc2_foundation::NSSize::new(bounds.size.width + 2. * bleed, bounds.size.height + 2. * bleed),
            );
            if let Some(layer) = self.view.layer() {
                layer.setMasksToBounds(bleed > 0.);
            }
            self.slot.place(frame, props.decode_scale(screen).into());
            return;
        };
        // The picture fills the whole box, the room under it included; the mask cuts the bubble
        // and its tail out of it, so the tail shows the picture's own corner.
        let drop = f64::from(nd_components::media::BUBBLE.tail_drop);
        let body = objc2_foundation::NSRect::new(
            objc2_foundation::NSPoint::new(0., 0.),
            objc2_foundation::NSSize::new(bounds.size.width, (bounds.size.height - drop).max(0.)),
        );
        let radius = style_radius(&self.props.borrow().style, body.size.width, body.size.height);
        let path = objc2_core_graphics::CGMutablePath::new();
        super::chat::rounded(&path, body, [radius; 4]);
        super::chat::tail(&path, body, side);
        objc2_quartz_core::CATransaction::begin();
        objc2_quartz_core::CATransaction::setDisableActions(true);
        let mask = objc2_quartz_core::CAShapeLayer::new();
        mask.setFrame(objc2_foundation::NSRect::new(objc2_foundation::NSPoint::ZERO, bounds.size));
        mask.setPath(Some(&path));
        self.slot.0.layer.setCornerRadius(0.);
        // SAFETY: the mask is a fresh layer used by no other layer.
        unsafe { self.slot.0.layer.setMask(Some(&mask)) };
        objc2_quartz_core::CATransaction::commit();
        self.slot.place(bounds, self.props.borrow().decode_scale(scale_of(&self.view) as f32).into());
    }
}

struct ImageNative {
    inner: Rc<ImageInner>,
}

impl CustomNative for ImageNative {
    fn view(&self) -> &NSView {
        &self.inner.view
    }

    fn update(&mut self, cx: &CustomCx<'_>) {
        *self.inner.props.borrow_mut() = props(cx).clone();
        self.inner.apply();
    }

    fn set_theme(&mut self, theme: &Theme) {
        *self.inner.theme.borrow_mut() = theme.clone();
        self.inner.apply();
    }

    fn measure(&self, width: Option<f32>) -> Size {
        self.inner.props.borrow().natural_size(width, ImagePipeline::global())
    }

    fn paints(&self) -> Vec<CustomPaint> {
        if self.inner.slot.0.shown.get().is_some() {
            return Vec::new();
        }
        let fill = self.inner.theme.borrow().surface.raised;
        vec![CustomPaint { label: "image placeholder".into(), color: fill, under: Vec::new(), text: None }]
    }
}
