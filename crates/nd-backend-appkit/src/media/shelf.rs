//! `shelf` on AppKit: an `NSScrollView` that scrolls sideways only, its document holding the
//! node's children at the frames nd lays them out at, with circular system buttons over the
//! edges to page it. A scroll that settles off a card's edge glides to the nearest one, once the
//! fingers and their momentum are done.
//!
//! A shelf sits in a page that scrolls the other way. AppKit latches a trackpad gesture to the
//! scroll view under the pointer when it starts, and a sideways-only scroll view keeps a vertical
//! gesture, so the page stalls and stutters whenever a shelf passes under the pointer. A gesture
//! that starts mostly vertical goes to the enclosing scroll view whole, momentum included.

use std::cell::{Cell, RefCell};
use std::ptr::NonNull;
use std::rc::{Rc, Weak};

use block2::RcBlock;
use nd_components::media::{ShelfArrows, ShelfProps, ShelfScrolled, scroll_target, snap_offset};
use nd_core::Rect;
use nd_theme::Theme;
use nd_ui::events::Emit;
use nd_ui::semantics::{ActionError, SemanticAction};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSAnimatablePropertyContainer, NSAnimationContext, NSBezelStyle, NSButton, NSEvent, NSEventPhase, NSResponder,
    NSScrollElasticity, NSScrollView, NSScrollViewDidEndLiveScrollNotification,
    NSScrollViewWillStartLiveScrollNotification, NSView, NSViewBoundsDidChangeNotification,
};
use objc2_foundation::{
    MainThreadMarker, NSNotification, NSNotificationCenter, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize,
    NSTimer,
};

use super::{Input, NdMediaView, reduce_motion};
use crate::custom::{CustomCx, CustomNative, CustomWidget, Waker};

/// How long a mouse wheel scroll must be still before it snaps. A trackpad scroll snaps when its
/// live scroll ends.
const SETTLE: f64 = 0.18;

#[derive(Default)]
pub struct ShelfScrollIvars {
    /// The gesture under way started vertical and belongs to the page.
    passing: Cell<bool>,
}

define_class!(
    // SAFETY: NSScrollView has no subclassing requirements beyond calling super, and this type
    // does not implement Drop.
    #[unsafe(super(NSScrollView, NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = ShelfScrollIvars]
    pub struct NdShelfScrollView;

    impl NdShelfScrollView {
        #[unsafe(method(scrollWheel:))]
        fn scroll_wheel(&self, event: &NSEvent) {
            if self.passes(event) {
                if let Some(next) = unsafe { self.nextResponder() } {
                    let _: () = unsafe { msg_send![&*next, scrollWheel: event] };
                }
            } else {
                let _: () = unsafe { msg_send![super(self), scrollWheel: event] };
            }
        }
    }
);

impl NdShelfScrollView {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ShelfScrollIvars::default());
        unsafe { msg_send![super(this), init] }
    }

    /// Whether `event` belongs to the page: every event of a gesture that began mostly vertical,
    /// through its momentum, and a mouse wheel turned without a sideways component.
    fn passes(&self, event: &NSEvent) -> bool {
        let passing = &self.ivars().passing;
        let (phase, momentum) = (event.phase(), event.momentumPhase());
        let (dx, dy) = (event.scrollingDeltaX().abs(), event.scrollingDeltaY().abs());
        if phase.contains(NSEventPhase::MayBegin) || phase.contains(NSEventPhase::Began) {
            passing.set(dy > dx);
            if phase.contains(NSEventPhase::MayBegin) && dx == 0. && dy == 0. {
                // Fingers resting, direction unknown: the first moving event decides.
                passing.set(false);
                return false;
            }
            return passing.get();
        }
        if phase.is_empty() && momentum.is_empty() {
            return dx == 0. && dy > 0.;
        }
        let pass = passing.get();
        if momentum.contains(NSEventPhase::Ended) || momentum.contains(NSEventPhase::Cancelled) {
            passing.set(false);
        }
        pass
    }
}

pub(crate) struct ShelfWidget;

