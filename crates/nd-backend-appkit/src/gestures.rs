//! Gestures on AppKit (ARCHITECTURE.md 6.1). The OS recognizes trackpad gestures itself:
//! `magnifyWithEvent:`, `rotateWithEvent:`, `smartMagnifyWithEvent:` and `pressureChangeWithEvent:`
//! arrive on the view under the pointer and walk up the responder chain to the first view that
//! listens, and page swipes run through `trackSwipeEventWithOptions:`, which follows the user's
//! "Swipe between pages" setting and its spring. Only pointer gestures (tap, double tap, long
//! press, drag) go through `nd-gesture`'s arena, fed from the press's mouse events. Everything
//! reaches the app through the event queue, never from inside the callback.

use std::cell::RefCell;
use std::collections::HashMap;
use std::time::Duration;

use block2::RcBlock;
use nd_core::{NodeId, Point, Rect};
use nd_gesture::{
    ForceClickEvent, GestureEvent, GesturePhase, GestureRouter, GestureSet, Haptic, HitEntry, Output, PinchEvent,
    PointerInput, PointerPhase, RotateEvent, SmartMagnifyEvent, SwipeDirection, SwipePageEvent,
};
use nd_ui::events::{Emit, emit_gesture};
use objc2::Message;
use objc2::rc::Retained;
use objc2::runtime::Bool;
use objc2_app_kit::{
    NSEvent, NSEventPhase, NSEventSwipeTrackingOptions, NSHapticFeedbackManager, NSHapticFeedbackPattern,
    NSHapticFeedbackPerformanceTime, NSHapticFeedbackPerformer, NSView,
};
use objc2_foundation::NSProcessInfo;

use crate::events::Wake;
use crate::views::{NdLayerView, NdRootView};

/// What a layer view listens for and where its events go.
#[derive(Clone)]
pub(crate) struct GestureHost {
    pub set: GestureSet,
    pub emit: Emit,
    pub wake: Wake,
}

impl GestureHost {
    fn deliver(&self, event: GestureEvent) {
        emit_gesture(&self.emit, event);
        self.wake.wake();
    }
}

/// The one pointer press in flight: the arena and the hosts of the nodes under it.
#[derive(Default)]
struct Press {
    router: GestureRouter,
    hosts: HashMap<NodeId, GestureHost>,
    timer: bool,
}

thread_local! {
    static PRESS: RefCell<Press> = RefCell::new(Press::default());
}

fn now() -> f64 {
    NSProcessInfo::processInfo().systemUptime()
}

/// `event`'s location in nd window coordinates: the root node's view, top-left origin.
pub(crate) fn window_point(view: &NSView, event: &NSEvent) -> Point {
    let Some(target) = root_content(view) else {
        let p = view.convertPoint_fromView(event.locationInWindow(), None);
        return Point { x: p.x as f32, y: p.y as f32 };
    };
    let p = target.convertPoint_fromView(event.locationInWindow(), None);
    Point { x: p.x as f32, y: p.y as f32 }
}

fn root_content(view: &NSView) -> Option<Retained<NSView>> {
    let content = view.window()?.contentView()?;
    let root = content.downcast::<NdRootView>().ok()?;
    Some(Retained::into_super(root.content().retain()))
}

fn window_rect(view: &NSView) -> Rect {
    let r = match root_content(view) {
        Some(root) => view.convertRect_toView(view.bounds(), Some(&root)),
        None => view.bounds(),
    };
    Rect::new(r.origin.x as f32, r.origin.y as f32, r.size.width as f32, r.size.height as f32)
}

fn phase(phase: NSEventPhase) -> GesturePhase {
    if phase.contains(NSEventPhase::Began) {
        GesturePhase::Began
    } else if phase.contains(NSEventPhase::Ended) {
        GesturePhase::Ended
    } else if phase.contains(NSEventPhase::Cancelled) {
        GesturePhase::Cancelled
    } else {
        GesturePhase::Changed
    }
}

