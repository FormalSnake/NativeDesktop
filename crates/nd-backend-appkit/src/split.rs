//! `split()` on AppKit: an `NSSplitView` with the thin divider, so the OS draws the divider and
//! owns its cursor and drag. Its two panes are [`NdPaneView`]s, layout boundaries like the
//! scaffold's: the split view frames them, nd lays each pane's content out inside, and every
//! size change asks the window for a layout pass.
//!
//! The ratio is controlled (ARCHITECTURE.md 3.13): the delegate places the panes by the ratio
//! last applied or reported, on every resize of the split view, so a window resize keeps it and
//! reports nothing. A divider move that leaves the first pane somewhere else than that placement
//! is the user's (a drag, or `setPosition:ofDividerAtIndex:`) and reports [`SplitResized`].

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use nd_components::split::Axis;
use nd_components::{SplitPaneProps, SplitProps, SplitResized};
use nd_core::Size;
use nd_ui::events::{Emit, Synced};
use nd_ui::semantics::{ActionError, SemanticAction, WidgetSemantics};
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSSplitView, NSSplitViewDelegate, NSSplitViewDividerStyle, NSView};
use objc2_foundation::{
    MainThreadMarker, NSInteger, NSNotification, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize,
};

use crate::events::Wake;
use crate::look::ViewLook;
use crate::scaffold::{Hook, NdPaneView};
use crate::views::NdLayerView;

/// Positions closer than this are the same placement: what a ratio round trip loses to floats.
const SAME_POSITION: f64 = 0.5; // nd-lint: allow, a float tolerance, not a metric

pub struct SplitIvars {
    ratio: Rc<RefCell<Synced<f32>>>,
    emit: Emit,
    wake: Wake,
    on_resize: Hook,
    /// Placements nd made, for the echo test.
    placements: Cell<usize>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements and this type does not implement Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = SplitIvars]
    struct NdSplitDelegate;

    unsafe impl NSObjectProtocol for NdSplitDelegate {}

    unsafe impl NSSplitViewDelegate for NdSplitDelegate {
        #[unsafe(method(splitView:constrainMinCoordinate:ofSubviewAt:))]
        fn constrain_min(&self, split: &NSSplitView, proposed: f64, _divider: NSInteger) -> f64 {
            let (lo, _) = bounds(split);
            proposed.max(lo)
        }

        #[unsafe(method(splitView:constrainMaxCoordinate:ofSubviewAt:))]
        fn constrain_max(&self, split: &NSSplitView, proposed: f64, _divider: NSInteger) -> f64 {
            let (_, hi) = bounds(split);
            proposed.min(hi)
        }

        // The split view changed size (the window, or nd's layout): the panes keep the ratio.
        #[unsafe(method(splitView:resizeSubviewsWithOldSize:))]
        fn resize_subviews(&self, split: &NSSplitView, _old: NSSize) {
            place(split, self.target());
        }

        #[unsafe(method(splitViewDidResizeSubviews:))]
        fn did_resize_subviews(&self, notification: &NSNotification) {
            let Some(split) = notification.object().and_then(|o| o.downcast::<NSSplitView>().ok()) else { return };
            (self.ivars().on_resize)();
            let Some(first) = first_length(&split) else { return };
            let target = self.target();
            if (first - position(&split, target)).abs() < SAME_POSITION {
                return;
            }
            let avail = available(&split);
            if avail <= 0. {
                return;
            }
            let ratio = (first / avail).clamp(0., 1.) as f32;
            self.ivars().ratio.borrow_mut().report(ratio);
            self.ivars().emit.emit(SplitResized { ratio });
            self.ivars().wake.wake();
        }
    }
);

impl NdSplitDelegate {
    fn new(mtm: MainThreadMarker, ivars: SplitIvars) -> Retained<Self> {
        unsafe { msg_send![super(Self::alloc(mtm).set_ivars(ivars)), init] }
    }

    fn target(&self) -> f32 {
        self.ivars().ratio.borrow().value().copied().unwrap_or(nd_components::split::DEFAULT_RATIO)
    }
}

/// The room the divider leaves along the split's axis.
fn available(split: &NSSplitView) -> f64 {
    let size = split.bounds().size;
    let total = if split.isVertical() { size.width } else { size.height };
    (total - split.dividerThickness()).max(0.)
}

fn panes(split: &NSSplitView) -> Vec<Retained<NSView>> {
    split.subviews().iter().collect()
}

/// The first pane's length along the axis.
fn first_length(split: &NSSplitView) -> Option<f64> {
    let first = panes(split).into_iter().next()?;
    let size = first.frame().size;
    Some(if split.isVertical() { size.width } else { size.height })
}

fn limits(view: Option<&Retained<NSView>>) -> (Option<f64>, Option<f64>) {
    view.and_then(|v| v.downcast_ref::<NdPaneView>()).map(NdPaneView::limits).unwrap_or_default()
}

