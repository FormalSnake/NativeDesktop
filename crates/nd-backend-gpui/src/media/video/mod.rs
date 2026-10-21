//! `video()` on GPUI. The platform decoder plays the file on a thread of its own, with its own
//! clock and sound: Media Foundation's media engine on Windows (`mf.rs`), GStreamer's playbin on
//! Linux (`gst.rs`, loaded with `dlopen`). It hands over BGRA frames scaled to the box, each of
//! which becomes a GPUI image in place of the last. Where there is no decoder, or it cannot open
//! the file, the node draws its placeholder and reports [`VideoFailed`].

#[cfg(target_os = "linux")]
mod gst;
#[cfg(windows)]
mod mf;

use std::cell::RefCell;
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use futures::StreamExt;
use futures::channel::mpsc::{UnboundedSender, unbounded};
use nd_components::{ImageFit, VideoEnded, VideoFailed, VideoProgress, VideoProps, VideoReady};
use nd_core::NodeId;
use nd_gpui::gpui::{
    self, AnyElement, AsyncApp, ElementId, InteractiveElement, IntoElement, ParentElement, RenderImage, SharedString,
    StatefulInteractiveElement, Styled, Task, div, px,
};

use super::fixed;
use super::image::picture;
use crate::forms::{Sink, with_style};
use crate::nav::Marks;
use crate::render::Build;

/// One decoded picture, `width * height` BGRA pixels with no row padding.
pub(crate) struct Frame {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
    /// Its time in the media, in seconds.
    pub position: f64,
}

#[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
pub(crate) enum Event {
    /// The media opened; the size is the picture as shown, rotation applied.
    Ready {
        width: u32,
        height: u32,
        duration: Option<f64>,
    },
    /// A new frame waits in [`Shared::frame`].
    Frame,
    Ended,
    Failed(String),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Command {
    Playing(bool),
    Muted(bool),
    Looping(bool),
    Seek(f64),
    /// The box in device pixels, and whether the picture covers it rather than fitting inside.
    Fit {
        width: u32,
        height: u32,
        cover: bool,
    },
}

/// What a decoder thread shares with its node: the newest frame (an older one not yet shown is
/// replaced) and the events channel.
#[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
pub(crate) struct Shared {
    frame: Mutex<Option<Frame>>,
    pending: AtomicBool,
    events: UnboundedSender<Event>,
}

#[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
impl Shared {
    pub fn event(&self, event: Event) {
        let _ = self.events.unbounded_send(event);
    }

    pub fn frame(&self, frame: Frame) {
        *self.frame.lock().expect("video frame") = Some(frame);
        if !self.pending.swap(true, Ordering::AcqRel) {
            self.event(Event::Frame);
        }
    }

    fn take(&self) -> Option<Frame> {
        self.pending.store(false, Ordering::Release);
        self.frame.lock().expect("video frame").take()
    }
}

/// The size to decode a `native` picture at for `fit`: inside the box (or covering it), never
/// larger than the source, even sides for the decoders' chroma.
#[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
pub(crate) fn fit_size(native: (u32, u32), fit: Option<(u32, u32, bool)>) -> (u32, u32) {
    let (w, h) = (native.0.max(1) as f64, native.1.max(1) as f64);
    let scale = match fit {
        Some((bw, bh, cover)) if bw > 0 && bh > 0 => {
            let (sx, sy) = (bw as f64 / w, bh as f64 / h);
            if cover { sx.max(sy) } else { sx.min(sy) }.min(1.)
        }
        _ => 1.,
    };
    let even = |v: f64| ((v.round() as u32).max(2) + 1) & !1;
    (even(w * scale).min(native.0.max(2)), even(h * scale).min(native.1.max(2)))
}

#[cfg(windows)]
use mf::open as open_decoder;

#[cfg(target_os = "linux")]
use gst::open as open_decoder;

#[cfg(not(any(windows, target_os = "linux")))]
fn open_decoder(_: &str, _: Arc<Shared>) -> Result<std::sync::mpsc::Sender<Command>, String> {
    Err("this backend has no video surface".into())
}

/// A video node's playback: the decoder thread's command line, the image on screen and what was
/// last asked of the decoder and told to the app.
#[derive(Default)]
struct Playback {
    source: Option<SharedString>,
    commands: Option<std::sync::mpsc::Sender<Command>>,
    shared: Option<Arc<Shared>>,
    listen: Option<Task<()>>,
    image: Option<Arc<RenderImage>>,
    props: Option<VideoProps>,
    playing: Option<bool>,
    muted: Option<bool>,
    looping: Option<bool>,
    seek: Option<u64>,
    fit: Option<(u32, u32, bool)>,
    ready: bool,
    ended: bool,
    position: f64,
    /// The last position and play state reported.
    reported: Option<(f64, bool)>,
    /// A seek in flight: reported once a frame from there lands.
    seeking: Option<f64>,
    sink: Option<Sink>,
    app: Option<AsyncApp>,
}

impl Drop for Playback {
    fn drop(&mut self) {
        if let (Some(app), Some(image)) = (self.app.take(), self.image.take()) {
            app.spawn(async move |cx| cx.update(|cx| cx.drop_image(image, None))).detach();
        }
    }
}

impl Playback {
    fn send(&self, command: Command) {
        if let Some(commands) = &self.commands {
            let _ = commands.send(command);
        }
    }

