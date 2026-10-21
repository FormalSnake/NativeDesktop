//! Chrome style on macOS. A non-null `parent_view` forces Alloy style (cef_types_mac.h), so a
//! Chrome-style browser is born in a CEF Views window: a frameless, invisible "anchor" window
//! hosting the BrowserView with its toolbar hidden. Its content view, the one NSView holding
//! everything Chromium shows (the compositor's view and the web contents' native views), is then
//! lifted into the page's own view, so the page's pixels and input live in the app's window as an
//! ordinary subview: clipped by its parents, under the window's overlays, resized in the window's
//! own layout pass.
//!
//! The anchor stays alive, alpha 0 and click-through, glued in screen coordinates over the page
//! and up to the app window's content corner (see [`Chrome::offset`]), as a child window of the
//! app's window. Chromium places what it draws outside the page (`<select>`
//! popups, autofill, permission and extension bubbles) against the widget's screen bounds, and
//! those come from the anchor; and Chromium only produces frames for a widget it believes is
//! shown.

use std::cell::{Cell, RefCell};
use std::ptr::NonNull;
use std::rc::Rc;

use block2::RcBlock;
use cef::*;
use objc2::rc::{Retained, Weak};
use objc2::runtime::{AnyObject, NSObjectProtocol, ProtocolObject};
use objc2::{MainThreadOnly, Message, msg_send};
use objc2_app_kit::{
    NSView, NSWindow, NSWindowAnimationBehavior, NSWindowCollectionBehavior, NSWindowDidBecomeKeyNotification,
    NSWindowDidChangeScreenNotification, NSWindowDidDeminiaturizeNotification, NSWindowDidEndLiveResizeNotification,
    NSWindowDidEnterFullScreenNotification, NSWindowDidExitFullScreenNotification, NSWindowDidMoveNotification,
    NSWindowDidResizeNotification, NSWindowOrderingMode,
};
use objc2_foundation::{
    NSNotification, NSNotificationCenter, NSPoint, NSRect, NSRunLoop, NSRunLoopCommonModes, NSTimer,
};

use nd_webview::extensions::Anchor;

use crate::page::{self, CefPage};

/// How often the lifted subtree is checked: Chromium re-attaches its native view host to the
/// anchor on paths with no callback (a cross-process navigation swapping the render widget, a
/// renderer crash).
const RELIFT_INTERVAL: f64 = 0.25;
/// A page smaller than this on either side shows nothing; its anchor stays hidden, like a
/// background tab's, instead of standing on screen as a window nobody can see.
const SHOWN_MINIMUM: f64 = 10.;

type Observer = Retained<ProtocolObject<dyn NSObjectProtocol>>;

/// One page's Views window, BrowserView and lifted subtree.
pub(crate) struct Chrome {
    page: u64,
    browser_view: RefCell<Option<BrowserView>>,
    window: RefCell<Option<cef::Window>>,
    anchor: RefCell<Option<Retained<NSWindow>>>,
    lifted: RefCell<Option<Retained<NSView>>>,
    relift: RefCell<Option<Retained<NSTimer>>>,
    observers: RefCell<Vec<Observer>>,
    observed: RefCell<Weak<NSWindow>>,
    focused: Cell<bool>,
    closed: Cell<bool>,
    /// The page's offset from the top left of its window's content, laid out as the BrowserView's
    /// inset in the Views window.
    inset: Cell<Option<(f64, f64)>>,
    popup: RefCell<Option<PopupTrack>>,
}

/// An extension popup on its way or on screen. Chromium anchors it to the hidden toolbar, so it
/// is moved under the app's anchor whenever Chromium sizes or places it.
struct PopupTrack {
    id: String,
    anchor: Anchor,
    /// The windows that existed when the click ran; the popup is the new one.
    before: Vec<*const NSWindow>,
    window: Option<Retained<NSWindow>>,
    timer: Option<Retained<NSTimer>>,
    observers: Vec<Observer>,
    deadline: std::time::Instant,
    placing: bool,
    reported: bool,
}

thread_local! {
    /// Pages created in `on_before_popup` for a Chrome-style opener, waiting for the BrowserView
    /// Chromium builds them (`get_delegate_for_popup_browser_view`, then
    /// `on_popup_browser_view_created`).
    static PENDING_POPUPS: RefCell<std::collections::VecDeque<u64>> = RefCell::default();
}

