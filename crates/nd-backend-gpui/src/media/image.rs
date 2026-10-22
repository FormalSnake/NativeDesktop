//! `image`: a picture from the shared pipeline, decoded on its workers at the size the node was
//! last painted, shown as a GPUI image. Until the first decode lands the node is a placeholder of
//! its final size on the raised surface; a new source keeps the old picture up until its own
//! decode lands, so a stream of frames (motion artwork) never flashes the placeholder.

use std::cell::RefCell;
use std::rc::{Rc, Weak};
use std::sync::Arc;

use futures::channel::oneshot;
use nd_components::{
    DecodeRequest, Decoded, ImageFailed, ImageFit, ImageLoaded, ImagePipeline, ImageProps, ImageSource,
};
use nd_core::NodeId;
use nd_core::time::{Duration, Instant};
use nd_gpui::gpui::{
    self, AnyElement, AsyncApp, ElementId, InteractiveElement, IntoElement, ObjectFit, ParentElement, RenderImage,
    StatefulInteractiveElement, StyleRefinement, Styled, StyledImage, Task, div, img, px,
};

use super::fixed;
use crate::forms::{Sink, with_style};
use crate::nav::Marks;
use crate::render::Build;

/// One picture a node shows: the decode it asked for, the one on screen, and the GPU images made
/// from its frames.
#[derive(Default)]
pub(crate) struct Picture {
    source: Option<ImageSource>,
    wanted: Option<DecodeRequest>,
    shown: Option<Arc<Decoded>>,
    frames: Vec<Option<Arc<RenderImage>>>,
    /// The aspect `frames` were cropped to.
    crop: Option<f32>,
    loading: Option<Task<()>>,
    tick: Option<Task<()>>,
    started: Option<Instant>,
    /// The source changed since `shown` was decoded: the next decode restarts the clock.
    replaced: bool,
    /// The source whose outcome the app heard.
    reported: Option<ImageSource>,
    /// The node's events; `None` for pictures inside another widget (an avatar).
    sink: Option<Sink>,
    app: Option<AsyncApp>,
}

impl Drop for Picture {
    fn drop(&mut self) {
        let images: Vec<Arc<RenderImage>> = self.frames.drain(..).flatten().collect();
        if let (Some(app), false) = (self.app.take(), images.is_empty()) {
            app.spawn(async move |cx| {
                cx.update(|cx| {
                    for image in images {
                        cx.drop_image(image, None);
                    }
                })
            })
            .detach();
        }
    }
}

/// RGBA from the pipeline as the BGRA GPUI uploads.
/// A cover fit is cropped here to the box's aspect: GPUI rounds the corners of the image's own
/// bounds, which a cover fit puts outside the box.
fn render_image(decoded: &Decoded, frame: usize, crop: Option<f32>) -> Option<Arc<RenderImage>> {
    let (w, h) = (decoded.width, decoded.height);
    let (cw, ch) = match crop {
        Some(aspect) if aspect > 0. && (w as f32 / h.max(1) as f32 - aspect).abs() > 0.01 => {
            if w as f32 / h as f32 > aspect {
                (((h as f32 * aspect).round() as u32).clamp(1, w), h)
            } else {
                (w, ((w as f32 / aspect).round() as u32).clamp(1, h))
            }
        }
        _ => (w, h),
    };
    let (x0, y0) = ((w - cw) / 2, (h - ch) / 2);
    let rgba = &decoded.frames.get(frame)?.rgba;
    let mut bgra = Vec::with_capacity((cw * ch * 4) as usize);
    for y in y0..y0 + ch {
        let row = ((y * w + x0) * 4) as usize;
        bgra.extend_from_slice(rgba.get(row..row + (cw * 4) as usize)?);
    }
    for pixel in bgra.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    let buffer = ::image::RgbaImage::from_raw(cw, ch, bgra)?;
    Some(Arc::new(RenderImage::new(vec![::image::Frame::new(buffer)])))
}

impl Picture {
    fn forget_frames(&mut self, window: &mut gpui::Window, cx: &mut gpui::App) {
        for image in self.frames.drain(..).flatten() {
            cx.drop_image(image, Some(window));
        }
    }