    fn close(&mut self, window: &mut gpui::Window, cx: &mut gpui::App) {
        self.commands = None;
        self.shared = None;
        self.listen = None;
        if let Some(image) = self.image.take() {
            cx.drop_image(image, Some(window));
        }
        (self.playing, self.muted, self.looping, self.seek, self.fit) = (None, None, None, None, None);
        (self.ready, self.ended, self.position, self.reported, self.seeking) = (false, false, 0., None, None);
    }

    fn open(this: &Rc<RefCell<Self>>, source: &str, window: &mut gpui::Window, cx: &mut gpui::App) {
        let (tx, mut rx) = unbounded();
        let shared = Arc::new(Shared { frame: Mutex::new(None), pending: AtomicBool::new(false), events: tx });
        match open_decoder(source, shared.clone()) {
            Ok(commands) => {
                let mut p = this.borrow_mut();
                p.commands = Some(commands);
                p.shared = Some(shared);
            }
            Err(error) => {
                let sink = this.borrow().sink.clone();
                if let Some(sink) = sink {
                    sink.send(VideoFailed { error }, cx);
                }
                return;
            }
        }
        let weak: Weak<RefCell<Self>> = Rc::downgrade(this);
        let handle = window.window_handle();
        let listen = cx.spawn(async move |cx| {
            while let Some(event) = rx.next().await {
                let alive = handle.update(cx, |_, window, cx| {
                    let Some(this) = weak.upgrade() else { return false };
                    Self::handle(&this, event, window, cx);
                    true
                });
                if !matches!(alive, Ok(true)) {
                    return;
                }
            }
        });
        this.borrow_mut().listen = Some(listen);
    }

    fn handle(this: &Rc<RefCell<Self>>, event: Event, window: &mut gpui::Window, cx: &mut gpui::App) {
        let mut p = this.borrow_mut();
        let sink = p.sink.clone();
        match event {
            Event::Ready { width, height, duration } => {
                p.ready = true;
                if let Some(sink) = &sink {
                    sink.send(VideoReady { width, height, duration }, cx);
                }
                if let Some(props) = p.props.clone() {
                    p.apply(&props, cx);
                }
            }
            Event::Frame => {
                let Some(frame) = p.shared.as_ref().and_then(|s| s.take()) else { return };
                let Some(buffer) = ::image::RgbaImage::from_raw(frame.width, frame.height, frame.bgra) else { return };
                let image = Arc::new(RenderImage::new(vec![::image::Frame::new(buffer)]));
                if let Some(old) = p.image.replace(image) {
                    cx.drop_image(old, Some(window));
                }
                p.position = frame.position;
                p.progress(cx);
                window.refresh();
            }
            Event::Ended => {
                p.ended = true;
                if !p.props.as_ref().is_some_and(|props| props.looping)
                    && let Some(sink) = &sink
                {
                    sink.send(VideoEnded, cx);
                }
            }
            Event::Failed(error) => {
                p.commands = None;
                p.shared = None;
                if let Some(sink) = &sink {
                    sink.send(VideoFailed { error }, cx);
                }
                window.refresh();
            }
        }
    }

    /// Tells the app where playback is when it arrived from a seek, started, stopped, or moved
    /// a second since it last heard (any move while paused).
    fn progress(&mut self, cx: &mut gpui::App) {
        let playing = self.playing == Some(true) && self.ready && !self.ended;
        let position = self.position;
        let moved = match (self.seeking, self.reported) {
            (Some(target), _) => {
                let arrived = (position - target).abs() < 0.5;
                if arrived {
                    self.seeking = None;
                }
                arrived
            }
            (None, None) => true,
            // Paused, a frame still landing (the decoder stops a frame or two late) moves the
            // position the app shows.
            (None, Some((last, was))) => {
                was != playing || (position - last).abs() >= 1. || (!playing && position != last)
            }
        };
        if moved && position.is_finite() {
            self.reported = Some((position, playing));
            if let Some(sink) = &self.sink {
                sink.send(VideoProgress { position, playing }, cx);
            }
        }
    }