impl Chrome {
    pub(crate) fn new(page: u64) -> Rc<Chrome> {
        Rc::new(Chrome {
            page,
            browser_view: RefCell::default(),
            window: RefCell::default(),
            anchor: RefCell::default(),
            lifted: RefCell::default(),
            relift: RefCell::default(),
            observers: RefCell::default(),
            observed: RefCell::default(),
            focused: Cell::new(false),
            closed: Cell::new(false),
            inset: Cell::new(None),
            popup: RefCell::default(),
        })
    }

    fn page(&self) -> Option<Rc<CefPage>> {
        page::get(self.page)
    }

    fn view(&self) -> Option<super::view::View> {
        self.page().map(|p| p.view.clone())
    }

    /// Creates the BrowserView and its anchor window. The browser itself arrives through
    /// `on_after_created`.
    pub(crate) fn create(
        &self,
        mut client: Client,
        settings: &BrowserSettings,
        mut context: Option<RequestContext>,
    ) -> Result<(), String> {
        let mut delegate = browser_view_delegate(self.page);
        let view = browser_view_create(
            Some(&mut client),
            Some(&CefString::from("")),
            Some(settings),
            None,
            context.as_mut(),
            Some(&mut delegate),
        )
        .ok_or("Chromium could not create the Chrome-style browser view")?;
        self.adopt(view)
    }

    /// Hosts `view` (made here, or a popup's that Chromium made) in a new anchor window.
    fn adopt(&self, view: BrowserView) -> Result<(), String> {
        *self.browser_view.borrow_mut() = Some(view);
        let mut delegate = window_delegate(self.page);
        // `on_window_created` runs before this returns.
        window_create_top_level(Some(&mut delegate)).ok_or("Chromium could not create the Chrome-style window")?;
        Ok(())
    }

    /// `on_window_created`: the BrowserView goes into the frameless window at the page's inset.
    fn window_created(&self, window: &cef::Window) {
        *self.window.borrow_mut() = Some(window.clone());
        if let Some(browser_view) = self.browser_view.borrow().as_ref() {
            window.add_child_view(Some(&mut View::from(browser_view)));
            match browser_view.chrome_toolbar() {
                Some(toolbar) => toolbar.set_visible(0),
                None => tracing::warn!("nd-cef: Chrome style has no toolbar to hide"),
            }
        }
        let handle = window.window_handle();
        // SAFETY: on macOS a Views window's handle is its content NSView.
        let Some(content) = (unsafe { handle.cast::<NSView>().as_ref() }) else {
            tracing::warn!("nd-cef: the Chrome-style window reported no native view");
            return;
        };
        let Some(anchor) = content.window() else {
            tracing::warn!("nd-cef: the Chrome-style window has no NSWindow");
            return;
        };
        imperceptible(&anchor);
        *self.anchor.borrow_mut() = Some(anchor);
        let host_was_key = self.view().and_then(|v| v.window()).is_some_and(|w| w.isKeyWindow());
        window.show();
        if host_was_key && let Some(host) = self.view().and_then(|v| v.window()) {
            host.makeKeyWindow();
        }
        self.geometry_changed();
    }

    /// `on_after_created`: the subtree moves into the page's view, and stays there.
    pub(crate) fn browser_created(&self) {
        self.lift();
        if self.relift.borrow().is_some() {
            return;
        }
        let id = self.page;
        let block = RcBlock::new(move |_timer: NonNull<NSTimer>| {
            if let Some(chrome) = page::get(id).and_then(|p| p.view.chrome()) {
                chrome.lift();
                chrome.sync_focus();
            }
        });
        // SAFETY: added to the main run loop, the thread this runs on.
        let timer = unsafe { NSTimer::timerWithTimeInterval_repeats_block(RELIFT_INTERVAL, true, &block) };
        unsafe { NSRunLoop::mainRunLoop().addTimer_forMode(&timer, NSRunLoopCommonModes) };
        *self.relift.borrow_mut() = Some(timer);
    }

