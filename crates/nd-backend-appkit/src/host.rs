//! One window's host: its content root, its display clock, traffic light placement and the
//! window delegate, wired to the shared [`AppKitBackend`].
//!
//! `ND_DIAG_WINDOW=1` prints `nd: ...` lines on stderr for live resize, full screen, key status,
//! the window's frame in global top-left coordinates and the traffic light origin.
//! `ND_DEBUG_EXPOSE=1` paints the window's own background magenta, so any area content fails to
//! cover (a frame laid out late during a resize) shows up in captures.
//! `ND_DEBUG_DEFER_LIVE_LAYOUT=1` skips layout until a live resize ends: the bug the resize gate
//! exists to catch, kept so the gate can prove it catches it.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use nd_core::{Size, WindowId};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{NSColor, NSScreen, NSViewFrameDidChangeNotification, NSWindow, NSWindowDelegate};
use objc2_foundation::{MainThreadMarker, NSNotification, NSNotificationCenter, NSObject, NSObjectProtocol};

use crate::NdWindow;
use crate::backend::{AppKitBackend, WindowRecord};
use crate::frame_clock::FrameClock;
use crate::traffic_lights::{self, TrafficLights};
use crate::views::NdRootView;

struct Inner {
    id: WindowId,
    window: Retained<NdWindow>,
    root: Retained<NdRootView>,
    backend: AppKitBackend,
    lights: Rc<TrafficLights>,
    clock: FrameClock,
    diag: bool,
    defer_live_layout: bool,
    live_layouts: Cell<Option<u32>>,
    delegate: RefCell<Option<Retained<NdWindowDelegate>>>,
}

impl Inner {
    fn place_lights(&self) {
        self.lights.apply(&self.window);
    }

    fn diag(&self, what: &str) {
        if self.diag {
            eprintln!("nd: {what}");
        }
    }

    fn diag_frame(&self) {
        if !self.diag {
            return;
        }
        let f = self.window.frame();
        // Global coordinates with a top-left origin on the primary display, as CGEvent and
        // CGWindowList use them.
        let mtm = MainThreadMarker::from(&*self.window);
        let primary = NSScreen::screens(mtm).firstObject().map_or(0., |s| s.frame().size.height);
        let top = primary - (f.origin.y + f.size.height);
        eprintln!(
            "nd: window frame {} {} {} {} number={}",
            f.origin.x,
            top,
            f.size.width,
            f.size.height,
            self.window.windowNumber()
        );
    }
}

fn env_flag(name: &str) -> bool {
    std::env::var_os(name).is_some_and(|v| v == "1")
}

pub struct DelegateIvars {
    host: Weak<Inner>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements and this type does not implement Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = DelegateIvars]
    struct NdWindowDelegate;

    unsafe impl NSObjectProtocol for NdWindowDelegate {}

    unsafe impl NSWindowDelegate for NdWindowDelegate {
        // The close button, cmd-w and a tab's close all land here. The app decides, on a later
        // turn (3.13); a close it allows comes back as the host's drop, which closes the window.
        #[unsafe(method(windowShouldClose:))]
        fn window_should_close(&self, _sender: &NSWindow) -> bool {
            if let Some(host) = self.ivars().host.upgrade() {
                let id = host.id;
                crate::defer_with_app(move |app| {
                    app.request_close(id);
                });
            }
            false
        }

        #[unsafe(method(windowWillStartLiveResize:))]
        fn will_start_live_resize(&self, _n: &NSNotification) {
            self.with(|h| {
                h.live_layouts.set(Some(0));
                h.diag("live resize start");
            });
        }

        #[unsafe(method(windowDidEndLiveResize:))]
        fn did_end_live_resize(&self, _n: &NSNotification) {
            self.with(|h| {
                let layouts = h.live_layouts.take().unwrap_or(0);
                h.diag(&format!("live resize end layouts={layouts}"));
                h.root.setNeedsLayout(true);
                h.place_lights();
                h.diag_frame();
            });
        }

        #[unsafe(method(windowDidResize:))]
        fn did_resize(&self, _n: &NSNotification) {
            self.with(Inner::place_lights);
        }

        #[unsafe(method(windowDidBecomeKey:))]
        fn did_become_key(&self, _n: &NSNotification) {
            self.with(|h| {
                h.place_lights();
                h.diag("window key");
                h.diag_frame();
            });
        }

        #[unsafe(method(windowDidResignKey:))]
        fn did_resign_key(&self, _n: &NSNotification) {
            self.with(Inner::place_lights);
        }

        #[unsafe(method(windowDidEnterFullScreen:))]
        fn did_enter_full_screen(&self, _n: &NSNotification) {
            self.with(|h| h.diag("full screen enter"));
        }

        #[unsafe(method(windowDidExitFullScreen:))]
        fn did_exit_full_screen(&self, _n: &NSNotification) {
            self.with(|h| {
                h.place_lights();
                h.diag("full screen exit");
                h.diag_frame();
            });
        }

        #[unsafe(method(windowDidChangeScreen:))]
        fn did_change_screen(&self, _n: &NSNotification) {
            self.with(Inner::place_lights);
        }
    }

    impl NdWindowDelegate {
        // Being in the responder chain is what makes AppKit show the tab bar's `+`.
        #[unsafe(method(newWindowForTab:))]
        fn new_window_for_tab(&self, _sender: Option<&AnyObject>) {
            let Some(host) = self.ivars().host.upgrade() else { return };
            let id = crate::app_host::key_window().unwrap_or(host.id);
            crate::defer_with_app(move |app| {
                app.request_new_tab(id);
            });
        }

        // Every title bar layout AppKit runs moves a button, and this lands inside that same
        // pass, so the stock position is never drawn.
        #[unsafe(method(ndTitlebarButtonMoved:))]
        fn titlebar_button_moved(&self, _n: &NSNotification) {
            self.with(Inner::place_lights);
        }
    }
);

