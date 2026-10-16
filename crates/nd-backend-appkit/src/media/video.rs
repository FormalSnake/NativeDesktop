//! `video()` on AppKit: an `AVPlayer` drawn by an `AVPlayerLayer` filling the view. A 10 Hz
//! ticker watches the item's status until it is ready or has failed; the end of playback comes
//! from AVFoundation's notification.

use std::cell::{Cell, RefCell};
use std::ptr::NonNull;
use std::rc::{Rc, Weak};

use block2::RcBlock;
use nd_components::media::{ImageFit, VideoEnded, VideoFailed, VideoProps, VideoReady};
use nd_theme::Theme;
use nd_ui::events::Emit;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObjectProtocol, ProtocolObject};
use objc2_app_kit::NSView;
use objc2_av_foundation::{
    AVLayerVideoGravityResizeAspect, AVLayerVideoGravityResizeAspectFill, AVPlayer, AVPlayerActionAtItemEnd,
    AVPlayerItem, AVPlayerItemDidPlayToEndTimeNotification, AVPlayerItemStatus, AVPlayerLayer,
};
use objc2_core_media::{CMTimeFlags, kCMTimeZero};
use objc2_foundation::{MainThreadMarker, NSNotification, NSNotificationCenter, NSOperationQueue, NSString, NSURL};
use objc2_quartz_core::CATransaction;

use super::{Input, NdMediaView, Ticker, cg};
use crate::custom::{CustomCx, CustomNative, CustomPaint, CustomWidget, Waker};

pub(crate) struct VideoWidget;

impl CustomWidget for VideoWidget {
    fn create(&self, cx: &CustomCx<'_>) -> Box<dyn CustomNative> {
        let view = NdMediaView::new(cx.mtm());
        let layer = unsafe { AVPlayerLayer::playerLayerWithPlayer(None) };
        view.layer().expect("layer-backed").addSublayer(&layer);
        let inner = Rc::new(VideoInner {
            mtm: cx.mtm(),
            view,
            layer,
            playback: RefCell::new(None),
            props: RefCell::new(None),
            theme: RefCell::new(cx.theme().clone()),
            emit: cx.emitter(),
            wake: cx.waker(),
            ticker: Rc::default(),
        });
        inner.view.set_input(Rc::downgrade(&inner) as Weak<dyn Input>);
        inner.apply(props(cx));
        Box::new(VideoNative { inner })
    }
}

fn props<'a>(cx: &'a CustomCx<'_>) -> &'a VideoProps {
    cx.props().downcast_ref::<VideoProps>().expect("video node needs VideoProps")
}

/// A path, or a URL with a scheme.
fn url_of(source: &str) -> Option<Retained<NSURL>> {
    if source.contains("://") {
        NSURL::URLWithString(&NSString::from_str(source)).filter(|u| u.scheme().is_some())
    } else {
        Some(NSURL::fileURLWithPath(&NSString::from_str(source)))
    }
}

struct Playback {
    player: Retained<AVPlayer>,
    item: Retained<AVPlayerItem>,
    end_observer: Retained<ProtocolObject<dyn NSObjectProtocol>>,
    /// Ready or failed has been reported for this item.
    settled: Cell<bool>,
}

impl Drop for Playback {
    fn drop(&mut self) {
        unsafe { self.player.pause() };
        // SAFETY: the observer came from this center's block API.
        unsafe { NSNotificationCenter::defaultCenter().removeObserver(self.end_observer.as_ref()) };
    }
}

struct VideoInner {
    mtm: MainThreadMarker,
    view: Retained<NdMediaView>,
    layer: Retained<AVPlayerLayer>,
    playback: RefCell<Option<Playback>>,
    props: RefCell<Option<VideoProps>>,
    theme: RefCell<Theme>,
    emit: Emit,
    wake: Waker,
    ticker: Rc<Ticker>,
}

impl VideoInner {
    fn apply(self: &Rc<Self>, props: &VideoProps) {
        let old = self.props.replace(Some(props.clone()));
        if old.as_ref().map(|p| &p.source) != Some(&props.source) {
            self.open(props.source.as_deref());
        }
        let gravity = match props.fit {
            ImageFit::Cover => unsafe { AVLayerVideoGravityResizeAspectFill },
            ImageFit::Contain => unsafe { AVLayerVideoGravityResizeAspect },
        };
        if let Some(gravity) = gravity {
            unsafe { self.layer.setVideoGravity(gravity) };
        }
        self.view.layer().expect("layer-backed").setBackgroundColor(Some(&cg(self.theme.borrow().surface.raised)));
        self.view.setToolTip(props.tooltip.as_deref().map(NSString::from_str).as_deref());
        if let Some(p) = self.playback.borrow().as_ref() {
            unsafe {
                p.player.setMuted(props.muted);
                if props.playing {
                    p.player.play();
                } else {
                    p.player.pause();
                }
            }
        }
    }