/// `magnifyWithEvent:`. False passes the event up the responder chain.
pub(crate) fn magnify(view: &NSView, host: Option<&GestureHost>, event: &NSEvent) -> bool {
    let Some(host) = host.filter(|h| h.set.pinch) else { return false };
    host.deliver(GestureEvent::Pinch(PinchEvent {
        position: window_point(view, event),
        scale: 1. + event.magnification() as f32,
        phase: phase(event.phase()),
    }));
    true
}

pub(crate) fn rotate(view: &NSView, host: Option<&GestureHost>, event: &NSEvent) -> bool {
    let Some(host) = host.filter(|h| h.set.rotate) else { return false };
    host.deliver(GestureEvent::Rotate(RotateEvent {
        position: window_point(view, event),
        degrees: event.rotation(),
        phase: phase(event.phase()),
    }));
    true
}

pub(crate) fn smart_magnify(view: &NSView, host: Option<&GestureHost>, event: &NSEvent) -> bool {
    let Some(host) = host.filter(|h| h.set.smart_magnify) else { return false };
    host.deliver(GestureEvent::SmartMagnify(SmartMagnifyEvent { position: window_point(view, event) }));
    true
}

/// `pressureChangeWithEvent:`: the step into stage 2 is a force click.
pub(crate) fn pressure(view: &NSView, host: Option<&GestureHost>, event: &NSEvent, last_stage: &mut isize) -> bool {
    let Some(host) = host.filter(|h| h.set.force_click) else { return false };
    let stage = event.stage();
    if stage == 2 && *last_stage < 2 {
        host.deliver(GestureEvent::ForceClick(ForceClickEvent { position: window_point(view, event) }));
    }
    *last_stage = stage;
    true
}

/// `scrollWheel:` on a view with `.on_swipe_page`: a horizontal two-finger swipe starts AppKit's
/// swipe tracking, which reports the gesture amount (and keeps animating after the fingers lift)
/// until the swipe completes or springs back. Anything else goes up the chain.
pub(crate) fn scroll_wheel(host: Option<&GestureHost>, event: &NSEvent) -> bool {
    let Some(host) = host.filter(|h| h.set.swipe_page) else { return false };
    if event.phase() != NSEventPhase::Began
        || !NSEvent::isSwipeTrackingFromScrollEventsEnabled()
        || event.scrollingDeltaX().abs() <= event.scrollingDeltaY().abs()
    {
        return false;
    }
    let host = host.clone();
    let state = RefCell::new((false, SwipeDirection::Back));
    let handler =
        RcBlock::new(move |amount: f64, ns_phase: NSEventPhase, complete: Bool, _stop: std::ptr::NonNull<Bool>| {
            let (began, direction) = {
                let mut s = state.borrow_mut();
                if amount != 0. {
                    s.1 = if amount > 0. { SwipeDirection::Back } else { SwipeDirection::Forward };
                }
                let began = !std::mem::replace(&mut s.0, true);
                (began, s.1)
            };
            let progress = amount.abs().min(1.) as f32;
            let phase = if began {
                GesturePhase::Began
            } else if complete.as_bool() {
                if progress >= 1. { GesturePhase::Ended } else { GesturePhase::Cancelled }
            } else if ns_phase.contains(NSEventPhase::Cancelled) {
                GesturePhase::Cancelled
            } else {
                GesturePhase::Changed
            };
            host.deliver(GestureEvent::SwipePage(SwipePageEvent { direction, progress, phase }));
        });
    event.trackSwipeEventWithOptions_dampenAmountThresholdMin_max_usingHandler(
        NSEventSwipeTrackingOptions::LockDirection | NSEventSwipeTrackingOptions::ClampGestureAmount,
        -1.,
        1.,
        &handler,
    );
    true
}