    /// Sends the decoder what changed in the props. A seek waits for the media to open.
    fn apply(&mut self, props: &VideoProps, cx: &mut gpui::App) {
        self.props = Some(props.clone());
        if self.commands.is_none() {
            return;
        }
        if self.muted != Some(props.muted) {
            self.muted = Some(props.muted);
            self.send(Command::Muted(props.muted));
        }
        if self.looping != Some(props.looping) {
            self.looping = Some(props.looping);
            self.send(Command::Looping(props.looping));
        }
        if self.ready
            && let Some((request, seconds)) = props.seek
            && self.seek.replace(request) != Some(request)
        {
            self.send(Command::Seek(seconds));
            self.position = seconds;
            self.seeking = Some(seconds);
            self.ended = false;
        }
        if self.playing != Some(props.playing) {
            self.playing = Some(props.playing);
            if props.playing {
                self.ended = false;
            }
            self.send(Command::Playing(props.playing));
            if self.ready {
                self.progress(cx);
            }
        }
    }

    fn fit(&mut self, fit: (u32, u32, bool)) {
        // Steps of 64 device pixels, so a window being resized does not rescale every frame.
        let step = |v: u32| v.div_ceil(64) * 64;
        let fit = (step(fit.0), step(fit.1), fit.2);
        if self.fit != Some(fit) {
            self.fit = Some(fit);
            self.send(Command::Fit { width: fit.0, height: fit.1, cover: fit.2 });
        }
    }
}

/// A released video node's playback stops now, sound included, even while something still
/// holds its state.
pub(crate) fn released(state: &Rc<dyn std::any::Any>) {
    if let Ok(playback) = state.clone().downcast::<RefCell<Playback>>()
        && let Ok(mut p) = playback.try_borrow_mut()
    {
        (p.commands, p.shared, p.listen) = (None, None, None);
    }
}

pub(super) fn video(
    b: &Build,
    id: NodeId,
    p: &VideoProps,
    marks: &Marks,
    window: &mut gpui::Window,
    cx: &mut gpui::App,
) -> AnyElement {
    let (ctl, state) = b.ctl(id, false, window, cx, Playback::default);
    {
        let mut s = state.borrow_mut();
        s.sink = Some(ctl.sink.clone());
        if s.app.is_none() {
            s.app = b.backend.state.borrow().app.clone();
        }
    }
    if state.borrow().source != p.source && !b.ghost {
        state.borrow_mut().close(window, cx);
        state.borrow_mut().source = p.source.clone();
        if let Some(source) = p.source.clone() {
            Playback::open(&state, &source, window, cx);
        }
    }
    let size = match b.backend.bounds(id) {
        Some(r) if r.size.width > 0. && r.size.height > 0. => Some((r.size.width, r.size.height)),
        _ => fixed(p.style.size.width).zip(fixed(p.style.size.height)),
    };
    {
        let mut s = state.borrow_mut();
        s.apply(p, cx);
        if let Some((w, h)) = size {
            let scale = window.scale_factor();
            s.fit(((w * scale).ceil() as u32, (h * scale).ceil() as u32, p.fit == ImageFit::Cover));
        }
    }
    let shown = state.borrow().image.clone();
    let root = div()
        .id(ElementId::Integer(id.0))
        .role(gpui::Role::Image)
        .aria_label(p.alt.clone().unwrap_or_default())
        .overflow_hidden()
        .flex()
        .items_center()
        .justify_center();
    let root = with_style(root, &p.style);
    match shown {
        // The picture is sized here rather than by the image's object fit, which stretched it to
        // the box on Windows.
        Some(image) => {
            let frame = image.size(0);
            let aspect = frame.width.0 as f32 / frame.height.0.max(1) as f32;
            let (w, h) = size.unwrap_or((frame.width.0 as f32, frame.height.0 as f32));
            let (pw, ph) = match p.fit {
                ImageFit::Contain if w / h > aspect => (h * aspect, h),
                ImageFit::Contain => (w, w / aspect),
                ImageFit::Cover if w / h > aspect => (w, w / aspect),
                ImageFit::Cover => (h * aspect, h),
            };
            root.child(div().flex_none().w(px(pw)).h(px(ph)).child(picture(image, ImageFit::Contain, &p.style)))
                .into_any_element()
        }
        None => {
            let fill = b.theme().surface.raised;
            marks.fill("video", fill, &[]);
            root.bg(fill).into_any_element()
        }
    }
}

#[cfg(all(test, any(windows, target_os = "linux")))]
mod tests {
    use std::time::{Duration, Instant};

