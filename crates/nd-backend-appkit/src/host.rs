//! One window's host: its content root, its display clock, traffic light placement and the
//! window delegate, wired to the shared [`AppKitBackend`].
//!
//! `ND_DIAG_WINDOW=1` prints `nd: ...` lines on stderr for live resize, full screen, key status,
//! the window's frame in global top-left coordinates and the traffic light origin.
//! `ND_DEBUG_EXPOSE=1` paints the window's own background magenta, so any area content fails to
//! cover (a frame laid out late during a resize) shows up in captures.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use nd_core::{Point, Size, WindowId};
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSColor, NSScreen, NSViewFrameDidChangeNotification, NSWindowDelegate, NSWindowTitleVisibility,
};
use objc2_foundation::{MainThreadMarker, NSNotification, NSNotificationCenter, NSObject, NSObjectProtocol};

use crate::backend::AppKitBackend;
use crate::frame_clock::FrameClock;
use crate::traffic_lights::{self, TrafficLights};
use crate::views::NdRootView;
use crate::NdWindow;

#[derive(Clone, Debug, Default)]
pub struct HostOptions {
    /// Pins the traffic lights' close button top-left at this point, in points from the
    /// window's top-left. `None` keeps AppKit's placement.
    pub traffic_lights: Option<Point>,
}

struct Inner {
    id: WindowId,
    window: Retained<NdWindow>,
    root: Retained<NdRootView>,
    backend: Rc<RefCell<AppKitBackend>>,
    lights: TrafficLights,
    clock: FrameClock,
    diag: bool,
    live_layouts: Cell<Option<u32>>,
    last_lights: Cell<Option<Point>>,
    delegate: RefCell<Option<Retained<NdWindowDelegate>>>,
}

impl Inner {
    fn place_lights(&self) {
        let placed = self.lights.apply(&self.window);
        if self.diag && placed.is_some() && placed != self.last_lights.get() {
            let p = placed.unwrap();
            eprintln!("nd: traffic lights close={},{}", p.x, p.y);
        }
        if placed.is_some() {
            self.last_lights.set(placed);
        }
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
    pub fn new(
        backend: &Rc<RefCell<AppKitBackend>>,
        id: WindowId,
        window: &NdWindow,
        options: HostOptions,
    ) -> Self {
        use objc2::Message;
        let mtm = MainThreadMarker::from(window);
        let diag = std::env::var_os("ND_DIAG_WINDOW").is_some_and(|v| v == "1");
        let root = NdRootView::new(mtm);
        window.setContentView(Some(&root));
        if std::env::var_os("ND_DEBUG_EXPOSE").is_some_and(|v| v == "1") {
            window.setBackgroundColor(Some(&NSColor::magentaColor()));
        }
        let clock = FrameClock::new(mtm, &root, diag);
        backend.borrow_mut().add_window(id, window, &root);
        let inner = Rc::new(Inner {
            id,
            window: window.retain(),
            root,
            backend: backend.clone(),
            lights: TrafficLights::new(options.traffic_lights),
            clock,
            diag,
            live_layouts: Cell::new(None),
            last_lights: Cell::new(None),
            delegate: RefCell::new(None),
        });
        let delegate = NdWindowDelegate::alloc(mtm).set_ivars(DelegateIvars { host: Rc::downgrade(&inner) });
        let delegate: Retained<NdWindowDelegate> = unsafe { msg_send![super(delegate), init] };
        window.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        if options.traffic_lights.is_some()
            && let Some(buttons) = traffic_lights::buttons(window)
        {
            // Pinned lights mean the app draws its own title row (`titlebar()`), so the stock
            // title and title bar material would sit under and beside the moved buttons.
            window.setTitleVisibility(NSWindowTitleVisibility::Hidden);
            window.setTitlebarAppearsTransparent(true);
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
            layout(size);
            if let Some(host) = host.upgrade() {
                if let Some(n) = host.live_layouts.get() {
                    host.live_layouts.set(Some(n + 1));
                }
                host.place_lights();
            }
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
        if let Ok(mut backend) = self.inner.backend.try_borrow_mut() {
            backend.remove_window(self.inner.id);
        }
    }
}
