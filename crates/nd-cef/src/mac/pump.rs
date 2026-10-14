//! Chromium on the app's run loop: `external_message_pump`, the `CefAppProtocol` Chromium
//! expects of `NSApp`, and shutdown when the app terminates.

use std::cell::Cell;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use block2::RcBlock;
use cef::do_message_loop_work;
use dispatch2::{DispatchQueue, DispatchTime};
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Bool};
use objc2::{ClassType, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSApplication, NSApplicationWillTerminateNotification, NSEvent};
use objc2_foundation::{
    MainThreadMarker, NSNotification, NSNotificationCenter, NSRunLoop, NSRunLoopCommonModes, NSTimer,
};

/// The longest Chromium waits for its work without a scheduled call, while pages are open.
const FALLBACK: f64 = 1. / 30.;

static IMMEDIATE: AtomicBool = AtomicBool::new(false);

thread_local! {
    static PUMPING: Cell<bool> = const { Cell::new(false) };
    static AGAIN: Cell<bool> = const { Cell::new(false) };
    static HANDLING_SEND_EVENT: Cell<bool> = const { Cell::new(false) };
}

/// `on_schedule_message_pump_work`: any thread.
pub(crate) fn schedule(delay_ms: i64) {
    if delay_ms <= 0 {
        if !IMMEDIATE.swap(true, Ordering::AcqRel) {
            DispatchQueue::main().exec_async(|| {
                IMMEDIATE.store(false, Ordering::Release);
                run();
            });
        }
        return;
    }
    if let Ok(when) = DispatchTime::try_from(Duration::from_millis(delay_ms as u64)) {
        let _ = DispatchQueue::main().after(when, run);
    }
}

/// One `do_message_loop_work`, never nested: a call made while Chromium is inside one (a
/// nested run loop under a callback) runs once it returns.
pub(crate) fn run() {
    if PUMPING.with(|p| p.replace(true)) {
        AGAIN.with(|a| a.set(true));
        return;
    }
    do_message_loop_work();
    PUMPING.with(|p| p.set(false));
    if AGAIN.with(|a| a.replace(false)) {
        schedule(0);
    }
}

pub(crate) fn start() {
    let block = RcBlock::new(|_timer: NonNull<NSTimer>| {
        if super::page::live_count() > 0 {
            run();
        }
    });
    // SAFETY: added to the main run loop, the thread this runs on.
    let timer = unsafe { NSTimer::timerWithTimeInterval_repeats_block(FALLBACK, true, &block) };
    unsafe { NSRunLoop::mainRunLoop().addTimer_forMode(&timer, NSRunLoopCommonModes) };
    let terminate = RcBlock::new(|_note: NonNull<NSNotification>| shutdown());
    let center = NSNotificationCenter::defaultCenter();
    let token = unsafe {
        center.addObserverForName_object_queue_usingBlock(
            Some(NSApplicationWillTerminateNotification),
            None,
            None,
            &terminate,
        )
    };
    std::mem::forget(token);
}

/// Closes every browser, lets Chromium finish closing them, then shuts it down.
fn shutdown() {
    super::page::close_all();
    let deadline = Instant::now() + Duration::from_secs(3);
    while super::page::browser_count() > 0 && Instant::now() < deadline {
        do_message_loop_work();
        std::thread::sleep(Duration::from_millis(5));
    }
    cef::shutdown();
}

define_class!(
    // SAFETY: NSApplication may be subclassed; no ivars, since the shared instance is retyped
    // to this class after it was allocated as NSApplication.
    #[unsafe(super(NSApplication))]
    #[thread_kind = MainThreadOnly]
    struct NdCefApplication;

    impl NdCefApplication {
        #[unsafe(method(isHandlingSendEvent))]
        fn is_handling_send_event(&self) -> Bool {
            Bool::new(HANDLING_SEND_EVENT.with(Cell::get))
        }

        #[unsafe(method(setHandlingSendEvent:))]
        fn set_handling_send_event(&self, handling: Bool) {
            HANDLING_SEND_EVENT.with(|h| h.set(handling.as_bool()));
        }

        #[unsafe(method(sendEvent:))]
        fn send_event(&self, event: &NSEvent) {
            let was = HANDLING_SEND_EVENT.with(|h| h.replace(true));
            let _: () = unsafe { msg_send![super(self), sendEvent: event] };
            HANDLING_SEND_EVENT.with(|h| h.set(was));
        }
    }
);

/// Chromium requires `NSApp` to implement `CefAppProtocol` (`isHandlingSendEvent`). nd's app
/// object is a plain `NSApplication`, so it is retyped to a subclass that adds the methods.
pub(crate) fn install_app_protocol() {
    let mtm = MainThreadMarker::new().expect("Chromium starts on the main thread");
    let app = NSApplication::sharedApplication(mtm);
    let object: &AnyObject = &app;
    let current = object.class();
    let ours = NdCefApplication::class();
    if std::ptr::eq(current, ours) {
        return;
    }
    if !std::ptr::eq(current, NSApplication::class()) {
        tracing::warn!(
            "nd-cef: NSApp is a {}, not NSApplication; Chromium may misbehave in nested loops",
            current.name().to_string_lossy()
        );
        return;
    }
    // SAFETY: NdCefApplication adds methods only, no ivars, to NSApplication; retyping the
    // instance keeps its layout.
    unsafe {
        objc2::ffi::object_setClass(Retained::as_ptr(&app).cast_mut().cast(), (ours as *const AnyClass).cast());
        for name in [c"CrAppProtocol", c"CrAppControlProtocol", c"CefAppProtocol"] {
            let protocol = objc2::ffi::objc_getProtocol(name.as_ptr());
            if !protocol.is_null() {
                objc2::ffi::class_addProtocol((ours as *const AnyClass).cast_mut(), protocol);
            }
        }
    }
}