impl CustomWidget for ShelfWidget {
    fn create(&self, cx: &CustomCx<'_>) -> Box<dyn CustomNative> {
        let mtm = cx.mtm();
        let props = props(cx).clone();
        let view = NdMediaView::new(mtm);
        let scroll = Retained::into_super(NdShelfScrollView::new(mtm));
        scroll.setDrawsBackground(false);
        scroll.setHasHorizontalScroller(true);
        scroll.setHasVerticalScroller(false);
        scroll.setAutohidesScrollers(true);
        scroll.setVerticalScrollElasticity(NSScrollElasticity::None);
        scroll.setHorizontalScrollElasticity(NSScrollElasticity::Automatic);
        scroll.setAutomaticallyAdjustsContentInsets(false);
        let document = NdMediaView::new(mtm);
        scroll.setDocumentView(Some(&document));
        view.addSubview(&scroll);
        let arrow = |icon: &str, tip: &str| {
            let b = NSButton::new(mtm);
            b.setBezelStyle(NSBezelStyle::Circular);
            b.setImage(crate::icons::symbol(icon).as_deref());
            b.setToolTip(Some(&objc2_foundation::NSString::from_str(tip)));
            b.setHidden(true);
            view.addSubview(&b);
            b
        };
        let back = arrow("chevron-left", "Back");
        let forward = arrow("chevron-right", "Forward");
        let inner = Rc::new(ShelfInner {
            view,
            scroll,
            document,
            back,
            forward,
            props: RefCell::new(props),
            hovered: Cell::new(false),
            programmatic: Rc::new(Cell::new(false)),
            flags: Cell::new(None),
            settle: RefCell::new(None),
            live: Cell::new(false),
            observer: RefCell::new(Vec::new()),
            targets: RefCell::new(Vec::new()),
            emit: cx.emitter(),
            wake: cx.waker(),
        });
        inner.view.set_input(Rc::downgrade(&inner) as Weak<dyn Input>);
        inner.wire(cx);
        Box::new(ShelfNative { inner })
    }
}

fn props<'a>(cx: &'a CustomCx<'_>) -> &'a ShelfProps {
    cx.props().downcast_ref::<ShelfProps>().expect("shelf node needs ShelfProps")
}

struct ShelfInner {
    view: Retained<NdMediaView>,
    scroll: Retained<NSScrollView>,
    document: Retained<NdMediaView>,
    back: Retained<NSButton>,
    forward: Retained<NSButton>,
    props: RefCell<ShelfProps>,
    hovered: Cell<bool>,
    programmatic: Rc<Cell<bool>>,
    /// The paging flags last reported, so `ShelfScrolled` fires only when they change.
    flags: Cell<Option<(bool, bool)>>,
    settle: RefCell<Option<Retained<NSTimer>>>,
    /// The user's fingers or their momentum are moving the shelf.
    live: Cell<bool>,
    observer: RefCell<Vec<Retained<ProtocolObject<dyn NSObjectProtocol>>>>,
    targets: RefCell<Vec<Retained<crate::forms::NdControlTarget>>>,
    emit: Emit,
    wake: Waker,
}

impl Drop for ShelfInner {
    fn drop(&mut self) {
        for o in self.observer.get_mut().drain(..) {
            unsafe { NSNotificationCenter::defaultCenter().removeObserver(o.as_ref()) };
        }
        if let Some(t) = self.settle.get_mut().take() {
            t.invalidate();
        }
    }
}

