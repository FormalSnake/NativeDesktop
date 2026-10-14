//! Real input through the HID event tap: the window server routes these events exactly like a
//! physical mouse and keyboard, so AppKit's tracking loops (live resize, window drags, menus)
//! believe them. Coordinates are global points with a top-left origin.
//!
//! A keyboard event posted with modifier flags sets the HID system's modifier state, and nothing
//! clears it until a matching flags-changed event arrives. A chord posted as one flagged key event
//! leaves Command and Control "held" machine-wide; every later synthetic click then becomes a
//! control-click (a secondary click) and every drag a Control-Command window move. So chords here
//! press and release each modifier as its own flags-changed event, and mouse events carry
//! explicitly empty flags rather than the session's current ones.

use std::time::Duration;

use objc2_core_foundation::{CFBoolean, CFRetained, CFString, CFType, CGPoint};
use objc2_core_graphics::{
    CGEvent, CGEventField, CGEventFlags, CGEventSource, CGEventSourceStateID, CGEventTapLocation, CGEventType,
    CGMouseButton, CGScrollEventUnit,
};

/// A modifier key: its flag and the virtual key code of its left-hand key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Modifier {
    Command,
    Control,
    Option,
    Shift,
}

impl Modifier {
    pub const ALL: [Modifier; 4] = [Modifier::Command, Modifier::Control, Modifier::Option, Modifier::Shift];

    pub fn flag(self) -> CGEventFlags {
        match self {
            Modifier::Command => CGEventFlags::MaskCommand,
            Modifier::Control => CGEventFlags::MaskControl,
            Modifier::Option => CGEventFlags::MaskAlternate,
            Modifier::Shift => CGEventFlags::MaskShift,
        }
    }

    fn key_code(self) -> u16 {
        match self {
            Modifier::Command => 0x37,
            Modifier::Control => 0x3B,
            Modifier::Option => 0x3A,
            Modifier::Shift => 0x38,
        }
    }
}

fn post(event: Option<CFRetained<CGEvent>>) {
    CGEvent::post(CGEventTapLocation::HIDEventTap, event.as_deref());
}

fn flags_changed(modifier: Modifier, flags: CGEventFlags) {
    let event = CGEvent::new_keyboard_event(None, modifier.key_code(), flags.contains(modifier.flag()));
    CGEvent::set_type(event.as_deref(), CGEventType::FlagsChanged);
    CGEvent::set_flags(event.as_deref(), flags);
    post(event);
    std::thread::sleep(Duration::from_millis(10));
}

/// The modifiers the HID system currently believes are down.
pub fn held_modifiers() -> Vec<Modifier> {
    let state = CGEventSource::flags_state(CGEventSourceStateID::HIDSystemState);
    Modifier::ALL.into_iter().filter(|m| state.contains(m.flag())).collect()
}

/// Posts a release for every modifier the HID system believes is down.
pub fn release_modifiers() {
    let mut flags = CGEventSource::flags_state(CGEventSourceStateID::HIDSystemState);
    for modifier in held_modifiers() {
        flags.remove(modifier.flag());
        flags_changed(modifier, flags);
    }
}

/// One mouse event at `at`, with no modifiers.
pub fn mouse(kind: CGEventType, at: (f64, f64)) {
    let event = CGEvent::new_mouse_event(None, kind, CGPoint { x: at.0, y: at.1 }, CGMouseButton::Left);
    CGEvent::set_flags(event.as_deref(), CGEventFlags::empty());
    if matches!(kind, CGEventType::LeftMouseDown | CGEventType::LeftMouseUp) {
        CGEvent::set_integer_value_field(event.as_deref(), CGEventField::MouseEventClickState, 1);
    }
    post(event);
}

pub fn move_to(at: (f64, f64)) {
    mouse(CGEventType::MouseMoved, at);
}

/// Moves to `at`, then presses and releases the left button there.
pub fn click(at: (f64, f64)) {
    move_to(at);
    std::thread::sleep(Duration::from_millis(80));
    mouse(CGEventType::LeftMouseDown, at);
    std::thread::sleep(Duration::from_millis(40));
    mouse(CGEventType::LeftMouseUp, at);
    std::thread::sleep(Duration::from_millis(150));
}