    /// The web contents' view under `point` (window coordinates). Pointer input over the page
    /// goes there directly: Chromium's widget answers for its own root view over the page once
    /// the app's overlays have come and gone, and drops what it gets.
    pub(crate) fn contents_at(&self, point: NSPoint) -> Option<Retained<NSView>> {
        fn find(view: &NSView, point: NSPoint, depth: usize) -> Option<Retained<NSView>> {
            if view.isHidden() || depth > 8 {
                return None;
            }
            for child in view.subviews().to_vec().into_iter().rev() {
                if let Some(found) = find(&child, point, depth + 1) {
                    return Some(found);
                }
            }
            let class = view.class().name();
            let local = view.convertPoint_fromView(point, None);
            let b = view.bounds();
            let inside = local.x >= b.origin.x
                && local.y >= b.origin.y
                && local.x < b.origin.x + b.size.width
                && local.y < b.origin.y + b.size.height;
            (class.to_bytes() == b"RenderWidgetHostViewCocoa" && inside).then(|| view.retain())
        }
        find(self.lifted.borrow().as_ref()?, point, 0)
    }

    /// Moves the anchor's content view into the page's view, or puts it back after Chromium
    /// re-attached it to the anchor. The anchor gets an empty content view of its own, since
    /// AppKit keeps one either way.
    fn lift(&self) {
        if self.closed.get() {
            return;
        }
        let (Some(view), Some(anchor)) = (self.view(), self.anchor.borrow().clone()) else { return };
        let bounds = view.bounds();
        let frame = self.lifted_frame(bounds);
        let lifted = self.lifted.borrow().clone();
        match lifted {
            Some(lifted) => {
                let home = unsafe { lifted.superview() }.is_some_and(|s| std::ptr::eq(&*s, &**view as &NSView));
                if !home {
                    lifted.removeFromSuperview();
                    view.addSubview(&lifted);
                }
                // Chromium sizes the widget's view to its window's content once more after the lift,
                // to nothing, and hides the web contents in a widget of no size.
                if lifted.frame() != frame {
                    lifted.setFrame(frame);
                }
            }
            None => {
                let Some(content) = anchor.contentView() else { return };
                let empty = NSView::initWithFrame(NSView::alloc(view.mtm()), content.frame());
                anchor.setContentView(Some(&empty));
                content.setFrame(frame);
                view.addSubview(&content);
                *self.lifted.borrow_mut() = Some(content);
            }
        }
    }

    /// The page's view changed size, moved, or came or went from a window: the lifted subtree
    /// takes the new size in the same layout pass, the anchor follows on screen.
    pub(crate) fn geometry_changed(&self) {
        if self.closed.get() {
            return;
        }
        let Some(view) = self.view() else { return };
        self.lay_out_inset(self.offset().unwrap_or_default());
        let host = view.window();
        let observed = self.observed.borrow().load();
        if host.as_deref().map(|h| h as *const NSWindow) != observed.as_deref().map(|h| h as *const NSWindow) {
            self.observe(host.as_deref());
        }
        self.place();
        if let Some(lifted) = self.lifted.borrow().as_ref() {
            lifted.setFrame(self.lifted_frame(view.bounds()));
        }
    }

    /// The page's offset from the top left of its window's content area. Chromium takes a mouse
    /// event's location against the content area of the window it arrived in, the app's, and the
    /// widget's own coordinates start there too only if the page's view sits at that corner: so
    /// the widget spans from the corner to the page's far edges, the BrowserView inset by this
    /// much in it, and the part outside the page is clipped by the page's view.
    fn offset(&self) -> Option<(f64, f64)> {
        let view = self.view()?;
        let host = view.window()?;
        let rect = view.convertRect_toView(view.bounds(), None);
        let content = host.contentRectForFrameRect(host.frame());
        let top = content.size.height - (rect.origin.y + rect.size.height);
        Some((rect.origin.x.round().max(0.), top.round().max(0.)))
    }

    fn lay_out_inset(&self, inset: (f64, f64)) {
        if self.inset.get() == Some(inset) {
            return;
        }
        let (Some(window), Some(browser_view)) = (self.window.borrow().clone(), self.browser_view.borrow().clone())
        else {
            return;
        };
        let settings = BoxLayoutSettings {
            inside_border_insets: Insets { top: inset.1 as i32, left: inset.0 as i32, bottom: 0, right: 0 },
            cross_axis_alignment: AxisAlignment::STRETCH,
            ..BoxLayoutSettings::default()
        };
        let Some(layout) = window.set_to_box_layout(Some(&settings)) else { return };
        layout.set_flex_for_view(Some(&mut View::from(&browser_view)), 1);
        window.layout();
        self.inset.set(Some(inset));
    }