/// The range the first pane's length may take: both panes' minimums and maximums.
fn bounds(split: &NSSplitView) -> (f64, f64) {
    let avail = available(split);
    let panes = panes(split);
    let (min1, max1) = limits(panes.first());
    let (min2, max2) = limits(panes.get(1));
    let lo = min1.unwrap_or(0.).max(max2.map_or(0., |m| avail - m));
    let hi = max1.unwrap_or(avail).min(avail - min2.unwrap_or(0.));
    (lo.min(avail).max(0.), hi.max(lo.min(avail)).max(0.))
}

/// The first pane's length `ratio` gives, held to the panes' limits.
fn position(split: &NSSplitView, ratio: f32) -> f64 {
    let (lo, hi) = bounds(split);
    (f64::from(ratio) * available(split)).clamp(lo, hi)
}

/// Frames both panes for `ratio`. Setting frames directly posts no divider move.
fn place(split: &NSSplitView, ratio: f32) {
    let size = split.bounds().size;
    let panes = panes(split);
    let vertical = split.isVertical();
    match panes.as_slice() {
        [only] => only.setFrame(NSRect::new(NSPoint::ZERO, size)),
        [first, second, ..] => {
            let d = split.dividerThickness();
            let a = position(split, ratio);
            let b = (available(split) - a).max(0.);
            let (r1, r2) = if vertical {
                (
                    NSRect::new(NSPoint::ZERO, NSSize::new(a, size.height)),
                    NSRect::new(NSPoint::new(a + d, 0.), NSSize::new(b, size.height)),
                )
            } else {
                (
                    NSRect::new(NSPoint::ZERO, NSSize::new(size.width, a)),
                    NSRect::new(NSPoint::new(0., a + d), NSSize::new(size.width, b)),
                )
            };
            first.setFrame(r1);
            second.setFrame(r2);
        }
        [] => {}
    }
}

pub(crate) struct SplitNative {
    pub view: Retained<NSSplitView>,
    delegate: Retained<NdSplitDelegate>,
    props: SplitProps,
}

impl SplitNative {
    pub fn new(mtm: MainThreadMarker, props: &SplitProps, emit: Emit, wake: Wake, on_resize: Hook) -> Self {
        let view = NSSplitView::new(mtm);
        view.setDividerStyle(NSSplitViewDividerStyle::Thin);
        view.setVertical(props.axis == Axis::Horizontal);
        let ivars = SplitIvars { ratio: Rc::default(), emit, wake, on_resize, placements: Cell::new(0) };
        let delegate = NdSplitDelegate::new(mtm, ivars);
        view.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        let this = Self { view, delegate, props: props.clone() };
        this.apply_ratio(props.ratio);
        this
    }

    fn apply_ratio(&self, ratio: f32) {
        let (view, delegate) = (&self.view, &self.delegate);
        delegate.ivars().ratio.borrow_mut().apply(&ratio, |r| {
            delegate.ivars().placements.set(delegate.ivars().placements.get() + 1);
            place(view, *r);
            *r
        });
    }

    pub fn update(&mut self, props: &SplitProps) {
        if props.axis != self.props.axis {
            self.view.setVertical(props.axis == Axis::Horizontal);
            place(&self.view, self.delegate.target());
        }
        self.apply_ratio(props.ratio);
        self.props = props.clone();
    }

    /// The first pane's share as the split view shows it.
    pub fn ratio(&self) -> Option<f32> {
        let (avail, first) = (available(&self.view), first_length(&self.view)?);
        (avail > 0.).then(|| (first / avail) as f32)
    }

    /// Times nd placed the panes for a ratio the app set.
    pub fn placements(&self) -> usize {
        self.delegate.ivars().placements.get()
    }

    /// `setValue`: moves the divider through the split view's own path, which reports the move
    /// as a drag does.
    pub fn perform(&self, action: &SemanticAction) -> Option<Result<(), ActionError>> {
        let response = self.props.respond(action)?;
        Some(response.map(|event| {
            if let Some(resized) = event.downcast_ref::<SplitResized>() {
                let at = f64::from(resized.ratio) * available(&self.view);
                self.view.setPosition_ofDividerAtIndex(at, 0);
            }
        }))
    }
}

/// One pane: the split view's arranged subview, holding the node's content.
pub(crate) struct SplitPaneNative {
    pub host: Retained<NdPaneView>,
    pub content: Retained<NdLayerView>,
}

impl SplitPaneNative {
    pub fn new(mtm: MainThreadMarker, props: &SplitPaneProps, expose: bool, on_resize: Hook) -> Self {
        let content = NdLayerView::new(mtm, &ViewLook::from_style(&props.style, expose));
        let host = NdPaneView::new(mtm, &content);
        host.set_on_resize(Some(on_resize));
        let this = Self { host, content };
        this.update(props, expose);
        this
    }

    pub fn update(&self, props: &SplitPaneProps, expose: bool) {
        self.content.set_props(&ViewLook::from_style(&props.style, expose));
        self.host.set_limits((props.min.map(f64::from), props.max.map(f64::from)));
    }

    /// The size the split view gives the pane: nd lays the content out across all of it.
    pub fn boundary(&self) -> Size {
        let s = self.host.bounds().size;
        Size { width: s.width.max(0.) as f32, height: s.height.max(0.) as f32 }
    }
}
