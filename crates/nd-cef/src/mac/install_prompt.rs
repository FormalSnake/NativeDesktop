//! Chromium's "Add <name>?" prompt for a Web Store install the app has already confirmed. The
//! prompt is a Views window no CEF callback is asked about, and no switch or policy skips it for
//! a store install. While an install is armed, the first window of this process to appear that
//! is big enough to be a dialog is taken as the prompt: it is made transparent and click-through,
//! which Views still counts as shown (it enables the accept button only once the dialog has been
//! up for a moment), and its accept button is pressed through the accessibility tree. A prompt
//! still up after [`HAND_BACK`] is shown for the user to answer. The caller disarms once the
//! install call returns.

use std::cell::RefCell;
use std::ptr::NonNull;
use std::time::{Duration, Instant};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObjectProtocol, ProtocolObject, Sel};
use objc2::{MainThreadMarker, Message, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSEvent, NSWindow, NSWindowDidBecomeKeyNotification, NSWindowDidUpdateNotification,
};
use objc2_foundation::{
    NSArray, NSNotification, NSNotificationCenter, NSRect, NSRunLoop, NSRunLoopCommonModes, NSString, NSTimer,
};

/// Views keeps the accept button disabled for its first half second on screen, and ignores a
/// click within a double-click interval of the dialog showing.
const FIRST_PRESS: Duration = Duration::from_millis(1200);
const HAND_BACK: Duration = Duration::from_secs(8);
const MIN_WIDTH: f64 = 200.;
const MIN_HEIGHT: f64 = 80.;

type Observer = Retained<ProtocolObject<dyn NSObjectProtocol>>;

struct Armed {
    until: Instant,
    before: Vec<*const NSWindow>,
    key_before: Option<Retained<NSWindow>>,
    prompt: Option<(Retained<NSWindow>, Instant)>,
    last_press: Option<Instant>,
    timer: Retained<NSTimer>,
    observers: Vec<Observer>,
}

thread_local! {
    static ARMED: RefCell<Option<Armed>> = const { RefCell::new(None) };
}

/// The next prompt within `wait` is answered unseen.
pub(crate) fn arm(wait: Duration) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    disarm();
    let app = NSApplication::sharedApplication(mtm);
    let before = app.windows().iter().map(|w| &*w as *const NSWindow).collect();
    let tick = RcBlock::new(|_timer: NonNull<NSTimer>| step());
    // SAFETY: added to the main run loop, the thread this runs on.
    let timer = unsafe { NSTimer::timerWithTimeInterval_repeats_block(1. / 120., true, &tick) };
    unsafe { NSRunLoop::mainRunLoop().addTimer_forMode(&timer, NSRunLoopCommonModes) };
    // A window becomes key and updates in the run loop turn that orders it in, before it is
    // drawn, so it is taken before its first frame reaches the screen.
    let seen = RcBlock::new(|_note: NonNull<NSNotification>| step());
    let center = NSNotificationCenter::defaultCenter();
    let observers = unsafe {
        [NSWindowDidBecomeKeyNotification, NSWindowDidUpdateNotification]
            .map(|name| center.addObserverForName_object_queue_usingBlock(Some(name), None, None, &seen))
    };
    ARMED.with(|a| {
        *a.borrow_mut() = Some(Armed {
            until: Instant::now() + wait,
            before,
            key_before: app.keyWindow(),
            prompt: None,
            last_press: None,
            timer,
            observers: observers.into(),
        })
    });
    tracing::debug!("nd-cef: install prompt armed");
}

/// Stops watching; a prompt already taken and still up is handed back to the user.
pub(crate) fn disarm() {
    let Some(armed) = ARMED.with(|a| a.borrow_mut().take()) else { return };
    armed.timer.invalidate();
    let center = NSNotificationCenter::defaultCenter();
    for observer in &armed.observers {
        unsafe { center.removeObserver(observer.as_ref()) };
    }
    if let Some((prompt, _)) = armed.prompt {
        if prompt.isVisible() {
            hand_back(&prompt);
        } else if let Some(key) = armed.key_before {
            key.makeKeyWindow();
        }
    }
}

fn hand_back(prompt: &NSWindow) {
    tracing::warn!("nd-cef: Chromium's install prompt stayed up; it is shown for the user to answer");
    prompt.setAlphaValue(1.);
    prompt.setIgnoresMouseEvents(false);
    prompt.makeKeyAndOrderFront(None);
}

