//! The media family on AppKit (`nd_components::media`), registered as custom widgets so the
//! backend frames, focuses and themes them like any node.
//!
//! Neither AppKit nor any other platform has a seek bar with a buffered range, a lyrics view, a
//! chat bubble or an avatar, so those are layer trees painted from the theme in effect at the
//! node (on macOS its tokens resolve from `NSColor`'s semantic colours). The shelf is an
//! `NSScrollView` with the OS's momentum, rubber band and overlay scroller. Images are layer
//! contents decoded by the shared pipeline off the main thread.

mod chat;
mod image;
mod lyrics;
mod seek;
mod shelf;

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use nd_components::media::{AvatarKind, ChatBubbleKind, ImageKind, LyricsKind, SeekBarKind, ShelfKind};
use nd_ui::prelude::Hsla;
use objc2::rc::Retained;
use objc2::{AnyThread, DefinedClass, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSColor, NSEvent, NSResponder, NSTrackingArea, NSTrackingAreaOptions, NSView, NSWorkspace};
use objc2_core_foundation::CGFloat;
use objc2_core_graphics::CGColor;
use objc2_foundation::{MainThreadMarker, NSPoint, NSRect, NSTimer};

use crate::look::Color;

pub(crate) fn register() {
    crate::custom::register::<ImageKind>(image::ImageWidget);
    crate::custom::register::<AvatarKind>(chat::AvatarWidget);
    crate::custom::register::<ChatBubbleKind>(chat::BubbleWidget);
    crate::custom::register::<SeekBarKind>(seek::SeekWidget);
    crate::custom::register::<LyricsKind>(lyrics::LyricsWidget);
    crate::custom::register::<ShelfKind>(shelf::ShelfWidget);
}

/// What a media view does with input and layout. The view holds it weakly; the node's native
/// object owns it.
pub(crate) trait Input {
    fn mouse_down(&self, _at: NSPoint, _event: &NSEvent) {}
    fn mouse_dragged(&self, _at: NSPoint, _event: &NSEvent) {}
    fn mouse_up(&self, _at: NSPoint, _event: &NSEvent) {}
    fn hover(&self, _inside: bool, _at: NSPoint) {}
    /// `true` when it took the key.
    fn key_down(&self, _event: &NSEvent) -> bool {
        false
    }
    fn layout(&self) {}
    fn takes_focus(&self) -> bool {
        false
    }
}

pub struct MediaIvars {
    input: RefCell<Option<Weak<dyn Input>>>,
    tracking: RefCell<Option<Retained<NSTrackingArea>>>,
}

define_class!(
    // SAFETY: NSView has no subclassing requirements beyond calling super in the overrides
    // below, and this type does not implement Drop.
    #[unsafe(super(NSView, NSResponder))]
    #[thread_kind = MainThreadOnly]
    #[ivars = MediaIvars]
    pub(crate) struct NdMediaView;

    impl NdMediaView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool {
            self.input().is_some_and(|i| i.takes_focus())
        }

        #[unsafe(method(mouseDownCanMoveWindow))]
        fn mouse_down_can_move_window(&self) -> bool {
            false
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            if let Some(input) = self.input() {
                input.mouse_down(self.point(event), event);
            }
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, event: &NSEvent) {
            if let Some(input) = self.input() {
                input.mouse_dragged(self.point(event), event);
            }
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, event: &NSEvent) {
            if let Some(input) = self.input() {
                input.mouse_up(self.point(event), event);
            }
        }

        #[unsafe(method(mouseEntered:))]
        fn mouse_entered(&self, event: &NSEvent) {
            if let Some(input) = self.input() {
                input.hover(true, self.point(event));
            }
        }

        #[unsafe(method(mouseMoved:))]
        fn mouse_moved(&self, event: &NSEvent) {
            if let Some(input) = self.input() {
                input.hover(true, self.point(event));
            }
        }

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, event: &NSEvent) {
            if let Some(input) = self.input() {
                input.hover(false, self.point(event));
            }
        }

        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            let taken = self.input().is_some_and(|i| i.key_down(event));
            if !taken {
                let _: () = unsafe { msg_send![super(self), keyDown: event] };
            }
        }

        #[unsafe(method(layout))]
        fn layout(&self) {
            let _: () = unsafe { msg_send![super(self), layout] };
            if let Some(input) = self.input() {
                input.layout();
            }
        }

        #[unsafe(method(setFrameSize:))]
        fn set_frame_size(&self, size: objc2_foundation::NSSize) {
            let _: () = unsafe { msg_send![super(self), setFrameSize: size] };
            if let Some(input) = self.input() {
                input.layout();
            }
        }

        #[unsafe(method(updateTrackingAreas))]
        fn update_tracking_areas(&self) {
            if let Some(old) = self.ivars().tracking.borrow_mut().take() {
                self.removeTrackingArea(&old);
            }
            let options = NSTrackingAreaOptions::MouseEnteredAndExited
                | NSTrackingAreaOptions::MouseMoved
                | NSTrackingAreaOptions::ActiveInKeyWindow
                | NSTrackingAreaOptions::InVisibleRect;
            let area = unsafe {
                NSTrackingArea::initWithRect_options_owner_userInfo(
                    NSTrackingArea::alloc(),
                    NSRect::ZERO,
                    options,
                    Some(self),
                    None,
                )
            };
            self.addTrackingArea(&area);
            *self.ivars().tracking.borrow_mut() = Some(area);
            let _: () = unsafe { msg_send![super(self), updateTrackingAreas] };
        }
    }
);

