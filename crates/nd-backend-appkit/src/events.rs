//! Native callbacks push onto `nd-ui`'s event queue (ARCHITECTURE.md 3.13 rule 1) and then wake
//! the host, which drains the queue on a later main run loop turn: outside the callback, outside
//! any transaction and outside every entity lease.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use block2::RcBlock;
use objc2_foundation::{NSArray, NSRunLoop, NSRunLoopCommonModes};

#[derive(Default)]
struct Inner {
    callback: RefCell<Option<Rc<dyn Fn()>>>,
    scheduled: Cell<bool>,
}

/// Runs the host's callback once per main run loop turn in which something woke it.
#[derive(Clone, Default)]
pub struct Wake(Rc<Inner>);

impl Wake {
    pub fn set_callback(&self, callback: impl Fn() + 'static) {
        *self.0.callback.borrow_mut() = Some(Rc::new(callback));
    }

    pub fn wake(&self) {
        if self.0.scheduled.replace(true) {
            return;
        }
        let inner = Rc::downgrade(&self.0);
        defer(move || {
            let Some(inner) = inner.upgrade() else { return };
            inner.scheduled.set(false);
            let callback = inner.callback.borrow().clone();
            if let Some(callback) = callback {
                callback();
            }
        });
    }
}

/// Runs `f` on a later main run loop turn, in common modes so a nested tracking loop (a
/// button press, a live resize) still runs it.
pub(crate) fn defer(f: impl FnOnce() + 'static) {
    let f = Cell::new(Some(f));
    let block = RcBlock::new(move || {
        if let Some(f) = f.take() {
            f();
        }
    });
    let modes = NSArray::from_slice(&[unsafe { NSRunLoopCommonModes }]);
    unsafe { NSRunLoop::mainRunLoop().performInModes_block(&modes, &block) };
}

/// Runs `f` once, `seconds` from now, on the main run loop in common modes. Invalidating the
/// returned timer cancels it.
pub(crate) fn after(seconds: f64, f: impl FnOnce() + 'static) -> objc2::rc::Retained<objc2_foundation::NSTimer> {
    let f = Cell::new(Some(f));
    let block = RcBlock::new(move |_timer: std::ptr::NonNull<objc2_foundation::NSTimer>| {
        if let Some(f) = f.take() {
            f();
        }
    });
    // SAFETY: the timer runs on the main run loop it is added to, the thread `f` came from.
    let timer = unsafe { objc2_foundation::NSTimer::timerWithTimeInterval_repeats_block(seconds, false, &block) };
    unsafe { NSRunLoop::mainRunLoop().addTimer_forMode(&timer, NSRunLoopCommonModes) };
    timer
}