fn step() {
    enum Next {
        Wait,
        Press(Retained<NSWindow>),
        Done,
    }
    let next = ARMED.with(|a| {
        // AppKit can post the notifications this runs on from inside the calls it makes.
        let Ok(mut a) = a.try_borrow_mut() else { return Next::Wait };
        let Some(armed) = a.as_mut() else { return Next::Wait };
        let now = Instant::now();
        match &armed.prompt {
            None => {
                if now > armed.until {
                    return Next::Done;
                }
                if let Some(window) = find(armed) {
                    window.setAlphaValue(0.);
                    window.setIgnoresMouseEvents(true);
                    tracing::debug!("nd-cef: install prompt taken, {:?} {:?}", window.frame().size, window.title());
                    armed.prompt = Some((window, now));
                }
                Next::Wait
            }
            Some((window, taken)) => {
                if now.duration_since(*taken) > HAND_BACK {
                    return Next::Done;
                }
                // Views orders the dialog out and back in while it sets up, so a hidden prompt is
                // not an answered one: the install's own result disarms this.
                if !window.isVisible() {
                    return Next::Wait;
                }
                // Views takes a click within a double-click interval of the last one as a repeat,
                // and ignores it and every one after it that comes as fast.
                let every = Duration::from_secs_f64(NSEvent::doubleClickInterval()) + Duration::from_millis(300);
                let due = now.duration_since(*taken) >= FIRST_PRESS
                    && armed.last_press.is_none_or(|t| now.duration_since(t) >= every);
                if due {
                    armed.last_press = Some(now);
                    Next::Press(window.clone())
                } else {
                    Next::Wait
                }
            }
        }
    });
    match next {
        Next::Wait => {}
        Next::Press(window) => press_accept(&window),
        Next::Done => disarm(),
    }
}

/// A window that appeared since the install was armed, big enough to be a dialog.
fn find(armed: &Armed) -> Option<Retained<NSWindow>> {
    let mtm = MainThreadMarker::new()?;
    NSApplication::sharedApplication(mtm).windows().iter().find(|w| {
        let size = w.frame().size;
        !armed.before.contains(&(&**w as *const NSWindow))
            && w.isVisible()
            && size.width >= MIN_WIDTH
            && size.height >= MIN_HEIGHT
    })
}

fn responds(object: &AnyObject, selector: Sel) -> bool {
    unsafe { msg_send![object, respondsToSelector: selector] }
}

/// Every button in `element`'s accessibility subtree.
fn buttons(element: &AnyObject, depth: u32, out: &mut Vec<Retained<AnyObject>>) {
    if depth > 16 {
        return;
    }
    if responds(element, sel!(accessibilityRole)) {
        let role: Option<Retained<NSString>> = unsafe { msg_send![element, accessibilityRole] };
        if role.is_some_and(|r| r.to_string() == "AXButton") {
            out.push(element.retain());
        }
    }
    if !responds(element, sel!(accessibilityChildren)) {
        return;
    }
    let children: Option<Retained<NSArray<AnyObject>>> = unsafe { msg_send![element, accessibilityChildren] };
    for child in children.map(|c| c.to_vec()).unwrap_or_default() {
        buttons(&child, depth + 1, out);
    }
}

/// Views puts a dialog's accept button at the trailing end of its bottom row on macOS.
fn press_accept(window: &NSWindow) {
    let mut found = Vec::new();
    buttons(window, 0, &mut found);
    let frame = |b: &AnyObject| -> NSRect { unsafe { msg_send![b, accessibilityFrame] } };
    let bottom = found.iter().map(|b| frame(b).origin.y).fold(f64::INFINITY, f64::min);
    let accept = found
        .iter()
        .filter(|b| (frame(b).origin.y - bottom).abs() < 4.)
        .max_by(|a, b| frame(a).origin.x.total_cmp(&frame(b).origin.x));
    for b in &found {
        let label: Option<Retained<NSString>> = unsafe { msg_send![&**b, accessibilityLabel] };
        let title: Option<Retained<NSString>> = unsafe { msg_send![&**b, accessibilityTitle] };
        let on: bool = unsafe { msg_send![&**b, isAccessibilityEnabled] };
        tracing::debug!(
            "nd-cef: install prompt button {title:?} {label:?} enabled {on} at {:?}, key {}",
            frame(b),
            window.isKeyWindow()
        );
    }
    let Some(accept) = accept else {
        tracing::debug!("nd-cef: the install prompt shows no buttons yet");
        return;
    };
    let enabled: bool =
        !responds(accept, sel!(isAccessibilityEnabled)) || unsafe { msg_send![&**accept, isAccessibilityEnabled] };
    if enabled && responds(accept, sel!(accessibilityPerformPress)) {
        let pressed: bool = unsafe { msg_send![&**accept, accessibilityPerformPress] };
        tracing::debug!("nd-cef: install prompt accept pressed: {pressed} ({} buttons)", found.len());
    }
}
