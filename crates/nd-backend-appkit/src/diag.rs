//! The AppKit side of `nd-diag`: input recording, the native view and layer hierarchy for
//! `tree.json`, the hang watchdog's main queue, and debug triggers for the incident gates.
//!
//! After the first frame, `ND_DEBUG_PANIC=1` sends the window three synthetic inputs and then
//! panics, `ND_DEBUG_HANG_MS=<ms>` blocks the main thread that long, and `ND_DEBUG_CRASH=1`
//! waits for the crash snapshot and writes through an unmapped address.

use dispatch2::DispatchQueue;
use nd_diag::{InputRecord, PostToMain};
use objc2::rc::Retained;
use objc2_app_kit::{NSApplication, NSEvent, NSEventModifierFlags, NSEventType, NSView};
use objc2_foundation::{MainThreadMarker, NSPoint, NSString};
use serde_json::{Value, json};

use crate::NdWindow;

pub(crate) fn post_to_main() -> PostToMain {
    Box::new(|f| DispatchQueue::main().exec_async(f))
}

fn kind(t: NSEventType) -> Option<&'static str> {
    Some(match t {
        NSEventType::KeyDown => "key_down",
        NSEventType::KeyUp => "key_up",
        NSEventType::FlagsChanged => "modifiers",
        NSEventType::LeftMouseDown => "mouse_down",
        NSEventType::LeftMouseUp => "mouse_up",
        NSEventType::RightMouseDown => "right_mouse_down",
        NSEventType::RightMouseUp => "right_mouse_up",
        NSEventType::OtherMouseDown => "other_mouse_down",
        NSEventType::OtherMouseUp => "other_mouse_up",
        NSEventType::ScrollWheel => "scroll",
        NSEventType::Magnify => "magnify",
        NSEventType::Rotate => "rotate",
        NSEventType::Swipe => "swipe",
        _ => return None,
    })
}

/// Records `event` if it is user input. Pointer moves are left out; they would flush the ring.
pub(crate) fn record_event(window: &NdWindow, event: &NSEvent) {
    let Some(diag) = nd_diag::global() else { return };
    let t = event.r#type();
    let Some(kind) = kind(t) else { return };
    let key = matches!(t, NSEventType::KeyDown | NSEventType::KeyUp);
    let position = (!key && t != NSEventType::FlagsChanged).then(|| {
        let p = event.locationInWindow();
        let height = window.contentView().map_or(0., |v| v.frame().size.height);
        [p.x, height - p.y]
    });
    diag.record_input(InputRecord {
        t: event.timestamp(),
        window: window.windowNumber() as i64,
        kind: kind.into(),
        position,
        key_code: key.then(|| event.keyCode()),
        text: if key { event.characters().map(|c| c.to_string()) } else { None },
        modifiers: event.modifierFlags().0 as u64,
    });
}

fn view_json(view: &NSView) -> Value {
    let f = view.frame();
    let mut v = json!({
        "class": view.class().name().to_string_lossy(),
        "frame": [f.origin.x, f.origin.y, f.size.width, f.size.height],
    });
    let map = v.as_object_mut().expect("object");
    if view.isHidden() {
        map.insert("hidden".into(), true.into());
    }
    if let Some(layer) = view.layer() {
        // SAFETY: read on the main thread, the only one that mutates nd's layers.
        let sublayers = unsafe { layer.sublayers() }.map_or(0, |s| s.count());
        map.insert("layer".into(), json!({ "class": layer.class().name().to_string_lossy(), "sublayers": sublayers }));
    }
    let subviews = view.subviews();
    if !subviews.is_empty() {
        map.insert("subviews".into(), subviews.iter().map(|s| view_json(&s)).collect());
    }
    v
}

/// Every window's content view hierarchy, with z-order as AppKit keeps it (later is on top).
pub(crate) fn native_tree(mtm: MainThreadMarker) -> Value {
    let windows: Vec<Value> = NSApplication::sharedApplication(mtm)
        .windows()
        .iter()
        .map(|w| {
            let f = w.frame();
            json!({
                "number": w.windowNumber(),
                "title": w.title().to_string(),
                "frame": [f.origin.x, f.origin.y, f.size.width, f.size.height],
                "visible": w.isVisible(),
                "key": w.isKeyWindow(),
                "content": w.contentView().map(|v| view_json(&v)),
            })
        })
        .collect();
    json!({ "windows": windows })
}

fn env_flag(name: &str) -> bool {
    std::env::var_os(name).is_some_and(|v| v == "1")
}

fn synthetic_inputs(window: &NdWindow) -> Vec<Retained<NSEvent>> {
    let number = window.windowNumber();
    let at = NSPoint::new(24., 24.);
    let mouse = |t| {
        NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
            t,
            at,
            NSEventModifierFlags::empty(),
            0.,
            number,
            None,
            0,
            1,
            1.,
        )
    };
    let n = NSString::from_str("n");
    // A key-up, since an unhandled key-down beeps.
    let key = NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
        NSEventType::KeyUp,
        at,
        NSEventModifierFlags::empty(),
        0.,
        number,
        None,
        &n,
        &n,
        false,
        45,
    );
    [mouse(NSEventType::LeftMouseDown), mouse(NSEventType::LeftMouseUp), key].into_iter().flatten().collect()
}

/// Runs the debug trigger the environment asks for, once the first frame is up.
pub(crate) fn after_first_frame(window: &NdWindow) {
    if env_flag("ND_DEBUG_PANIC") {
        let events = synthetic_inputs(window);
        let window = objc2::Message::retain(window);
        crate::events::defer(move || {
            for event in &events {
                window.sendEvent(event);
            }
            crate::events::defer(|| panic!("ND_DEBUG_PANIC: forced panic after the first frame"));
        });
    }
    if let Some(ms) = std::env::var("ND_DEBUG_HANG_MS").ok().and_then(|v| v.parse().ok()) {
        crate::events::defer(move || std::thread::sleep(std::time::Duration::from_millis(ms)));
    }
    if env_flag("ND_DEBUG_CRASH") {
        crate::events::defer(|| {
            // The crash helper reads the snapshot written about once a second.
            std::thread::sleep(std::time::Duration::from_millis(2500));
            // SAFETY: none; the page at 16 is never mapped and the fault is the point. Null would
            // trip the debug precondition check and panic instead.
            unsafe { (16 as *mut u8).write_volatile(1) };
        });
    }
}