/// Moves to `at`, then presses and releases the right button there.
pub fn right_click(at: (f64, f64)) {
    move_to(at);
    std::thread::sleep(Duration::from_millis(80));
    for kind in [CGEventType::RightMouseDown, CGEventType::RightMouseUp] {
        let event = CGEvent::new_mouse_event(None, kind, CGPoint { x: at.0, y: at.1 }, CGMouseButton::Right);
        CGEvent::set_flags(event.as_deref(), CGEventFlags::empty());
        CGEvent::set_integer_value_field(event.as_deref(), CGEventField::MouseEventClickState, 1);
        post(event);
        std::thread::sleep(Duration::from_millis(40));
    }
    std::thread::sleep(Duration::from_millis(150));
}

/// Scrolls `dy` pixels (positive moves content down) with the pointer at `at`, in `steps` events.
pub fn scroll(at: (f64, f64), dy: i32, steps: u32) {
    move_to(at);
    std::thread::sleep(Duration::from_millis(60));
    for _ in 0..steps.max(1) {
        let event = CGEvent::new_scroll_wheel_event2(None, CGScrollEventUnit::Pixel, 1, dy / steps.max(1) as i32, 0, 0);
        CGEvent::set_location(event.as_deref(), CGPoint { x: at.0, y: at.1 });
        CGEvent::set_flags(event.as_deref(), CGEventFlags::empty());
        post(event);
        std::thread::sleep(Duration::from_millis(16));
    }
    std::thread::sleep(Duration::from_millis(120));
}

/// Presses at `from`, drags by `(dx, dy)` in `steps` moves at about 80 Hz, releases. The gaps
/// matter: AppKit coalesces a drag posted without them into its end point.
pub fn drag(from: (f64, f64), dx: f64, dy: f64, steps: u32) {
    move_to(from);
    std::thread::sleep(Duration::from_millis(150));
    mouse(CGEventType::LeftMouseDown, from);
    std::thread::sleep(Duration::from_millis(60));
    for i in 1..=steps {
        let t = f64::from(i) / f64::from(steps);
        mouse(CGEventType::LeftMouseDragged, (from.0 + dx * t, from.1 + dy * t));
        std::thread::sleep(Duration::from_millis(12));
    }
    std::thread::sleep(Duration::from_millis(60));
    mouse(CGEventType::LeftMouseUp, (from.0 + dx, from.1 + dy));
    std::thread::sleep(Duration::from_millis(300));
}

/// Presses and releases the key `code` with `modifiers` held, each modifier going down and up as
/// its own flags-changed event so the HID state ends where it started.
pub fn key(code: u16, modifiers: &[Modifier]) {
    let mut flags = CGEventFlags::empty();
    for &modifier in modifiers {
        flags.insert(modifier.flag());
        flags_changed(modifier, flags);
    }
    for down in [true, false] {
        let event = CGEvent::new_keyboard_event(None, code, down);
        CGEvent::set_flags(event.as_deref(), flags);
        post(event);
        std::thread::sleep(Duration::from_millis(30));
    }
    for &modifier in modifiers.iter().rev() {
        flags.remove(modifier.flag());
        flags_changed(modifier, flags);
    }
}

/// A gesture's phase as the window server encodes it in gesture and scroll events.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GesturePhase {
    Began,
    Changed,
    Ended,
}

impl GesturePhase {
    /// `CGScrollPhase` and the gesture phase field share these values.
    fn value(self) -> i64 {
        match self {
            GesturePhase::Began => 1,
            GesturePhase::Changed => 2,
            GesturePhase::Ended => 4,
        }
    }
}

/// The window server's gesture event type and the private fields AppKit reads to build a
/// `.magnify` NSEvent. CoreGraphics has no public constructor for gesture events; this layout is
/// undocumented and checked by the M4 gate on each macOS release.
const GESTURE_EVENT: u32 = 29;
const GESTURE_SUBTYPE: u32 = 110;
const GESTURE_SUBTYPE_MAGNIFY: i64 = 8;
const GESTURE_MAGNIFICATION: u32 = 113;
const GESTURE_PHASE: u32 = 132;

