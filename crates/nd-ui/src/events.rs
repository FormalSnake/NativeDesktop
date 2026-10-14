//! Native callbacks and controlled values (ARCHITECTURE.md 3.13).
//!
//! A backend never calls app code. Native callbacks push onto the [`EventQueue`] through an
//! [`Emit`], and `nd-ui` drains the queue after the transaction commits and after any nested
//! run loop returns, outside every entity lease.

use std::any::Any;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use nd_core::{NodeId, WindowId};

use crate::{App, Window};

/// One native event, addressed to the retained node that produced it.
pub struct NativeEvent {
    pub node: NodeId,
    pub event: Box<dyn Any>,
}

/// A delivery queued for a window, for a widget drawn from elements: it has no retained node of
/// its own to address, so it queues the call to its handlers instead.
pub type Delivery = Box<dyn FnOnce(&mut Window, &mut App)>;

pub(crate) enum Queued {
    Event(NativeEvent),
    Call(WindowId, Delivery),
}

#[derive(Default)]
struct Queue {
    events: VecDeque<Queued>,
    programmatic: u32,
}

/// The app-wide queue of native events. Cheap to clone; backends keep one.
#[derive(Clone, Default)]
pub struct EventQueue(Rc<RefCell<Queue>>);

impl EventQueue {
    pub fn emitter(&self, node: NodeId) -> Emit {
        Emit { queue: self.clone(), node }
    }

    /// Runs `f`, dropping every event emitted meanwhile. Backends wrap programmatic setters in
    /// it, so a delegate the setter fires synchronously never reaches the app.
    pub fn programmatic<R>(&self, f: impl FnOnce() -> R) -> R {
        self.0.borrow_mut().programmatic += 1;
        let result = f();
        self.0.borrow_mut().programmatic -= 1;
        result
    }

    pub fn is_programmatic(&self) -> bool {
        self.0.borrow().programmatic > 0
    }

    pub fn push(&self, node: NodeId, event: Box<dyn Any>) {
        let mut queue = self.0.borrow_mut();
        if queue.programmatic == 0 {
            queue.events.push_back(Queued::Event(NativeEvent { node, event }));
        }
    }

    /// Queues `call` to run with `window` when the queue drains, like a native event.
    pub fn push_call(&self, window: WindowId, call: Delivery) {
        let mut queue = self.0.borrow_mut();
        if queue.programmatic == 0 {
            queue.events.push_back(Queued::Call(window, call));
        }
    }

    pub(crate) fn pop(&self) -> Option<Queued> {
        self.0.borrow_mut().events.pop_front()
    }

    pub fn is_empty(&self) -> bool {
        self.0.borrow().events.is_empty()
    }
}

/// Emits events for one node. Queues, never calls app code.
#[derive(Clone)]
pub struct Emit {
    queue: EventQueue,
    node: NodeId,
}

impl Emit {
    pub fn emit<E: 'static>(&self, event: E) {
        self.queue.push(self.node, Box::new(event));
    }

    /// Emits an event already boxed, such as a [`Response`](crate::semantics::Response)'s.
    pub fn emit_boxed(&self, event: Box<dyn Any>) {
        self.queue.push(self.node, event);
    }

    pub fn node(&self) -> NodeId {
        self.node
    }

    pub fn programmatic<R>(&self, f: impl FnOnce() -> R) -> R {
        self.queue.programmatic(f)
    }
}

/// A controlled prop's native side, kept in the widget's backend state. It tracks the value the
/// native control holds as far as nd knows: the last value applied or reported. An app that
/// feeds a reported or normalised value back therefore causes no second set.
#[derive(Clone, Debug)]
pub struct Synced<T> {
    native: Option<T>,
}

impl<T> Default for Synced<T> {
    fn default() -> Self {
        Self { native: None }
    }
}

impl<T: Clone + PartialEq> Synced<T> {
    /// Calls `set` when `value` differs from what the control holds. `set` runs the setter
    /// (inside [`Emit::programmatic`]) and returns the value the control holds afterwards.
    /// Returns that value when the control normalised it, for the widget to emit as a change.
    pub fn apply(&mut self, value: &T, set: impl FnOnce(&T) -> T) -> Option<T> {
        if self.native.as_ref() == Some(value) {
            return None;
        }
        let actual = set(value);
        let normalised = (actual != *value).then(|| actual.clone());
        self.native = Some(actual);
        normalised
    }

    /// Records a value the control produced itself (user input). The widget emits it.
    pub fn report(&mut self, value: T) {
        self.native = Some(value);
    }

    pub fn value(&self) -> Option<&T> {
        self.native.as_ref()
    }
}

/// Queues a recognized gesture for its node as the concrete event type its handler takes
/// (`.on_pinch` takes [`nd_gesture::PinchEvent`], and so on).
pub fn emit_gesture(emit: &Emit, event: nd_gesture::GestureEvent) {
    use nd_gesture::GestureEvent as G;
    match event {
        G::Tap(e) if e.count >= 2 => emit.emit(nd_gesture::DoubleTapEvent { position: e.position }),
        G::Tap(e) => emit.emit(e),
        G::LongPress(e) => emit.emit(e),
        G::Drag(e) => emit.emit(e),
        G::Pinch(e) => emit.emit(e),
        G::Rotate(e) => emit.emit(e),
        G::SwipePage(e) => emit.emit(e),
        G::RowSwipe(e) => emit.emit(e),
        G::ForceClick(e) => emit.emit(e),
        G::SmartMagnify(e) => emit.emit(e),
    }
}