/// The press's hit path from `view` outwards: every layer view with pointer gestures.
fn hit_path(view: &NdLayerView) -> Vec<(NodeId, Rect, GestureHost)> {
    let mut path = Vec::new();
    let mut cursor: Option<Retained<NSView>> = Some(Retained::into_super(view.retain()));
    while let Some(v) = cursor {
        if let Some(layer) = v.downcast_ref::<NdLayerView>()
            && let Some(host) = layer.gesture_host()
            && host.set.has_pointer()
        {
            path.push((host.emit.node(), window_rect(&v), host));
        }
        // SAFETY: called on the main thread on a view in a live window; `superview` is a weak
        // property, and the returned view is retained before use.
        cursor = unsafe { v.superview() };
    }
    path
}

/// A primary mouse event on `view`. True when a pointer recognizer is listening, so the view
/// keeps the event; false lets AppKit carry on (window drags, native controls).
pub(crate) fn mouse(view: &NdLayerView, event: &NSEvent, pointer_phase: PointerPhase) -> bool {
    let input = PointerInput::new(pointer_phase, window_point(view, event), event.timestamp());
    let outputs = PRESS.with_borrow_mut(|press| {
        if pointer_phase == PointerPhase::Down {
            let path = hit_path(view);
            if path.is_empty() {
                // A held double-tap wait expires on its timer; this press is AppKit's.
                return None;
            }
            let sets: Vec<GestureSet> = path.iter().map(|(_, _, h)| h.set).collect();
            let hit: Vec<HitEntry<'_>> = path
                .iter()
                .zip(&sets)
                .map(|((node, bounds, _), set)| HitEntry { node: *node, gestures: set, bounds: *bounds })
                .collect();
            let outputs = press.router.pointer(&input, &hit);
            press.hosts.extend(path.into_iter().map(|(node, _, host)| (node, host)));
            Some(outputs)
        } else if press_active(press) {
            Some(press.router.pointer(&input, &[]))
        } else {
            None
        }
    });
    let Some(outputs) = outputs else { return false };
    apply(outputs);
    arm_timer();
    true
}

fn press_active(press: &Press) -> bool {
    press.router.pointer_active()
}

fn apply(outputs: Vec<Output>) {
    let hosts = PRESS.with_borrow(|p| p.hosts.clone());
    for output in outputs {
        match output {
            Output::Event(node, event) => {
                if let Some(host) = hosts.get(&node) {
                    host.deliver(event);
                }
            }
            Output::Haptic(h) => play(h),
            Output::Scroll { .. } => {}
        }
    }
    PRESS.with_borrow_mut(|p| {
        if !p.router.pointer_active() {
            p.hosts.clear();
        }
    });
}

/// Schedules the arena's next timer (long press, double-tap wait) on the main queue.
fn arm_timer() {
    let deadline = PRESS.with_borrow_mut(|p| {
        let deadline = p.router.next_deadline()?;
        (!std::mem::replace(&mut p.timer, true)).then_some(deadline)
    });
    let Some(deadline) = deadline else { return };
    let delay = Duration::from_secs_f64((deadline - now()).max(0.) + 0.001);
    let Ok(when) = dispatch2::DispatchTime::try_from(delay) else { return };
    let _ = dispatch2::DispatchQueue::main().after(when, || {
        let outputs = PRESS.with_borrow_mut(|p| {
            p.timer = false;
            p.router.step(now())
        });
        apply(outputs);
        arm_timer();
    });
}

/// Plays `haptic` when the next frame is drawn, so the tick lands with the snap it marks.
pub(crate) fn play(haptic: Haptic) {
    let pattern = match haptic {
        Haptic::Alignment => NSHapticFeedbackPattern::Alignment,
        Haptic::LevelChange => NSHapticFeedbackPattern::LevelChange,
        Haptic::Generic => NSHapticFeedbackPattern::Generic,
    };
    NSHapticFeedbackManager::defaultPerformer()
        .performFeedbackPattern_performanceTime(pattern, NSHapticFeedbackPerformanceTime::DrawCompleted);
}
