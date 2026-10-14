use std::time::{Duration, Instant};

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate, NSEventMask, NSEventModifierFlags, NSMenu,
    NSMenuItem, NSWindowStyleMask,
};
use objc2_foundation::{
    MainThreadMarker, NSDate, NSDefaultRunLoopMode, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString,
};

use crate::NdWindow;

define_class!(
    // SAFETY: NSObject has no subclassing requirements and this type does not implement Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    struct NdAppDelegate;

    unsafe impl NSObjectProtocol for NdAppDelegate {}

    unsafe impl NSApplicationDelegate for NdAppDelegate {
        #[unsafe(method(applicationShouldTerminateAfterLastWindowClosed:))]
        fn should_terminate_after_last_window_closed(&self, _app: &NSApplication) -> bool {
            true
        }

        // Light-dismiss overlays close when the app loses activation (ARCHITECTURE.md 6.3).
        #[unsafe(method(applicationDidResignActive:))]
        fn did_resign_active(&self, _notification: &objc2_foundation::NSNotification) {
            crate::defer_with_app(|app| app.app_deactivated());
        }
    }
);

impl NdAppDelegate {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        unsafe { msg_send![Self::alloc(mtm), init] }
    }
}

/// nd's owner of `NSApplication`: activation policy, delegate, main menu, windows and the run
/// loop. `gpui_macos` is never linked; GPUI's `Platform` trait is implemented on top of this
/// in the AppKit host lane.
pub struct NdAppKitPlatform {
    mtm: MainThreadMarker,
    app: Retained<NSApplication>,
    _delegate: Retained<NdAppDelegate>,
}

impl NdAppKitPlatform {
    pub fn new(mtm: MainThreadMarker) -> Self {
        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
        let delegate = NdAppDelegate::new(mtm);
        app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        app.setMainMenu(Some(&main_menu(mtm)));
        app.finishLaunching();
        Self { mtm, app, _delegate: delegate }
    }

    pub fn mtm(&self) -> MainThreadMarker {
        self.mtm
    }

    pub fn app(&self) -> &NSApplication {
        &self.app
    }

    /// A titled, resizable window with a full-size content view, not yet shown.
    pub fn open_window(&self, title: &str, width: f64, height: f64) -> Retained<NdWindow> {
        let style = NSWindowStyleMask::Titled
            | NSWindowStyleMask::Closable
            | NSWindowStyleMask::Miniaturizable
            | NSWindowStyleMask::Resizable
            | NSWindowStyleMask::FullSizeContentView;
        let rect = NSRect::new(NSPoint::new(200., 200.), NSSize::new(width, height));
        let window = NdWindow::new(self.mtm, rect, style);
        window.setTitle(&NSString::from_str(title));
        window
    }

    /// Shows `window` and asks for activation. Activation is cooperative since macOS 14, so a
    /// process started in the background may stay inactive; check `isKeyWindow`.
    pub fn show(&self, window: &NdWindow) {
        window.makeKeyAndOrderFront(None);
        #[allow(deprecated)]
        self.app.activateIgnoringOtherApps(true);
    }

    /// Delivers queued events through `NSApp.sendEvent`, the same path as `run`, until the
    /// queue stays empty for `idle` or `limit` passes.
    pub fn pump(&self, idle: Duration, limit: Duration) {
        let start = Instant::now();
        while start.elapsed() < limit {
            let until = NSDate::dateWithTimeIntervalSinceNow(idle.as_secs_f64());
            let event = unsafe {
                self.app.nextEventMatchingMask_untilDate_inMode_dequeue(
                    NSEventMask::Any,
                    Some(&until),
                    NSDefaultRunLoopMode,
                    true,
                )
            };
            match event {
                Some(event) => self.app.sendEvent(&event),
                None => break,
            }
        }
        self.app.updateWindows();
    }

    pub fn run(&self) {
        self.app.run();
    }
}

fn main_menu(mtm: MainThreadMarker) -> Retained<NSMenu> {
    let bar = NSMenu::new(mtm);
    let app_item = NSMenuItem::new(mtm);
    let app_menu = NSMenu::new(mtm);
    let quit = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &NSString::from_str("Quit"),
            Some(sel!(terminate:)),
            &NSString::from_str("q"),
        )
    };
    app_menu.addItem(&quit);
    app_item.setSubmenu(Some(&app_menu));
    bar.addItem(&app_item);

    let file_item = NSMenuItem::new(mtm);
    let file_menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str("File"));
    let close = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &NSString::from_str("Close"),
            Some(sel!(performClose:)),
            &NSString::from_str("w"),
        )
    };
    file_menu.addItem(&close);
    file_item.setSubmenu(Some(&file_menu));
    bar.addItem(&file_item);

    let view_item = NSMenuItem::new(mtm);
    let view_menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str("View"));
    let full_screen = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &NSString::from_str("Enter Full Screen"),
            Some(sel!(toggleFullScreen:)),
            &NSString::from_str("f"),
        )
    };
    full_screen.setKeyEquivalentModifierMask(NSEventModifierFlags::Control | NSEventModifierFlags::Command);
    view_menu.addItem(&full_screen);
    view_item.setSubmenu(Some(&view_menu));
    bar.addItem(&view_item);
    bar
}