impl ShelfInner {
    fn wire(self: &Rc<Self>, cx: &CustomCx<'_>) {
        let clip = self.scroll.contentView();
        clip.setPostsBoundsChangedNotifications(true);
        let me = Rc::downgrade(self);
        let moved = RcBlock::new(move |_n: NonNull<NSNotification>| {
            if let Some(me) = me.upgrade() {
                me.moved();
            }
        });
        let clip_obj: &AnyObject = &clip;
        let observer = unsafe {
            NSNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(
                Some(NSViewBoundsDidChangeNotification),
                Some(clip_obj),
                None,
                &moved,
            )
        };
        let scroll_obj: &AnyObject = &self.scroll;
        let me = Rc::downgrade(self);
        let started = RcBlock::new(move |_n: NonNull<NSNotification>| {
            if let Some(me) = me.upgrade() {
                me.live.set(true);
                if let Some(t) = me.settle.borrow_mut().take() {
                    t.invalidate();
                }
            }
        });
        let me = Rc::downgrade(self);
        let ended = RcBlock::new(move |_n: NonNull<NSNotification>| {
            if let Some(me) = me.upgrade() {
                me.live.set(false);
                if me.props.borrow().snap {
                    me.snap();
                }
            }
        });
        let center = NSNotificationCenter::defaultCenter();
        let live = unsafe {
            [
                center.addObserverForName_object_queue_usingBlock(
                    Some(NSScrollViewWillStartLiveScrollNotification),
                    Some(scroll_obj),
                    None,
                    &started,
                ),
                center.addObserverForName_object_queue_usingBlock(
                    Some(NSScrollViewDidEndLiveScrollNotification),
                    Some(scroll_obj),
                    None,
                    &ended,
                ),
            ]
        };
        let mut observers = self.observer.borrow_mut();
        observers.push(observer);
        observers.extend(live);
        let mut targets = Vec::new();
        for (button, direction) in [(&self.back, -1), (&self.forward, 1)] {
            let me = Rc::downgrade(self);
            let target = crate::forms::NdControlTarget::new(cx.mtm(), move |_| {
                if let Some(me) = me.upgrade() {
                    me.page(direction);
                }
            });
            target.wire(button);
            targets.push(target);
        }
        *self.targets.borrow_mut() = targets;
    }

    fn offset(&self) -> f64 {
        self.scroll.contentView().bounds().origin.x
    }

    fn max_offset(&self) -> f64 {
        (self.document.frame().size.width - self.scroll.contentSize().width).max(0.)
    }

    fn cards(&self) -> Vec<Rect> {
        let mut cards: Vec<Rect> = self
            .document
            .subviews()
            .iter()
            .map(|v| {
                let f = v.frame();
                Rect::new(f.origin.x as f32, f.origin.y as f32, f.size.width as f32, f.size.height as f32)
            })
            .collect();
        cards.sort_by(|a, b| a.origin.x.total_cmp(&b.origin.x));
        cards
    }

    fn inset(&self, cards: &[Rect]) -> f32 {
        cards.first().map_or(0., |c| c.origin.x)
    }

    /// The clip view moved: report it, show the arrows that can page, and snap once it settles.
    fn moved(self: &Rc<Self>) {
        self.report();
        if self.programmatic.get() || self.live.get() || !self.props.borrow().snap {
            return;
        }
        if let Some(t) = self.settle.borrow_mut().take() {
            t.invalidate();
        }
        let me = Rc::downgrade(self);
        let timer = crate::events::after(SETTLE, move || {
            if let Some(me) = me.upgrade() {
                me.settle.borrow_mut().take();
                me.snap();
            }
        });
        *self.settle.borrow_mut() = Some(timer);
    }

    fn snap(&self) {
        let cards = self.cards();
        let max = self.max_offset() as f32;
        let target = snap_offset(self.offset() as f32, &cards, self.inset(&cards), max, 0);
        self.scroll_to(f64::from(target), true);
    }

    fn report(&self) {
        let (offset, max, viewport) = (self.offset(), self.max_offset(), self.scroll.contentSize().width);
        let handle = self.props.borrow().handle.clone();
        handle.report(offset as f32, max as f32, viewport as f32);
        let flags = (handle.can_page_back(), handle.can_page_forward());
        if self.flags.replace(Some(flags)) != Some(flags) {
            self.emit.emit(ShelfScrolled { offset: offset as f32, can_page_back: flags.0, can_page_forward: flags.1 });
            self.wake.wake();
        }
        self.arrows();
    }

    fn arrows(&self) {
        let props = self.props.borrow();
        let show = match props.arrows {
            ShelfArrows::Always => true,
            ShelfArrows::Hover => self.hovered.get(),
            ShelfArrows::Never => false,
        };
        let handle = &props.handle;
        self.back.setHidden(!(show && handle.can_page_back()));
        self.forward.setHidden(!(show && handle.can_page_forward()));
    }