    /// The lifted subtree in the page's (flipped) view: from the window content's corner to the
    /// page's far edges.
    fn lifted_frame(&self, bounds: NSRect) -> NSRect {
        let (left, top) = self.inset.get().unwrap_or_default();
        NSRect::new(
            NSPoint::new(-left, -top),
            objc2_foundation::NSSize::new(bounds.size.width + left, bounds.size.height + top),
        )
    }

    /// The anchor's rectangle in screen coordinates: the page's, grown to the window content's
    /// corner. `None` when the page is not on screen.
    fn anchor_frame(&self) -> Option<NSRect> {
        let view = self.view()?;
        let host = view.window()?;
        if view.isHiddenOrHasHiddenAncestor() {
            return None;
        }
        let mut rect = host.convertRectToScreen(view.convertRect_toView(view.bounds(), None));
        if rect.size.width < SHOWN_MINIMUM || rect.size.height < SHOWN_MINIMUM {
            return None;
        }
        let (left, top) = self.inset.get().unwrap_or_default();
        rect.origin.x -= left;
        rect.size.width += left;
        rect.size.height += top;
        Some(rect)
    }

    fn place(&self) {
        let (Some(anchor), Some(window)) = (self.anchor.borrow().clone(), self.window.borrow().clone()) else { return };
        let host = self.view().and_then(|v| v.window());
        let (Some(host), Some(rect)) = (host, self.anchor_frame()) else {
            if anchor.isVisible() {
                window.hide();
            }
            return;
        };
        let parent = anchor.parentWindow();
        if parent.as_deref().map(|p| p as *const NSWindow) != Some(&*host as *const NSWindow) {
            if let Some(parent) = parent {
                parent.removeChildWindow(&anchor);
            }
            unsafe { host.addChildWindow_ordered(&anchor, NSWindowOrderingMode::Above) };
        }
        if anchor.frame() != rect {
            anchor.setFrame_display(rect, false);
        }
        if !anchor.isVisible() {
            window.show();
        }
    }

    /// Follows the host window's moves and resizes, and hands key back to it whenever Chromium
    /// activates its own widget: the anchor is invisible, and key on it would take the app
    /// window's title bar out of its active look. Refusing key in an NSWindow subclass is not an
    /// option, since Chromium's activation path crashes on a window that cannot become key.
    fn observe(&self, host: Option<&NSWindow>) {
        let center = NSNotificationCenter::defaultCenter();
        for observer in self.observers.borrow_mut().drain(..) {
            unsafe { center.removeObserver(observer.as_ref()) };
        }
        *self.observed.borrow_mut() = host.map(Weak::new).unwrap_or_default();
        let Some(host) = host else { return };
        let id = self.page;
        let mut observers = Vec::new();
        if let Some(anchor) = self.anchor.borrow().as_ref() {
            let key = RcBlock::new(move |_note: NonNull<NSNotification>| {
                if let Some(host) = page::get(id).and_then(|p| p.view.window()) {
                    host.makeKeyWindow();
                }
            });
            let object: &AnyObject = anchor;
            observers.push(unsafe {
                center.addObserverForName_object_queue_usingBlock(
                    Some(NSWindowDidBecomeKeyNotification),
                    Some(object),
                    None,
                    &key,
                )
            });
        }
        let sync = RcBlock::new(move |_note: NonNull<NSNotification>| {
            if let Some(chrome) = page::get(id).and_then(|p| p.view.chrome()) {
                chrome.place();
            }
        });
        for name in unsafe {
            [
                NSWindowDidResizeNotification,
                NSWindowDidMoveNotification,
                NSWindowDidEndLiveResizeNotification,
                NSWindowDidEnterFullScreenNotification,
                NSWindowDidExitFullScreenNotification,
                NSWindowDidChangeScreenNotification,
                NSWindowDidDeminiaturizeNotification,
            ]
        } {
            let object: &AnyObject = host;
            observers.push(unsafe {
                center.addObserverForName_object_queue_usingBlock(Some(name), Some(object), None, &sync)
            });
        }
        *self.observers.borrow_mut() = observers;
    }

    /// The view that takes the page's keys: the page's own `RenderWidgetHostViewCocoa`. The lifted
    /// root as first responder would leave Chromium's focus where it was; `set_focus` on the host
    /// would activate the anchor instead.
    pub(crate) fn focus_target(&self) -> Option<Retained<NSView>> {
        let lifted = self.lifted.borrow().clone()?;
        Some(render_widget(&lifted).unwrap_or(lifted))
    }