    fn show(&mut self, decoded: Arc<Decoded>, window: &mut gpui::Window, cx: &mut gpui::App) {
        if self.shown.as_ref().is_some_and(|s| Arc::ptr_eq(s, &decoded)) {
            return;
        }
        self.forget_frames(window, cx);
        self.frames = vec![None; decoded.frames.len()];
        self.shown = Some(decoded);
        if std::mem::take(&mut self.replaced) {
            self.started = None;
        }
        self.started.get_or_insert_with(Instant::now);
    }

    /// Tells the app once per source how its load ended.
    fn report(&mut self, result: Result<&Decoded, &str>, cx: &mut gpui::App) {
        if self.reported == self.source {
            return;
        }
        self.reported = self.source.clone();
        let Some(sink) = &self.sink else { return };
        match result {
            Ok(d) => {
                sink.send(ImageLoaded { width: d.source_width, height: d.source_height, animated: d.is_animated() }, cx)
            }
            Err(e) => sink.send(ImageFailed { error: e.to_owned() }, cx),
        }
    }

    /// The decode for `request`: on screen now when the pipeline holds it, else asked of the
    /// workers, with the result hopping back to the main thread through a GPUI task.
    pub fn want(
        this: &Rc<RefCell<Picture>>,
        request: Option<DecodeRequest>,
        window: &mut gpui::Window,
        cx: &mut gpui::App,
    ) {
        let mut p = this.borrow_mut();
        let source = request.as_ref().map(|r| r.source.clone());
        if p.source != source {
            p.loading = None;
            p.replaced = true;
            if source.is_none() {
                p.forget_frames(window, cx);
                p.shown = None;
            }
            p.source = source;
        }
        let Some(request) = request else {
            p.wanted = None;
            return;
        };
        if p.wanted.as_ref() == Some(&request) {
            if p.shown.is_some() {
                ImagePipeline::global().touch(&request);
            }
            return;
        }
        p.wanted = Some(request.clone());
        let pipeline = ImagePipeline::global();
        if let Some(decoded) = pipeline.get(&request) {
            p.loading = None;
            p.report(Ok(&decoded), cx);
            p.show(decoded, window, cx);
            return;
        }
        if let Some(error) = pipeline.failed(&request) {
            p.loading = None;
            p.report(Err(&error), cx);
            return;
        }
        let (tx, rx) = oneshot::channel();
        pipeline.load(request.clone(), move |result| {
            let _ = tx.send(result);
        });
        let weak: Weak<RefCell<Picture>> = Rc::downgrade(this);
        let handle = window.window_handle();
        p.loading = Some(cx.spawn(async move |cx| {
            let Ok(result) = rx.await else { return };
            let _ = handle.update(cx, |_, window, cx| {
                let Some(this) = weak.upgrade() else { return };
                let mut p = this.borrow_mut();
                if p.wanted.as_ref() != Some(&request) {
                    return;
                }
                p.loading = None;
                match result {
                    Ok(decoded) => {
                        p.report(Ok(&decoded), cx);
                        p.show(decoded, window, cx);
                    }
                    Err(error) => {
                        p.report(Err(&error), cx);
                    }
                }
                window.refresh();
            });
        }));
    }

    /// The GPU image to draw now, scheduling the next frame of an animation.
    pub fn current(
        this: &Rc<RefCell<Picture>>,
        animate: bool,
        crop: Option<f32>,
        window: &mut gpui::Window,
        cx: &mut gpui::App,
    ) -> Option<Arc<RenderImage>> {
        let mut p = this.borrow_mut();
        let decoded = p.shown.clone()?;
        let playing = animate && decoded.is_animated() && !cx.reduce_motion();
        let index = if playing {
            let elapsed = p.started.map_or(Duration::ZERO, |s| s.elapsed());
            let index = decoded.frame_at(elapsed);
            if p.tick.is_none() {
                let next = next_change(&decoded, elapsed);
                let weak = Rc::downgrade(this);
                let handle = window.window_handle();
                p.tick = Some(cx.spawn(async move |cx| {
                    cx.background_executor().timer(next).await;
                    let _ = handle.update(cx, |_, window, _| {
                        if let Some(this) = weak.upgrade() {
                            this.borrow_mut().tick = None;
                        }
                        window.refresh();
                    });
                }));
            }
            index
        } else {
            0
        };
        if p.crop != crop {
            p.forget_frames(window, cx);
            p.frames = vec![None; decoded.frames.len()];
            p.crop = crop;
        }
        if p.frames.get(index).is_some_and(Option::is_none) {
            p.frames[index] = render_image(&decoded, index, crop);
        }
        p.frames.get(index).cloned().flatten()
    }