impl NdWindowDelegate {
    fn with(&self, f: impl FnOnce(&Inner)) {
        if let Some(host) = self.ivars().host.upgrade() {
            f(&host);
        }
    }
}

/// Owns the window's nd side. Dropping it detaches the window from the backend.
pub struct WindowHost {
    inner: Rc<Inner>,
}

impl WindowHost {
    pub fn new(backend: &AppKitBackend, id: WindowId, window: &NdWindow) -> Self {
        use objc2::Message;
        let mtm = MainThreadMarker::from(window);
        let diag = env_flag("ND_DIAG_WINDOW");
        let root = NdRootView::new(mtm);
        window.setContentView(Some(&root));
        if env_flag("ND_DEBUG_EXPOSE") {
            window.setBackgroundColor(Some(&NSColor::magentaColor()));
        }
        let clock = FrameClock::new(mtm, &root, diag);
        let lights = Rc::new(TrafficLights::new(diag));
        backend.add_window(id, WindowRecord { window: window.retain(), root: root.clone(), lights: lights.clone() });
        let inner = Rc::new(Inner {
            id,
            window: window.retain(),
            root,
            backend: backend.clone(),
            lights,
            clock,
            diag,
            defer_live_layout: env_flag("ND_DEBUG_DEFER_LIVE_LAYOUT"),
            live_layouts: Cell::new(None),
            delegate: RefCell::new(None),
        });
        let delegate = NdWindowDelegate::alloc(mtm).set_ivars(DelegateIvars { host: Rc::downgrade(&inner) });
        let delegate: Retained<NdWindowDelegate> = unsafe { msg_send![super(delegate), init] };
        window.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        if let Some(buttons) = traffic_lights::buttons(window) {
            let center = NSNotificationCenter::defaultCenter();
            for button in &buttons {
                button.setPostsFrameChangedNotifications(true);
                unsafe {
                    center.addObserver_selector_name_object(
                        &delegate,
                        sel!(ndTitlebarButtonMoved:),
                        Some(NSViewFrameDidChangeNotification),
                        Some(button),
                    );
                }
            }
        }
        *inner.delegate.borrow_mut() = Some(delegate);
        inner.place_lights();
        Self { inner }
    }

    pub fn id(&self) -> WindowId {
        self.inner.id
    }

    /// Applies what the window was opened with: a custom title bar (content under a
    /// transparent title bar, title hidden) or the settings window's toolbar style.
    pub fn apply_options(&self, options: &nd_ui::WindowOptions) {
        let window = &self.inner.window;
        if options.titlebar == nd_ui::Titlebar::Custom {
            window.setTitlebarAppearsTransparent(true);
            window.setTitleVisibility(objc2_app_kit::NSWindowTitleVisibility::Hidden);
            self.inner.root.set_full_size(true);
        }
        match options.kind {
            nd_ui::WindowKind::Settings => {
                window.setToolbarStyle(objc2_app_kit::NSWindowToolbarStyle::Preference);
                let mask = window.styleMask() & !objc2_app_kit::NSWindowStyleMask::Miniaturizable;
                window.setStyleMask(mask);
            }
            nd_ui::WindowKind::Sheet { .. } => {
                let mask = window.styleMask() & !objc2_app_kit::NSWindowStyleMask::Miniaturizable;
                window.setStyleMask(mask);
            }
            nd_ui::WindowKind::Normal => {}
        }
        if let Some(min) = options.min_size {
            window.setContentMinSize(objc2_foundation::NSSize::new(
                min.width.as_f32().into(),
                min.height.as_f32().into(),
            ));
        }
    }

    pub fn window(&self) -> &NdWindow {
        &self.inner.window
    }

    pub fn clock(&self) -> &FrameClock {
        &self.inner.clock
    }

    /// Runs `layout` with the content size inside AppKit's layout pass: on every display cycle
    /// that needs it, and on every frame of a live resize, before that frame is drawn. It is
    /// where the transaction carrying frames for this window commits.
    pub fn on_layout(&self, layout: impl Fn(Size) + 'static) {
        let host = Rc::downgrade(&self.inner);
        self.inner.root.set_layout_hook(Some(Rc::new(move |size| {
            let Some(host) = host.upgrade() else { return };
            let live = host.live_layouts.get();
            if let Some(n) = live {
                host.live_layouts.set(Some(n + 1));
            }
            if live.is_none() || !host.defer_live_layout {
                layout(size);
            }
            host.place_lights();
        })));
    }

    /// Asks AppKit for a layout pass on the next display cycle.
    pub fn request_layout(&self) {
        self.inner.root.setNeedsLayout(true);
    }
}

impl Drop for WindowHost {
    fn drop(&mut self) {
        self.inner.root.set_layout_hook(None);
        self.inner.window.setDelegate(None);
        if let Some(delegate) = self.inner.delegate.borrow().as_ref() {
            unsafe { NSNotificationCenter::defaultCenter().removeObserver(delegate) };
        }
        // The app closed the window: a sheet ends on its parent; any other window closes, so a
        // tab leaves its group and AppKit selects its neighbour.
        if let Some(parent) = self.inner.window.sheetParent() {
            parent.endSheet(&self.inner.window);
        } else {
            self.inner.window.close();
        }
        self.inner.backend.remove_window(self.inner.id);
    }
}