    /// Whether the keyboard is somewhere in the page.
    pub(crate) fn has_focus(&self) -> bool {
        let Some(lifted) = self.lifted.borrow().clone() else { return false };
        let Some(window) = lifted.window() else { return false };
        if !window.isKeyWindow() {
            return false;
        }
        let Some(responder) = window.firstResponder() else { return false };
        let responder: &AnyObject = &responder;
        let Some(view) = responder.downcast_ref::<NSView>() else { return false };
        view.isDescendantOf(&lifted)
    }

    /// Reports focus changes the page cannot report itself: Chromium has a callback for gaining
    /// the keyboard, none for losing it.
    pub(crate) fn sync_focus(&self) {
        let focused = self.has_focus();
        if self.focused.replace(focused) != focused
            && let Some(page) = self.page()
        {
            page.page.report_focus(focused);
        }
    }

    /// An action click ran: the next window of this process to appear is its popup.
    pub(crate) fn track_popup(&self, id: &str, anchor: Anchor) {
        self.untrack_popup();
        let Some(view) = self.view() else { return };
        let app = objc2_app_kit::NSApplication::sharedApplication(view.mtm());
        let before = app.windows().iter().map(|w| &*w as *const NSWindow).collect();
        let page_id = self.page;
        let block = RcBlock::new(move |_timer: NonNull<NSTimer>| {
            if let Some(chrome) = page::get(page_id).and_then(|p| p.view.chrome()) {
                chrome.find_popup();
            }
        });
        // SAFETY: added to the main run loop, the thread this runs on.
        let timer = unsafe { NSTimer::timerWithTimeInterval_repeats_block(1. / 120., true, &block) };
        unsafe { NSRunLoop::mainRunLoop().addTimer_forMode(&timer, NSRunLoopCommonModes) };
        *self.popup.borrow_mut() = Some(PopupTrack {
            id: id.to_owned(),
            anchor,
            before,
            window: None,
            timer: Some(timer),
            observers: Vec::new(),
            deadline: std::time::Instant::now() + crate::extensions::POPUP_SEARCH,
            placing: false,
            reported: false,
        });
    }

    pub(crate) fn untrack_popup(&self) {
        let Some(track) = self.popup.borrow_mut().take() else { return };
        if let Some(timer) = track.timer {
            timer.invalidate();
        }
        let center = NSNotificationCenter::defaultCenter();
        for observer in track.observers {
            unsafe { center.removeObserver(observer.as_ref()) };
        }
    }

    fn find_popup(&self) {
        let Some(view) = self.view() else { return };
        let found = {
            let popup = self.popup.borrow();
            let Some(track) = popup.as_ref() else { return };
            if track.window.is_some() {
                return;
            }
            if std::time::Instant::now() > track.deadline {
                drop(popup);
                self.untrack_popup();
                return;
            }
            let anchor = self.anchor.borrow().clone();
            let host = view.window();
            let app = objc2_app_kit::NSApplication::sharedApplication(view.mtm());
            app.windows().iter().find(|w| {
                let ptr = &**w as *const NSWindow;
                !track.before.contains(&ptr)
                    && w.isVisible()
                    && anchor.as_deref().is_none_or(|a| !std::ptr::eq(a, &**w))
                    && host.as_deref().is_none_or(|h| !std::ptr::eq(h, &**w))
            })
        };
        let Some(window) = found else { return };
        let id = self.page;
        let place = RcBlock::new(move |_note: NonNull<NSNotification>| {
            if let Some(chrome) = page::get(id).and_then(|p| p.view.chrome()) {
                chrome.place_popup();
            }
        });
        let center = NSNotificationCenter::defaultCenter();
        let object: &AnyObject = &window;
        let observers = unsafe {
            [NSWindowDidResizeNotification, NSWindowDidMoveNotification]
                .map(|name| center.addObserverForName_object_queue_usingBlock(Some(name), Some(object), None, &place))
        };
        if let Some(track) = self.popup.borrow_mut().as_mut() {
            if let Some(timer) = track.timer.take() {
                timer.invalidate();
            }
            track.window = Some(window);
            track.observers.extend(observers);
        }
        self.place_popup();
    }