impl NdMediaView {
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(MediaIvars { input: RefCell::new(None), tracking: RefCell::new(None) });
        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] };
        this.setWantsLayer(true);
        this
    }

    pub fn set_input(&self, input: Weak<dyn Input>) {
        *self.ivars().input.borrow_mut() = Some(input);
    }

    fn input(&self) -> Option<Rc<dyn Input>> {
        self.ivars().input.borrow().as_ref().and_then(Weak::upgrade)
    }

    fn point(&self, event: &NSEvent) -> NSPoint {
        self.convertPoint_fromView(event.locationInWindow(), None)
    }
}

pub(crate) fn cg(color: Hsla) -> Retained<CGColor> {
    Color::from(color).ns_color().CGColor()
}

pub(crate) fn ns(color: Hsla) -> Retained<NSColor> {
    Color::from(color).ns_color()
}

pub(crate) fn reduce_motion() -> bool {
    NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion()
}

/// The view's backing scale; 2 before it is in a window, the common Retina case.
pub(crate) fn scale_of(view: &NSView) -> CGFloat {
    view.window().map_or(2., |w| w.backingScaleFactor())
}

/// A repeating main-thread timer for animation, running while `tick` returns `true`.
#[derive(Default)]
pub(crate) struct Ticker {
    timer: RefCell<Option<Retained<NSTimer>>>,
    running: Cell<bool>,
}

impl Ticker {
    /// Starts ticking at `hz`; a running ticker keeps its timer.
    pub fn start(self: &Rc<Self>, hz: f64, tick: impl Fn() -> bool + 'static) {
        if self.running.replace(true) {
            return;
        }
        let me = Rc::downgrade(self);
        let block = block2::RcBlock::new(move |_timer: std::ptr::NonNull<NSTimer>| {
            let Some(me) = me.upgrade() else { return };
            if !tick() {
                me.stop();
            }
        });
        // SAFETY: the timer runs on the main run loop it is added to, the thread `tick` came from.
        let timer = unsafe { NSTimer::timerWithTimeInterval_repeats_block(1. / hz, true, &block) };
        unsafe {
            objc2_foundation::NSRunLoop::mainRunLoop().addTimer_forMode(&timer, objc2_foundation::NSRunLoopCommonModes)
        };
        *self.timer.borrow_mut() = Some(timer);
    }

    pub fn stop(&self) {
        self.running.set(false);
        if let Some(timer) = self.timer.borrow_mut().take() {
            timer.invalidate();
        }
    }
}

impl Drop for Ticker {
    fn drop(&mut self) {
        if let Some(timer) = self.timer.get_mut().take() {
            timer.invalidate();
        }
    }
}

/// The first corner radius a style sets, clamped to a circle for `rounded_full`.
pub(crate) fn style_radius(style: &nd_ui::StyleRefinement, width: f64, height: f64) -> f64 {
    let r = style.corner_radii.top_left.map(crate::look::length).unwrap_or(0.);
    f64::from(r).min(width.min(height) / 2.).max(0.)
}