    use futures::channel::mpsc::{TryRecvError, UnboundedReceiver};

    use super::*;

    struct Run {
        commands: std::sync::mpsc::Sender<Command>,
        shared: Arc<Shared>,
        events: UnboundedReceiver<Event>,
    }

    fn open(name: &str) -> Run {
        let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
        let (tx, events) = unbounded();
        let shared = Arc::new(Shared { frame: Mutex::new(None), pending: AtomicBool::new(false), events: tx });
        let commands = open_decoder(&path, shared.clone()).expect("decoder opens");
        commands.send(Command::Muted(true)).unwrap();
        Run { commands, shared, events }
    }

    impl Run {
        /// Events until `done` says stop, failing on a decoder error or after ten seconds.
        fn until(&mut self, mut done: impl FnMut(&Event, Option<&Frame>) -> bool) {
            let deadline = Instant::now() + Duration::from_secs(10);
            while Instant::now() < deadline {
                match self.events.try_recv() {
                    Ok(Event::Failed(e)) => panic!("decoder failed: {e}"),
                    Ok(event) => {
                        let frame = matches!(event, Event::Frame).then(|| self.shared.take()).flatten();
                        if done(&event, frame.as_ref()) {
                            return;
                        }
                    }
                    Err(TryRecvError::Closed) => panic!("decoder stopped"),
                    Err(TryRecvError::Empty) => std::thread::sleep(Duration::from_millis(5)),
                }
            }
            panic!("timed out");
        }

        fn ready(&mut self) -> (u32, u32, Option<f64>) {
            let mut ready = None;
            self.until(|e, _| {
                if let Event::Ready { width, height, duration } = e {
                    ready = Some((*width, *height, *duration));
                }
                ready.is_some()
            });
            ready.unwrap()
        }
    }

    #[test]
    fn h264_mp4_plays_scaled_to_the_box() {
        let mut run = open("video-h264.mp4");
        run.commands.send(Command::Fit { width: 160, height: 160, cover: false }).unwrap();
        let (w, h, duration) = run.ready();
        assert_eq!((w, h), (320, 180));
        assert!(duration.is_some_and(|d| (d - 3.).abs() < 0.2), "{duration:?}");
        run.commands.send(Command::Playing(true)).unwrap();
        let mut frames = 0;
        let mut last = -1.;
        run.until(|_, frame| {
            if let Some(f) = frame {
                assert_eq!(f.bgra.len(), (f.width * f.height * 4) as usize);
                if (f.width, f.height) == (160, 90) {
                    assert!(f.position >= last, "frames go forward");
                    last = f.position;
                    frames += 1;
                }
            }
            frames >= 10
        });
    }

    #[test]
    fn mpeg4_part2_plays() {
        let mut run = open("video-mpeg4.mp4");
        run.commands.send(Command::Fit { width: 1024, height: 640, cover: false }).unwrap();
        let (w, h, _) = run.ready();
        assert_eq!((w, h), (320, 240));
        run.commands.send(Command::Playing(true)).unwrap();
        let mut frames = 0;
        run.until(|_, frame| {
            if let Some(f) = frame {
                assert_eq!((f.width, f.height), (320, 240));
                frames += 1;
            }
            frames >= 5
        });
    }

    #[test]
    fn rotated_hevc_mov_stands_upright() {
        let mut run = open("video-hevc-rotated.mov");
        let (w, h, _) = run.ready();
        run.commands.send(Command::Playing(true)).unwrap();
        let mut size = None;
        run.until(|_, frame| {
            size = frame.map(|f| (f.width, f.height)).or(size);
            size.is_some()
        });
        assert_eq!((w, h), (176, 320), "ready reports the upright size");
        assert_eq!(size, Some((176, 320)));
    }

    #[test]
    fn a_paused_seek_shows_the_frame_there_and_the_end_is_reported() {
        let mut run = open("video-h264.mp4");
        run.ready();
        run.commands.send(Command::Seek(2.)).unwrap();
        run.until(|_, frame| frame.is_some_and(|f| (f.position - 2.).abs() < 0.1));
        run.commands.send(Command::Playing(true)).unwrap();
        run.until(|e, _| matches!(e, Event::Ended));
    }
}