    fn page(&self, direction: i32) {
        self.apply(nd_components::media::ShelfScroll::Page(direction));
    }

    fn apply(&self, request: nd_components::media::ShelfScroll) {
        let cards = self.cards();
        let target = scroll_target(
            request,
            self.offset() as f32,
            self.scroll.contentSize().width as f32,
            self.max_offset() as f32,
            &cards,
            self.inset(&cards),
        );
        self.scroll_to(f64::from(target), true);
    }

    fn scroll_to(&self, x: f64, animated: bool) {
        let clip = self.scroll.contentView();
        if (clip.bounds().origin.x - x).abs() < 0.5 {
            return;
        }
        let target = NSPoint::new(x, 0.);
        self.programmatic.set(true);
        if !animated || reduce_motion() {
            clip.setBoundsOrigin(target);
            self.scroll.reflectScrolledClipView(&clip);
            self.programmatic.set(false);
            self.report();
            return;
        }
        let scroll = self.scroll.clone();
        let changes = RcBlock::new(move |ctx: NonNull<NSAnimationContext>| {
            unsafe { ctx.as_ref() }.setDuration(0.3);
            let clip = scroll.contentView();
            clip.animator().setBoundsOrigin(target);
            scroll.reflectScrolledClipView(&clip);
        });
        let programmatic = self.programmatic.clone();
        let done = RcBlock::new(move || programmatic.set(false));
        NSAnimationContext::runAnimationGroup_completionHandler(&changes, Some(&done));
    }

    /// Sizes the document to the cards' extent plus the trailing padding the style gives.
    fn fit(&self) {
        let was = self.programmatic.replace(true);
        self.fit_inner();
        self.programmatic.set(was);
    }

    fn fit_inner(&self) {
        let b = self.view.bounds();
        if self.scroll.frame() != b {
            self.scroll.setFrame(b);
        }
        let trailing = self.props.borrow().style.padding.right.map_or(0., |p| match p {
            nd_ui::prelude::DefiniteLength::Absolute(a) => f64::from(crate::look::length(a)),
            nd_ui::prelude::DefiniteLength::Fraction(f) => b.size.width * f64::from(f),
        });
        let right =
            self.document.subviews().iter().map(|v| v.frame().origin.x + v.frame().size.width).fold(0., f64::max);
        let size = NSSize::new((right + trailing).max(b.size.width), b.size.height);
        if self.document.frame().size != size {
            self.document.setFrameSize(size);
        }
        let side = |button: &NSButton, leading: bool| {
            let s = button.fittingSize();
            let x = if leading { 8. } else { b.size.width - s.width - 8. };
            button.setFrame(NSRect::new(NSPoint::new(x, ((b.size.height - s.height) / 2.).round()), s));
        };
        side(&self.back, true);
        side(&self.forward, false);
    }
}

impl Input for ShelfInner {
    fn hover(&self, inside: bool, _at: NSPoint) {
        if self.hovered.replace(inside) != inside {
            self.arrows();
        }
    }

    fn layout(&self) {
        self.fit();
        self.report();
    }
}

struct ShelfNative {
    inner: Rc<ShelfInner>,
}

impl CustomNative for ShelfNative {
    fn view(&self) -> &NSView {
        &self.inner.view
    }

    fn container(&self) -> Option<&NSView> {
        Some(&self.inner.document)
    }

    fn update(&mut self, cx: &CustomCx<'_>) {
        *self.inner.props.borrow_mut() = props(cx).clone();
        self.inner.arrows();
    }

    fn set_theme(&mut self, _theme: &Theme) {}

    fn committed(&mut self) {
        self.inner.fit();
        let handle = self.inner.props.borrow().handle.clone();
        if let Some((seq, request)) = handle.pending() {
            handle.applied(seq);
            self.inner.apply(request);
        }
        self.inner.report();
    }

    fn perform(&self, action: &SemanticAction) -> Option<Result<(), ActionError>> {
        let SemanticAction::Scroll { dx, .. } = action else { return None };
        let x = (self.inner.offset() + f64::from(*dx)).clamp(0., self.inner.max_offset());
        self.inner.scroll_to(x, false);
        Some(Ok(()))
    }
}
