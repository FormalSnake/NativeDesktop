//! The per-backend native event queue (ARCHITECTURE.md 3.13 rule 1). Native callbacks only push
//! here; the queue's waker runs on a later main run loop turn, outside the callback and outside
//! any transaction, and that is where app code sees the events.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;

use block2::RcBlock;
use nd_core::NodeId;
use objc2_foundation::{NSArray, NSRunLoop, NSRunLoopCommonModes};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeEventKind {
    /// A native button's action fired.
    Click,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeEvent {
    pub node: NodeId,
    pub kind: NativeEventKind,
}

#[derive(Default)]
struct Inner {
    events: RefCell<VecDeque<NativeEvent>>,
    waker: RefCell<Option<Rc<dyn Fn()>>>,
    scheduled: Cell<bool>,
    programmatic: Cell<bool>,
}

#[derive(Clone, Default)]
pub struct EventQueue(Rc<Inner>);

impl EventQueue {
    /// Called once per main run loop turn that has events waiting.
    pub fn set_waker(&self, waker: impl Fn() + 'static) {
        *self.0.waker.borrow_mut() = Some(Rc::new(waker));
    }

    pub fn drain(&self) -> Vec<NativeEvent> {
        self.0.events.borrow_mut().drain(..).collect()
    }

    pub(crate) fn push(&self, event: NativeEvent) {
        // An event fired by a setter nd itself ran is an echo of nd's own write.
        if self.0.programmatic.get() {
            return;
        }
        self.0.events.borrow_mut().push_back(event);
        if self.0.scheduled.replace(true) {
            return;
        }
        let inner = Rc::downgrade(&self.0);
        let block = RcBlock::new(move || {
            let Some(inner) = inner.upgrade() else { return };
            inner.scheduled.set(false);
            let waker = inner.waker.borrow().clone();
            if let Some(waker) = waker {
                waker();
            }
        });
        // Common modes, so a click inside a nested tracking loop is still delivered.
        let modes = NSArray::from_slice(&[unsafe { NSRunLoopCommonModes }]);
        unsafe { NSRunLoop::mainRunLoop().performInModes_block(&modes, &block) };
    }

    pub(crate) fn set_programmatic(&self, on: bool) {
        self.0.programmatic.set(on);
    }
}
