//! A window's display clock (ARCHITECTURE.md 5.3): `NSView.displayLink(target:selector:)`, which
//! follows the display the window is on, so each window ticks at its own display's rate. Paused
//! while idle; nothing requests frames unless something animates.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use objc2::rc::Retained;
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::NSView;
use objc2_foundation::{MainThreadMarker, NSObject, NSObjectProtocol, NSRunLoop, NSRunLoopCommonModes};
use objc2_quartz_core::{CADisplayLink, CAFrameRateRange};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameTick {
    /// When the previous frame was displayed, in `CACurrentMediaTime` seconds.
    pub timestamp: f64,
    /// When the frame being prepared now will be displayed. Animations sample at this time.
    pub target_timestamp: f64,
}

type Callback = Rc<dyn Fn(FrameTick)>;

struct Inner {
    link: RefCell<Option<Retained<CADisplayLink>>>,
    callback: RefCell<Option<Callback>>,
    now: Cell<f64>,
    ticks: Cell<u64>,
    /// First and last target timestamps and tick count of the current animating run.
    run: Cell<Option<(f64, f64, u64)>>,
    diag: bool,
}

impl Drop for Inner {
    fn drop(&mut self) {
        if let Some(link) = self.link.get_mut() {
            link.invalidate();
        }
    }
}

pub struct TargetIvars {
    clock: Weak<Inner>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements and this type does not implement Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = TargetIvars]
    struct NdFrameTarget;

    unsafe impl NSObjectProtocol for NdFrameTarget {}

    impl NdFrameTarget {
        #[unsafe(method(tick:))]
        fn tick(&self, link: &CADisplayLink) {
            let Some(clock) = self.ivars().clock.upgrade() else { return };
            let tick = FrameTick { timestamp: link.timestamp(), target_timestamp: link.targetTimestamp() };
            clock.now.set(tick.target_timestamp);
            clock.ticks.set(clock.ticks.get() + 1);
            let run = clock.run.get().map_or((tick.target_timestamp, tick.target_timestamp, 0), |r| r);
            clock.run.set(Some((run.0, tick.target_timestamp, run.2 + 1)));
            let callback = clock.callback.borrow().clone();
            if let Some(callback) = callback {
                callback(tick);
            }
        }
    }
);

#[derive(Clone)]
pub struct FrameClock {
    inner: Rc<Inner>,
}

impl FrameClock {
    pub(crate) fn new(mtm: MainThreadMarker, view: &NSView, diag: bool) -> Self {
        let inner = Rc::new(Inner {
            link: RefCell::new(None),
            callback: RefCell::new(None),
            now: Cell::new(0.),
            ticks: Cell::new(0),
            run: Cell::new(None),
            diag,
        });
        let target = NdFrameTarget::alloc(mtm).set_ivars(TargetIvars { clock: Rc::downgrade(&inner) });
        let target: Retained<NdFrameTarget> = unsafe { msg_send![super(target), init] };
        // The link retains its target until invalidated (in `Inner::drop`).
        let link = unsafe { view.displayLinkWithTarget_selector(&target, sel!(tick:)) };
        link.setPaused(true);
        unsafe { link.addToRunLoop_forMode(&NSRunLoop::mainRunLoop(), NSRunLoopCommonModes) };
        *inner.link.borrow_mut() = Some(link);
        Self { inner }
    }

    pub fn on_frame(&self, callback: impl Fn(FrameTick) + 'static) {
        *self.inner.callback.borrow_mut() = Some(Rc::new(callback));
    }

    /// Ticks every display refresh while on (up to 120 Hz on ProMotion), none while off.
    pub fn set_animating(&self, on: bool) {
        let link = self.inner.link.borrow();
        let Some(link) = link.as_ref() else { return };
        if on != link.isPaused() {
            return;
        }
        if on {
            link.setPreferredFrameRateRange(CAFrameRateRange { minimum: 60., maximum: 120., preferred: 120. });
            self.inner.run.set(None);
        } else if self.inner.diag
            && let Some((first, last, ticks)) = self.inner.run.get()
            && ticks > 1
        {
            let hz = (ticks - 1) as f64 / (last - first);
            eprintln!("nd: frame clock {ticks} ticks at {hz:.1} Hz");
        }
        link.setPaused(!on);
    }

    /// The target timestamp of the latest frame.
    pub fn now(&self) -> f64 {
        self.inner.now.get()
    }

    pub fn ticks(&self) -> u64 {
        self.inner.ticks.get()
    }
}
