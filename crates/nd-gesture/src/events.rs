//! What apps receive. Each type is delivered to the node's handler through the event queue, never
//! from inside a native callback.

use nd_core::Point;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GesturePhase {
    Began,
    Changed,
    Ended,
    /// The gesture stopped without completing: another recognizer won, the OS took the pointer,
    /// or a swipe fell short. Undo whatever the earlier phases previewed.
    Cancelled,
}

/// A press released in place. `count` is 2 for the second press of a double tap.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TapEvent {
    pub position: Point,
    pub count: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LongPressEvent {
    pub position: Point,
    pub phase: GesturePhase,
}

/// A pointer drag (`.on_drag`). `translation` is from the press point; `velocity` is in points
/// per second, set on `Ended` for a fling.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DragEvent {
    pub phase: GesturePhase,
    pub start: Point,
    pub position: Point,
    pub translation: Point,
    pub velocity: Point,
}

/// A trackpad pinch. `scale` multiplies: `zoom *= event.scale`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PinchEvent {
    pub position: Point,
    pub scale: f32,
    pub phase: GesturePhase,
}

/// A trackpad rotation, macOS only. `degrees` is this event's change, counterclockwise positive.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RotateEvent {
    pub position: Point,
    pub degrees: f32,
    pub phase: GesturePhase,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwipeDirection {
    /// Fingers moved right (left in right-to-left layouts is the backend's to flip).
    Back,
    Forward,
}

/// A two-finger horizontal page swipe (`.on_swipe_page`). `progress` runs 0 to 1 while the
/// fingers track; `Ended` means the swipe committed, `Cancelled` that it fell back.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SwipePageEvent {
    pub direction: SwipeDirection,
    pub progress: f32,
    pub phase: GesturePhase,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RowEdge {
    Leading,
    Trailing,
}

/// A row's swipe-to-reveal state. `offset` is the row content's horizontal displacement
/// (positive reveals the leading edge). `open` is the edge left revealed once the row settles;
/// `full` means a full swipe committed the edge's first action.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RowSwipeEvent {
    pub offset: f32,
    pub phase: GesturePhase,
    pub open: Option<RowEdge>,
    pub full: bool,
}

/// A deep press on a Force Touch trackpad, macOS only.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ForceClickEvent {
    pub position: Point,
}

/// Two-finger double tap, macOS only.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SmartMagnifyEvent {
    pub position: Point,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GestureEvent {
    Tap(TapEvent),
    LongPress(LongPressEvent),
    Drag(DragEvent),
    Pinch(PinchEvent),
    Rotate(RotateEvent),
    SwipePage(SwipePageEvent),
    RowSwipe(RowSwipeEvent),
    ForceClick(ForceClickEvent),
    SmartMagnify(SmartMagnifyEvent),
}
