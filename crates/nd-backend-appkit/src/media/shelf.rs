//! `shelf` on AppKit: an `NSScrollView` that scrolls sideways only, its document holding the
//! node's children at the frames nd lays them out at, with circular system buttons over the
//! edges to page it. A scroll that settles off a card's edge glides to the nearest one.

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
use objc2_app_kit::{
    NSAnimatablePropertyContainer, NSAnimationContext, NSBezelStyle, NSButton, NSScrollElasticity, NSScrollView,
    NSView, NSViewBoundsDidChangeNotification,
};
use objc2_foundation::{NSNotification, NSNotificationCenter, NSObjectProtocol, NSPoint, NSRect, NSSize, NSTimer};

use super::{Input, NdMediaView, reduce_motion};
use crate::custom::{CustomCx, CustomNative, CustomWidget, Waker};

/// How long a scroll must be still before it snaps.
const SETTLE: f64 = 0.18;

pub(crate) struct ShelfWidget;

impl CustomWidget for ShelfWidget {
    fn create(&self, cx: &CustomCx<'_>) -> Box<dyn CustomNative> {
        let mtm = cx.mtm();
        let props = props(cx).clone();
        let view = NdMediaView::new(mtm);
        let scroll = NSScrollView::new(mtm);
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
            observer: RefCell::new(None),
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
    observer: RefCell<Option<Retained<ProtocolObject<dyn NSObjectProtocol>>>>,
    targets: RefCell<Vec<Retained<crate::forms::NdControlTarget>>>,
    emit: Emit,
    wake: Waker,
}

impl Drop for ShelfInner {
    fn drop(&mut self) {
        if let Some(o) = self.observer.get_mut().take() {
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
        *self.observer.borrow_mut() = Some(observer);
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
        if self.programmatic.get() || !self.props.borrow().snap {
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