    /// The popup's top edge on the anchor's bottom, its leading edge on the anchor's, kept on
    /// the screen (flipped above the anchor when there is no room under it).
    fn place_popup(&self) {
        let Some(host) = self.view().and_then(|v| v.window()) else { return };
        let (window, anchor) = {
            let mut popup = self.popup.borrow_mut();
            let Some(track) = popup.as_mut() else { return };
            let Some(window) = track.window.clone() else { return };
            if track.placing {
                return;
            }
            track.placing = true;
            (window, track.anchor)
        };
        let content = host.contentRectForFrameRect(host.frame());
        let top = content.origin.y + content.size.height;
        let frame = window.frame();
        let size = frame.size;
        let visible = host.screen().map(|s| s.visibleFrame()).unwrap_or(content);
        let mut x = content.origin.x + anchor.x;
        if x + size.width > visible.origin.x + visible.size.width {
            x = content.origin.x + anchor.x + anchor.width - size.width;
        }
        x = x.max(visible.origin.x);
        let mut y = top - anchor.y - anchor.height - size.height;
        if y < visible.origin.y {
            y = top - anchor.y;
        }
        let origin = NSPoint::new(x.round(), y.round());
        if frame.origin != origin {
            window.setFrameOrigin(origin);
        }
        let bounds = Anchor::new(origin.x - content.origin.x, top - (origin.y + size.height), size.width, size.height);
        let report = {
            let mut popup = self.popup.borrow_mut();
            let Some(track) = popup.as_mut() else { return };
            track.placing = false;
            (!std::mem::replace(&mut track.reported, true)).then(|| track.id.clone())
        };
        if let (Some(id), Some(page)) = (report, self.page()) {
            page.page.emit(nd_webview::ExtensionPopupOpened { id, bounds });
        }
    }

    /// The subtree goes home before the browser closes: closing it while it lives in another
    /// window takes Chromium's teardown through a hierarchy that no longer exists. The anchor
    /// leaves the screen first, or AppKit can make it key while the app's windows close, after
    /// Chromium's observer for that is gone.
    pub(crate) fn teardown(&self) {
        if self.closed.replace(true) {
            return;
        }
        if let Some(timer) = self.relift.borrow_mut().take() {
            timer.invalidate();
        }
        self.untrack_popup();
        let center = NSNotificationCenter::defaultCenter();
        for observer in self.observers.borrow_mut().drain(..) {
            unsafe { center.removeObserver(observer.as_ref()) };
        }
        let anchor = self.anchor.borrow_mut().take();
        if let (Some(lifted), Some(anchor)) = (self.lifted.borrow_mut().take(), anchor.as_ref()) {
            lifted.removeFromSuperview();
            anchor.setContentView(Some(&lifted));
        }
        if let Some(anchor) = anchor {
            if let Some(parent) = anchor.parentWindow() {
                parent.removeChildWindow(&anchor);
            }
            anchor.orderOut(None);
        }
    }

    /// The browser is gone (`on_before_close`): its window goes with it.
    pub(crate) fn browser_closed(&self) {
        self.teardown();
        self.browser_view.borrow_mut().take();
        if let Some(window) = self.window.borrow_mut().take() {
            window.close();
        }
    }
}

fn imperceptible(anchor: &NSWindow) {
    anchor.setAlphaValue(0.);
    anchor.setHasShadow(false);
    anchor.setIgnoresMouseEvents(true);
    anchor.setExcludedFromWindowsMenu(true);
    // A frameless window in front of the app's own is what assistive clients would read as its
    // first window; the page's accessibility tree hangs off the lifted view instead.
    let _: () = unsafe { msg_send![anchor, setAccessibilityElement: false] };
    anchor.setAnimationBehavior(NSWindowAnimationBehavior::None);
    anchor.setCollectionBehavior(
        NSWindowCollectionBehavior::Transient
            | NSWindowCollectionBehavior::IgnoresCycle
            | NSWindowCollectionBehavior::FullScreenAuxiliary,
    );
}

fn render_widget(view: &NSView) -> Option<Retained<NSView>> {
    for child in view.subviews().iter() {
        if child.class().name().to_bytes().ends_with(b"RenderWidgetHostViewCocoa") {
            return Some(child);
        }
        if let Some(found) = render_widget(&child) {
            return Some(found);
        }
    }
    None
}

/// Queues the page that the next Chrome-style popup of an opener becomes.
pub(crate) fn expect_popup(child: u64) {
    PENDING_POPUPS.with(|p| p.borrow_mut().push_back(child));
}