    pub fn attach(&mut self, sink: Option<Sink>, app: Option<AsyncApp>) {
        self.sink = sink;
        if self.app.is_none() {
            self.app = app;
        }
    }
}

/// How long until the animation shows its next frame.
fn next_change(decoded: &Decoded, elapsed: Duration) -> Duration {
    let total = decoded.loop_duration();
    if total.is_zero() {
        return Duration::from_millis(100);
    }
    let mut t = Duration::from_nanos((elapsed.as_nanos() % total.as_nanos()) as u64);
    for frame in &decoded.frames {
        if t < frame.delay {
            return (frame.delay - t).max(Duration::from_millis(8));
        }
        t -= frame.delay;
    }
    Duration::from_millis(16)
}

/// A GPUI image filling its parent, clipped to `corners`.
pub(crate) fn picture(image: Arc<RenderImage>, fit: ImageFit, corners: &StyleRefinement) -> AnyElement {
    let mut el = img(gpui::ImageSource::Render(image)).size_full().object_fit(match fit {
        ImageFit::Cover => ObjectFit::Cover,
        ImageFit::Contain => ObjectFit::Contain,
    });
    el.style().corner_radii = corners.corner_radii.clone();
    el.into_any_element()
}

pub(super) fn image(
    b: &Build,
    id: NodeId,
    p: &ImageProps,
    marks: &Marks,
    window: &mut gpui::Window,
    cx: &mut gpui::App,
) -> AnyElement {
    let (ctl, picture_state) = b.ctl(id, false, window, cx, Picture::default);
    let theme = b.theme();
    let pipeline = ImagePipeline::global();
    let (w, h) = (fixed(p.style.size.width), fixed(p.style.size.height));
    let natural = p.natural_size(w, pipeline);
    let aspect = natural.width / natural.height.max(1.);
    let size = match b.backend.bounds(id) {
        Some(r) if r.size.width > 0. && r.size.height > 0. => (r.size.width, r.size.height),
        _ => match (w, h) {
            (Some(w), Some(h)) => (w, h),
            (Some(w), None) => (w, w / aspect),
            (None, Some(h)) => (h * aspect, h),
            (None, None) => (natural.width, natural.height),
        },
    };
    let scale = window.scale_factor();
    let bleed = p.bleed(scale);
    let request = p.source.clone().map(|s| {
        DecodeRequest::at(s, size.0 + 2. * bleed, size.1 + 2. * bleed, p.decode_scale(scale), p.fit)
            .tinted(p.tint)
            .blurred(p.blur_sigma(scale))
    });
    picture_state.borrow_mut().attach(Some(ctl.sink.clone()), b.backend.state.borrow().app.clone());
    Picture::want(&picture_state, request, window, cx);
    let crop = (p.fit == ImageFit::Cover).then(|| (size.0 + 2. * bleed) / (size.1 + 2. * bleed).max(1.));
    let shown = Picture::current(&picture_state, p.animate, crop, window, cx);

    let mut root = div()
        .id(ElementId::Integer(id.0))
        .role(gpui::Role::Image)
        .aria_label(p.alt.clone().unwrap_or_default())
        .overflow_hidden();
    // A side the style leaves open follows the image's aspect, so the placeholder has the final
    // shape before anything decodes.
    root = match (w, h) {
        (None, None) => root.w(px(natural.width)).h(px(natural.height)),
        (Some(_), None) | (None, Some(_)) => root.aspect_ratio(aspect),
        _ => root,
    };
    let root = with_style(root, &p.style);
    match shown {
        Some(image) if bleed > 0. => root
            .relative()
            .child(
                div()
                    .absolute()
                    .top(px(-bleed))
                    .left(px(-bleed))
                    .w(px(size.0 + 2. * bleed))
                    .h(px(size.1 + 2. * bleed))
                    .child(picture(image, p.fit, &StyleRefinement::default())),
            )
            .into_any_element(),
        Some(image) => root.child(picture(image, p.fit, &p.style)).into_any_element(),
        None => {
            let fill = theme.surface.raised;
            marks.fill("image placeholder", fill, &[]);
            root.bg(fill).into_any_element()
        }
    }
}