    fn open(self: &Rc<Self>, source: Option<&str>) {
        self.ticker.stop();
        self.playback.replace(None);
        unsafe { self.layer.setPlayer(None) };
        let Some(source) = source else { return };
        let Some(url) = url_of(source) else {
            self.report(VideoFailed { error: format!("not a path or URL: {source}") });
            return;
        };
        let item = unsafe { AVPlayerItem::playerItemWithURL(&url, self.mtm) };
        let player = unsafe { AVPlayer::playerWithPlayerItem(Some(&item), self.mtm) };
        unsafe { player.setActionAtItemEnd(AVPlayerActionAtItemEnd::Pause) };
        let me = Rc::downgrade(self);
        let on_end = RcBlock::new(move |_: NonNull<NSNotification>| {
            if let Some(me) = me.upgrade() {
                me.ended();
            }
        });
        // SAFETY: `Playback` removes the observer when it drops; the main queue runs the block on
        // the main thread, where the view lives.
        let end_observer = unsafe {
            NSNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(
                Some(AVPlayerItemDidPlayToEndTimeNotification),
                Some(&item as &AnyObject),
                Some(&NSOperationQueue::mainQueue()),
                &on_end,
            )
        };
        unsafe { self.layer.setPlayer(Some(&player)) };
        self.playback.replace(Some(Playback { player, item, end_observer, settled: Cell::new(false) }));
        let me = Rc::downgrade(self);
        self.ticker.start(10., move || me.upgrade().is_some_and(|me| me.poll()));
    }

    /// Reports ready or failed once; `false` once there is nothing left to watch.
    fn poll(&self) -> bool {
        let event = {
            let playback = self.playback.borrow();
            let Some(p) = playback.as_ref().filter(|p| !p.settled.get()) else { return false };
            match unsafe { p.item.status() } {
                AVPlayerItemStatus::ReadyToPlay => {
                    p.settled.set(true);
                    let size = unsafe { p.item.presentationSize() };
                    let duration = unsafe { p.item.duration() };
                    let duration = (duration.flags.contains(CMTimeFlags::Valid)
                        && !duration.flags.intersects(CMTimeFlags::ImpliedValueFlagsMask))
                    .then(|| unsafe { duration.seconds() })
                    .filter(|s| s.is_finite());
                    Ok(VideoReady { width: size.width as u32, height: size.height as u32, duration })
                }
                AVPlayerItemStatus::Failed => {
                    p.settled.set(true);
                    let error = unsafe { p.item.error() }
                        .map(|e| e.localizedDescription().to_string())
                        .unwrap_or_else(|| "the video could not be played".into());
                    Err(VideoFailed { error })
                }
                _ => return true,
            }
        };
        match event {
            Ok(ready) => self.report(ready),
            Err(failed) => self.report(failed),
        }
        false
    }

    fn ended(&self) {
        let looping = self.props.borrow().as_ref().is_some_and(|p| p.looping);
        if looping {
            if let Some(p) = self.playback.borrow().as_ref() {
                unsafe {
                    p.player.seekToTime(kCMTimeZero);
                    p.player.play();
                }
            }
            return;
        }
        self.report(VideoEnded);
    }

    fn report<E: 'static>(&self, event: E) {
        self.emit.emit(event);
        self.wake.wake();
    }
}

impl Input for VideoInner {
    fn layout(&self) {
        CATransaction::begin();
        CATransaction::setDisableActions(true);
        self.layer.setFrame(self.view.bounds());
        let radius = self
            .props
            .borrow()
            .as_ref()
            .map_or(0., |p| super::style_radius(&p.style, self.view.bounds().size.width, self.view.bounds().size.height));
        self.view.layer().expect("layer-backed").setCornerRadius(radius);
        self.view.layer().expect("layer-backed").setMasksToBounds(radius > 0.);
        CATransaction::commit();
    }
}

struct VideoNative {
    inner: Rc<VideoInner>,
}

impl CustomNative for VideoNative {
    fn view(&self) -> &NSView {
        &self.inner.view
    }

    fn update(&mut self, cx: &CustomCx<'_>) {
        self.inner.apply(props(cx));
        self.inner.layout();
    }

    fn set_theme(&mut self, theme: &Theme) {
        *self.inner.theme.borrow_mut() = theme.clone();
        let props = self.inner.props.borrow().clone();
        if let Some(props) = props {
            self.inner.apply(&props);
        }
    }

    fn paints(&self) -> Vec<CustomPaint> {
        let fill = self.inner.theme.borrow().surface.raised;
        vec![CustomPaint { label: "video".into(), color: fill, under: Vec::new(), text: None }]
    }
}