/// One trackpad pinch event at `at`: `amount` is the magnification delta (0.1 zooms in 10%).
pub fn magnify_event(at: (f64, f64), amount: f64, phase: GesturePhase) -> Option<CFRetained<CGEvent>> {
    let event = CGEvent::new(None)?;
    CGEvent::set_type(Some(&event), CGEventType(GESTURE_EVENT));
    CGEvent::set_location(Some(&event), CGPoint { x: at.0, y: at.1 });
    CGEvent::set_integer_value_field(Some(&event), CGEventField(GESTURE_SUBTYPE), GESTURE_SUBTYPE_MAGNIFY);
    CGEvent::set_double_value_field(Some(&event), CGEventField(GESTURE_MAGNIFICATION), amount);
    CGEvent::set_integer_value_field(Some(&event), CGEventField(GESTURE_PHASE), phase.value());
    Some(event)
}

/// One continuous (trackpad) scroll event at `at`, in pixels, with its finger phase. Positive
/// `dx` is the fingers moving right under natural scrolling.
pub fn scroll_event(at: (f64, f64), dx: i32, dy: i32, phase: GesturePhase) -> Option<CFRetained<CGEvent>> {
    let event = CGEvent::new_scroll_wheel_event2(None, CGScrollEventUnit::Pixel, 2, dy, dx, 0)?;
    CGEvent::set_location(Some(&event), CGPoint { x: at.0, y: at.1 });
    CGEvent::set_flags(Some(&event), CGEventFlags::empty());
    CGEvent::set_integer_value_field(Some(&event), CGEventField::ScrollWheelEventIsContinuous, 1);
    CGEvent::set_integer_value_field(Some(&event), CGEventField::ScrollWheelEventScrollPhase, phase.value());
    Some(event)
}

/// A pinch at `at` through the HID tap: began, `steps` changes of `amount` each, ended, at about
/// 60 Hz.
pub fn pinch(at: (f64, f64), amount: f64, steps: u32) {
    move_to(at);
    std::thread::sleep(Duration::from_millis(100));
    post(magnify_event(at, 0., GesturePhase::Began));
    for _ in 0..steps {
        std::thread::sleep(Duration::from_millis(16));
        post(magnify_event(at, amount, GesturePhase::Changed));
    }
    std::thread::sleep(Duration::from_millis(16));
    post(magnify_event(at, 0., GesturePhase::Ended));
    std::thread::sleep(Duration::from_millis(200));
}

/// A two-finger horizontal swipe at `at` through the HID tap: `steps` scroll events of `dx`
/// pixels each between a began and an ended phase.
pub fn two_finger_swipe(at: (f64, f64), dx: i32, steps: u32) {
    move_to(at);
    std::thread::sleep(Duration::from_millis(100));
    post(scroll_event(at, dx, 0, GesturePhase::Began));
    for _ in 0..steps {
        std::thread::sleep(Duration::from_millis(12));
        post(scroll_event(at, dx, 0, GesturePhase::Changed));
    }
    std::thread::sleep(Duration::from_millis(12));
    post(scroll_event(at, 0, 0, GesturePhase::Ended));
    std::thread::sleep(Duration::from_millis(400));
}

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXUIElementCreateApplication(pid: i32) -> *mut std::ffi::c_void;
    fn AXUIElementSetAttributeValue(element: *mut std::ffi::c_void, attribute: &CFString, value: &CFType) -> i32;
}

/// Makes the app `pid` active, as a click on it would. Activation is cooperative, so an app
/// launched in the background never takes it on its own and key events would go elsewhere.
pub fn activate(pid: u32) -> bool {
    let attribute = CFString::from_static_str("AXFrontmost");
    let status = unsafe {
        let app = AXUIElementCreateApplication(pid as i32);
        let Some(app) = std::ptr::NonNull::new(app) else { return false };
        let status = AXUIElementSetAttributeValue(app.as_ptr(), &attribute, CFBoolean::new(true));
        // The create rule: this call owns the element.
        drop(CFRetained::from_raw(app.cast::<CFType>()));
        status
    };
    status == 0
}
