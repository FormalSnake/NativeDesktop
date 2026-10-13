//! Gesture recognition, scroll physics and haptic vocabulary (ARCHITECTURE.md 6.1). Pure logic:
//! no GPUI, no OS. Time is seconds on the window's frame clock, positions are logical points in
//! window coordinates.
//!
//! - [`Arena`] arbitrates pointer gestures (tap, double tap, long press, drag) between the nested
//!   recognizers under a press: the first to claim wins, ties go to the innermost.
//! - [`ScrollArbiter`] does the same for two-finger scrolls: after a direction lock the innermost
//!   consumer of that axis wins (a scroller with room, a row swipe, a page swipe), so a row swipe
//!   inside a vertical list never fights the list.
//! - [`Scroller`] is momentum and rubber banding for backends that get no OS momentum (Wayland,
//!   zed#14473) or must own the spring (GPUI ignores `momentumPhase`): exponential decay at
//!   Apple's 0.998 per millisecond, rubber band `x*d*c/(d+c*x)` with `c = 0.55`, a critically
//!   damped spring back. Everything advances in [`Scroller::step`] on the frame clock, and the
//!   decay is evaluated in closed form, so the path is the same at 60 Hz and 240 Hz.
//! - [`GestureRouter`] is what a backend holds per window: it takes raw input plus the hit path
//!   and returns events to queue, scroll offsets to apply and haptics to play.
//!
//! On macOS the AppKit backend uses the OS's own recognition for pinch, rotate, smart zoom, force
//! click and page swipes (`trackSwipeEvent`) and `NSScrollView` for scrolling; only the pointer
//! arena is shared.

mod arena;
mod events;
mod haptic;
mod input;
mod recognizer;
mod router;
mod scroll;
mod set;
mod spring;
mod swipe;
mod velocity;

pub use arena::{Arena, ArenaMember};
pub use events::{
    DragEvent, ForceClickEvent, GestureEvent, GesturePhase, LongPressEvent, PinchEvent, RotateEvent, RowEdge,
    RowSwipeEvent, SmartMagnifyEvent, SwipeDirection, SwipePageEvent, TapEvent,
};
pub use haptic::Haptic;
pub use input::{MomentumInference, MomentumPhase, PointerInput, PointerPhase, ScrollInput, ScrollPhase};
pub use recognizer::{GestureKind, LongPress, Pan, Recognizer, Tap, Verdict};
pub use router::{GestureRouter, HitEntry, Output, ScrollArbiter};
pub use scroll::{DECELERATION_RATE, Scroller, project, rubber_band};
pub use set::{Axes, Axis, GestureConfig, GestureSet, RowSwipeConfig};
pub use spring::{Spring, SpringMotion};
pub use swipe::{PageSwipe, RowSwipe};
pub use velocity::VelocityTracker;
