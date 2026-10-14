//! The AppKit presenter. Each `webview()` node is a plain host view the backend frames like any
//! other; the page's engine view fills it through autoresizing, so it moves and resizes in the
//! same Core Animation transaction as its siblings, and overlays (their own windows, 6.3) stay
//! above it. The host only holds the engine view, so a page moving to another node (or window)
//! is a reparent, never a reload.

pub mod wk;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use nd_backend_appkit::custom::{CustomCx, CustomNative, CustomWidget};
use nd_theme::Theme;
use nd_ui::semantics::Probe;
use objc2::rc::Retained;
use objc2::runtime::NSObjectProtocol;
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSAutoresizingMaskOptions, NSColor, NSResponder, NSTextField, NSView};
use objc2_foundation::{MainThreadMarker, NSRect, NSString};

use crate::element::{WebViewProps, WebViewWidget};
use crate::engine::{EngineView, Target};
use crate::events::LoadFailed;
use crate::handle::WebView;

pub(crate) fn register() {
    nd_backend_appkit::custom::register::<WebViewWidget>(Presenter);
}

/// Runs `f` once, `seconds` from now, on the main run loop in common modes, so it fires during
/// tracking loops and live resize too.
pub(crate) fn after(seconds: f64, f: impl FnOnce() + 'static) {
    use block2::RcBlock;
    use objc2_foundation::{NSRunLoop, NSRunLoopCommonModes, NSTimer};
    let f = Cell::new(Some(f));
    let block = RcBlock::new(move |_timer: std::ptr::NonNull<NSTimer>| {
        if let Some(f) = f.take() {
            f();
        }
    });
    // SAFETY: the timer is added to the main run loop, the thread `f` was created on.
    let timer = unsafe { NSTimer::timerWithTimeInterval_repeats_block(seconds, false, &block) };
    unsafe { NSRunLoop::mainRunLoop().addTimer_forMode(&timer, NSRunLoopCommonModes) };
}

struct Presenter;

impl CustomWidget for Presenter {
    fn create(&self, cx: &CustomCx<'_>) -> Box<dyn CustomNative> {
        let props = props(cx);
        let host = HostView::new(cx.mtm());
        let mut native = Native { host, page: props.page.clone(), engine: None, props: props.clone() };
        native.mount(cx);
        Box::new(native)
    }
}

fn props(cx: &CustomCx<'_>) -> WebViewProps {
    cx.props().downcast_ref::<WebViewProps>().expect("webview node needs WebViewProps").clone()
}

struct Native {
    host: Retained<HostView>,
    page: WebView,
    engine: Option<Rc<dyn EngineView>>,
    props: WebViewProps,
}

impl Native {
    fn mount(&mut self, cx: &CustomCx<'_>) {
        let page = self.page.clone();
        let shared = page.0.shared.clone();
        let wake = cx.waker();
        *shared.target.borrow_mut() = Some(Target { emit: cx.emitter(), wake: Rc::new(move || wake.wake()) });
        *shared.props.borrow_mut() = Some(self.props.clone());
        *page.0.mounts.borrow_mut() += 1;
        match page.view() {
            Ok(engine) => {
                let view = engine.ns_view();
                view.setFrame(self.host.bounds());
                view.setAutoresizingMask(
                    NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
                );
                self.host.addSubview(&view);
                *self.host.ivars().content.borrow_mut() = Some(view.clone());
                engine.set_attached(true);
                engine.apply(&self.props, None);
                self.engine = Some(engine);
                // The element that shows a parked page hears where it is.
                let state = page.state();
                let p = page.page();
                let replay = crate::engine::PageState::default();
                *shared.state.borrow_mut() = crate::engine::PageState { zoom: state.zoom, ..replay };
                p.report_url(&state.url);
                p.report_title(&state.title);
                p.report_loading(state.loading);
                p.report_back(state.can_go_back);
                p.report_forward(state.can_go_forward);
                p.report_progress(state.progress);
            }
            Err(error) => {
                self.host.show_error(&error);
                page.page().emit(LoadFailed { url: self.props.url.clone().unwrap_or_default(), error });
            }
        }
    }
}

impl Native {
    /// Gives the page up: it parks, unless a later mount has taken it over already.
    fn unmount(&mut self) {
        let shared = &self.page.0.shared;
        {
            let mut mounts = self.page.0.mounts.borrow_mut();
            *mounts = mounts.saturating_sub(1);
        }
        let content = self.host.ivars().content.borrow_mut().take();
        let still_ours = content
            .as_ref()
            .is_some_and(|v| unsafe { v.superview() }.is_some_and(|s| std::ptr::eq(&*s, &**self.host as &NSView)));
        if still_ours {
            *shared.target.borrow_mut() = None;
            *shared.props.borrow_mut() = None;
            if let Some(view) = content {
                view.removeFromSuperview();
            }
            if let Some(engine) = &self.engine {
                engine.set_attached(false);
            }
        }
        self.engine = None;
    }
}

impl Drop for Native {
    fn drop(&mut self) {
        self.unmount();
    }
}

impl CustomNative for Native {
    fn view(&self) -> &NSView {
        &self.host
    }

    fn update(&mut self, cx: &CustomCx<'_>) {
        let new = props(cx);
        if new.page != self.page {
            // A different page under the same key: a fresh mount.
            self.unmount();
            self.page = new.page.clone();
            self.props = new;
            self.mount(cx);
            return;
        }
        *self.page.0.shared.props.borrow_mut() = Some(new.clone());
        if let Some(engine) = &self.engine {
            engine.apply(&new, Some(&self.props));
        }
        self.props = new;
    }

    fn set_theme(&mut self, _theme: &Theme) {}

    /// The engine's own view takes the keyboard; the host never does, so a click into the page
    /// goes straight to the engine.
    fn focus_view(&self) -> Option<Retained<NSView>> {
        self.host.ivars().content.borrow().clone()
    }

    fn probe(&self, probe: &mut Probe) {
        let state = self.page.state();
        probe.name = Some(state.title);
        probe.value = Some(state.url);
    }
}

pub struct HostIvars {
    content: RefCell<Option<Retained<NSView>>>,
}

define_class!(
    // SAFETY: NSView has no subclassing requirements; the overrides call super where AppKit
    // expects it. No Drop impl.
    #[unsafe(super(NSView, NSResponder))]
    #[thread_kind = MainThreadOnly]
    #[ivars = HostIvars]
    pub struct HostView;

    unsafe impl NSObjectProtocol for HostView {}

    impl HostView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }
    }
);

impl HostView {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(HostIvars { content: RefCell::default() });
        unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] }
    }

    /// The engine could not start: say why where the page would be.
    fn show_error(&self, error: &str) {
        let mtm = self.mtm();
        let label = NSTextField::wrappingLabelWithString(&NSString::from_str(error), mtm);
        label.setTextColor(Some(&NSColor::secondaryLabelColor()));
        label.setFrame(self.bounds().inset(16., 16.));
        label.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        self.addSubview(&label);
    }
}

trait Inset {
    fn inset(self, dx: f64, dy: f64) -> Self;
}

impl Inset for NSRect {
    fn inset(mut self, dx: f64, dy: f64) -> Self {
        self.origin.x += dx;
        self.origin.y += dy;
        self.size.width = (self.size.width - 2. * dx).max(0.);
        self.size.height = (self.size.height - 2. * dy).max(0.);
        self
    }
}
