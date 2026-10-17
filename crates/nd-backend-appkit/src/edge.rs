//! A pane's scroll edge effect under the toolbar shown only while the pointer is over the
//! toolbar's band (`Scaffold::content_edge_on_hover`), as Messages shows its transcript's on
//! macOS 26: at rest the content runs under the toolbar untouched.
//!
//! AppKit has no public switch for the effect. It draws it in a scroll pocket, a private
//! subview of the scroll view (`NSScrollPocket`); nd fades that view's alpha, with the durations
//! and curves measured off Messages at 60 fps: 0.25 s easing out on the way in, 0.3 s easing in
//! and out on the way out.

use std::cell::Cell;
use std::rc::Rc;

use block2::RcBlock;
use objc2::rc::{Retained, Weak};
use objc2::{AnyThread, DefinedClass, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSAnimatablePropertyContainer, NSAnimationContext, NSEvent, NSScrollView, NSTrackingArea, NSTrackingAreaOptions,
    NSView, NSWindow, NSWorkspace,
};
use objc2_foundation::{MainThreadMarker, NSObject, NSRect};
use objc2_quartz_core::{CAMediaTimingFunction, kCAMediaTimingFunctionEaseInEaseOut, kCAMediaTimingFunctionEaseOut};

pub struct EdgeIvars {
    window: Weak<NSWindow>,
    pane: Weak<NSView>,
    shown: Rc<Cell<bool>>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements and this type does not implement Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = EdgeIvars]
    pub(crate) struct NdEdgeTracker;

    impl NdEdgeTracker {
        #[unsafe(method(mouseMoved:))]
        fn mouse_moved(&self, event: &NSEvent) {
            self.track(Some(event));
        }

        #[unsafe(method(mouseEntered:))]
        fn mouse_entered(&self, event: &NSEvent) {
            self.track(Some(event));
        }

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, _event: &NSEvent) {
            self.track(None);
        }
    }
);

impl NdEdgeTracker {
    /// Follows the pointer over `window` for `pane`. The tracking area sits on the theme frame:
    /// the titlebar covers the content view there and takes its events.
    pub(crate) fn install(window: &NSWindow, pane: &NSView, shown: Rc<Cell<bool>>) -> Option<Retained<Self>> {
        let mtm = MainThreadMarker::new()?;
        let frame = window.contentView().and_then(|v| unsafe { v.superview() })?;
        let this = Self::alloc(mtm).set_ivars(EdgeIvars { window: Weak::new(window), pane: Weak::new(pane), shown });
        let this: Retained<Self> = unsafe { msg_send![super(this), init] };
        let options = NSTrackingAreaOptions::MouseEnteredAndExited
            | NSTrackingAreaOptions::MouseMoved
            | NSTrackingAreaOptions::ActiveAlways
            | NSTrackingAreaOptions::InVisibleRect;
        let area = unsafe {
            NSTrackingArea::initWithRect_options_owner_userInfo(
                NSTrackingArea::alloc(),
                NSRect::ZERO,
                options,
                Some(&this),
                None,
            )
        };
        frame.addTrackingArea(&area);
        Some(this)
    }

    /// Over the band: as far down from the window's top as the pane's content is inset for the
    /// bars (the titlebar and the accessories under it).
    fn track(&self, event: Option<&NSEvent>) {
        let (Some(window), Some(pane)) = (self.ivars().window.load(), self.ivars().pane.load()) else { return };
        let band = deepest_bars(&pane);
        let height = window.frame().size.height;
        let over = band > 0. && event.is_some_and(|e| e.locationInWindow().y >= height - band);
        if self.ivars().shown.replace(over) != over {
            apply(&pane, over);
        }
    }
}

/// The largest top safe area inset of a scroll view under `view`: how far the bars reach.
fn deepest_bars(view: &NSView) -> f64 {
    let own = view.downcast_ref::<NSScrollView>().map_or(0., |s| s.safeAreaInsets().top);
    view.subviews().iter().map(|c| deepest_bars(&c)).fold(own, f64::max)
}

/// Shows or hides the edge effect of every scroll view under `view` that runs under the bars.
pub(crate) fn apply(view: &NSView, shown: bool) {
    if let Some(scroll) = view.downcast_ref::<NSScrollView>() {
        set(scroll, shown, true);
    }
    for child in view.subviews().iter() {
        apply(&child, shown);
    }
}

/// How long the effect takes to show and to go, in seconds.
const SHOW: f64 = 0.25;
const HIDE: f64 = 0.3;

/// One scroll view, when it runs under the bars: its pocket's alpha, faded when `animated`.
pub(crate) fn set(scroll: &NSScrollView, shown: bool, animated: bool) {
    if scroll.safeAreaInsets().top <= 0. {
        return;
    }
    let to = if shown { 1. } else { 0. };
    for pocket in scroll.subviews().iter() {
        if !pocket.class().name().to_string_lossy().ends_with("NSScrollPocket") || pocket.alphaValue() == to {
            continue;
        }
        if !animated || NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion() {
            pocket.setAlphaValue(to);
            continue;
        }
        let fading = pocket.clone();
        let changes = RcBlock::new(move |context: std::ptr::NonNull<NSAnimationContext>| {
            let context = unsafe { context.as_ref() };
            context.setDuration(if shown { SHOW } else { HIDE });
            let curve = if shown {
                unsafe { kCAMediaTimingFunctionEaseOut }
            } else {
                unsafe { kCAMediaTimingFunctionEaseInEaseOut }
            };
            context.setTimingFunction(Some(&CAMediaTimingFunction::functionWithName(curve)));
            fading.animator().setAlphaValue(to);
        });
        NSAnimationContext::runAnimationGroup(&changes);
    }
}