fn browser_view_delegate(id: u64) -> BrowserViewDelegate {
    NdBrowserView::new(id)
}

fn window_delegate(id: u64) -> WindowDelegate {
    NdChromeWindow::new(id)
}

wrap_browser_view_delegate! {
    struct NdBrowserView {
        id: u64,
    }

    impl ViewDelegate {}

    impl BrowserViewDelegate {
        fn browser_runtime_style(&self) -> RuntimeStyle {
            RuntimeStyle::CHROME
        }

        /// A normal toolbar, hidden once the window exists: Chrome's extension container hangs off
        /// it, and with none (what a native-parent browser gets) extension actions crash.
        fn chrome_toolbar_type(&self, _browser_view: Option<&mut BrowserView>) -> ChromeToolbarType {
            ChromeToolbarType::NORMAL
        }

        fn delegate_for_popup_browser_view(
            &self,
            _browser_view: Option<&mut BrowserView>,
            _settings: Option<&BrowserSettings>,
            _client: Option<&mut Client>,
            is_devtools: std::os::raw::c_int,
        ) -> Option<BrowserViewDelegate> {
            if is_devtools != 0 {
                return None;
            }
            PENDING_POPUPS.with(|p| p.borrow().front().copied()).map(browser_view_delegate)
        }

        /// A popup the opener's page adopted gets a window of its own, like any page; anything
        /// else (DevTools) gets Chromium's default window.
        fn on_popup_browser_view_created(
            &self,
            _browser_view: Option<&mut BrowserView>,
            popup_browser_view: Option<&mut BrowserView>,
            is_devtools: std::os::raw::c_int,
        ) -> std::os::raw::c_int {
            if is_devtools != 0 {
                return 0;
            }
            let (Some(popup), Some(child)) = (popup_browser_view, PENDING_POPUPS.with(|p| p.borrow_mut().pop_front()))
            else {
                return 0;
            };
            let Some(chrome) = page::get(child).and_then(|p| p.view.chrome()) else { return 0 };
            match chrome.adopt(popup.clone()) {
                Ok(()) => 1,
                Err(e) => {
                    tracing::warn!("nd-cef: {e}");
                    0
                }
            }
        }
    }
}

wrap_window_delegate! {
    struct NdChromeWindow {
        id: u64,
    }

    impl ViewDelegate {}

    impl PanelDelegate {}

    impl WindowDelegate {
        fn on_window_created(&self, window: Option<&mut cef::Window>) {
            if let (Some(window), Some(chrome)) = (window, page::get(self.id).and_then(|p| p.view.chrome())) {
                chrome.window_created(window);
            }
        }

        fn window_runtime_style(&self) -> RuntimeStyle {
            RuntimeStyle::CHROME
        }

        fn is_frameless(&self, _window: Option<&mut cef::Window>) -> std::os::raw::c_int {
            1
        }

        fn with_standard_window_buttons(&self, _window: Option<&mut cef::Window>) -> std::os::raw::c_int {
            0
        }

        fn can_resize(&self, _window: Option<&mut cef::Window>) -> std::os::raw::c_int {
            0
        }

        fn can_maximize(&self, _window: Option<&mut cef::Window>) -> std::os::raw::c_int {
            0
        }

        fn can_minimize(&self, _window: Option<&mut cef::Window>) -> std::os::raw::c_int {
            0
        }

        /// The browser's close reaches its window here. One the app started (the page closing for
        /// good) proceeds; one the page asked for (`window.close()`, an approved `request_close`)
        /// goes to the app, which unmounts the page when it agrees.
        fn can_close(&self, _window: Option<&mut cef::Window>) -> std::os::raw::c_int {
            page::get(self.id).map_or(1, |p| std::os::raw::c_int::from(!p.do_close()))
        }

        /// Presses into a page of an inactive window act at once, as in an off-screen page.
        fn accepts_first_mouse(&self, _window: Option<&mut cef::Window>) -> State {
            State::ENABLED
        }

        fn initial_bounds(&self, _window: Option<&mut cef::Window>) -> Rect {
            let rect = page::get(self.id).and_then(|p| p.view.chrome()).and_then(|c| c.anchor_frame());
            rect.map_or(Rect { x: 0, y: 0, width: 800, height: 600 }, |r| Rect {
                x: 0,
                y: 0,
                width: r.size.width.round() as i32,
                height: r.size.height.round() as i32,
            })
        }
    }
}
