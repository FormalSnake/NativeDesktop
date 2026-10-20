//! Row swipe actions on a list's table, drawn by nd. `NSTableViewRowAction` narrows its button
//! from the trailing edge while the row closes, cutting off the icon and title; here the actions
//! slide with the row, whole, under nd-gesture's row swipe: the row follows the fingers, rubber
//! bands past its actions, settles on a spring that carries the release velocity, and ticks a
//! haptic when it crosses the full-swipe threshold.
//!
//! The row view's bounds origin moves the row's cells; the actions sit beside them in the row
//! view, so they move as one.

use std::cell::{Cell, OnceCell, RefCell};
use std::rc::Rc;

use nd_gesture::{RowSwipe, RowSwipeConfig};
use nd_virtual::{ROW_ACTION_WIDTH, RowActionInvoked, RowActionSpec, RowActionsSpec, RowEdge, RowId};
use objc2::Message;
use objc2::rc::{Retained, Weak};
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSColor, NSEvent, NSEventPhase, NSFont, NSFontWeightRegular, NSImageScaling, NSImageSymbolConfiguration,
    NSImageView, NSResponder, NSTableRowView, NSTableView, NSTextAlignment, NSTextField, NSView,
};
use objc2_foundation::{MainThreadMarker, NSInteger, NSPoint, NSRect, NSSize, NSString};
use objc2_quartz_core::CACurrentMediaTime;

use super::Shared;
use crate::frame_clock::FrameClock;

/// Where the current scroll gesture over the table is going.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Gesture {
    #[default]
    Idle,
    /// Fingers down, direction not known yet; the began event is held.
    Pending,
    /// A horizontal swipe on a row with actions.
    Swiping,
    /// Vertical, or a row without actions: the scroll view's.
    Passing,
    /// The swipe ended; its momentum events are dropped, the spring settles the row.
    Momentum,
}

struct Swipe {
    row: RowId,
    spec: RowActionsSpec,
    motion: RowSwipe,
    row_view: Retained<NSTableRowView>,
    /// The revealed edge's actions, one slot each.
    actions: Retained<NSView>,
    built: Option<RowEdge>,
}

thread_local! {
    /// The table with a row swiped open (or settling), for clicks anywhere in its window.
    static OPEN: RefCell<Option<Weak<NdTableView>>> = const { RefCell::new(None) };
}

/// A press in `window` while a row is swiped open: one on an action runs it, any other closes the
/// row. True when the press was on the table, which then never sees it; a press elsewhere goes
/// on as usual. The window takes it first because the row's own views keep their presses.
pub(crate) fn window_press(window: &objc2_app_kit::NSWindow, event: &NSEvent) -> bool {
    let Some(table) = OPEN.with_borrow(|o| o.as_ref().and_then(Weak::load)) else { return false };
    if table.window().is_none_or(|w| !std::ptr::eq(&*w, window)) {
        return false;
    }
    table.swipe_click(event);
    let point = table.convertPoint_fromView(event.locationInWindow(), None);
    table.mouse_inRect(point, table.visibleRect())
}

pub struct TableIvars {
    shared: OnceCell<Rc<Shared>>,
    gesture: Cell<Gesture>,
    began: RefCell<Option<Retained<NSEvent>>>,
    swipe: RefCell<Option<Swipe>>,
    clock: OnceCell<FrameClock>,
}

define_class!(
    // SAFETY: NSTableView has no subclassing requirements beyond calling super, and this type
    // does not implement Drop.
    #[unsafe(super(NSTableView, objc2_app_kit::NSControl, NSView, NSResponder, objc2_foundation::NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = TableIvars]
    pub struct NdTableView;

    impl NdTableView {
        #[unsafe(method(scrollWheel:))]
        fn scroll_wheel(&self, event: &NSEvent) {
            if !self.swipe_scroll(event) {
                let _: () = unsafe { msg_send![super(self), scrollWheel: event] };
            }
        }
    }
);

impl NdTableView {
    pub(super) fn new(mtm: MainThreadMarker, shared: Rc<Shared>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(TableIvars {
            shared: OnceCell::from(shared),
            gesture: Cell::default(),
            began: RefCell::default(),
            swipe: RefCell::default(),
            clock: OnceCell::new(),
        });
        unsafe { msg_send![super(this), init] }
    }

    fn shared(&self) -> &Shared {
        self.ivars().shared.get().expect("set in new")
    }

    fn pass(&self, event: &NSEvent) {
        let _: () = unsafe { msg_send![super(self), scrollWheel: event] };
    }

    /// A scroll event while row swipes are possible. False hands it to the table as usual.
    fn swipe_scroll(&self, event: &NSEvent) -> bool {
        let ivars = self.ivars();
        let phase = event.phase();
        if event.momentumPhase() != NSEventPhase::empty() {
            if ivars.gesture.get() != Gesture::Momentum {
                return false;
            }
            if event.momentumPhase().contains(NSEventPhase::Ended) {
                ivars.gesture.set(Gesture::Idle);
            }
            return true;
        }
        if phase.contains(NSEventPhase::MayBegin) {
            return false;
        }
        if phase.contains(NSEventPhase::Began) {
            if !event.hasPreciseScrollingDeltas() || self.shared().actions.borrow().is_empty() {
                ivars.gesture.set(Gesture::Passing);
                return false;
            }
            ivars.gesture.set(Gesture::Pending);
            *ivars.began.borrow_mut() = Some(event.retain());
            return self.decide(event);
        }
        let ended = phase.contains(NSEventPhase::Ended) || phase.contains(NSEventPhase::Cancelled);
        match ivars.gesture.get() {
            Gesture::Pending if ended => {
                ivars.gesture.set(Gesture::Idle);
                if let Some(began) = ivars.began.borrow_mut().take() {
                    self.pass(&began);
                }
                false
            }
            Gesture::Pending => self.decide(event),
            Gesture::Swiping if ended => {
                ivars.gesture.set(Gesture::Momentum);
                self.release(event.timestamp());
                true
            }
            Gesture::Swiping => {
                let dx = event.scrollingDeltaX() * if event.isDirectionInvertedFromDevice() { 1. } else { -1. };
                self.track(dx as f32, event.timestamp());
                true
            }
            Gesture::Momentum => true,
            Gesture::Idle | Gesture::Passing => false,
        }
    }

    /// The first moving event of a gesture picks its owner: a mostly horizontal one on a row
    /// with actions swipes that row, anything else scrolls (and closes an open row).
    fn decide(&self, event: &NSEvent) -> bool {
        let ivars = self.ivars();
        let (dx, dy) = (event.scrollingDeltaX(), event.scrollingDeltaY());
        if dx == 0. && dy == 0. {
            return true;
        }
        let began = ivars.began.borrow_mut().take();
        let target = (dx.abs() > dy.abs()).then(|| self.row_under(event)).flatten();
        let open = ivars.swipe.borrow().as_ref().map(|s| s.row);
        match (target, open) {
            (Some((row, _)), Some(open)) if row != open => {
                // A swipe on another row while one is open only closes that one.
                self.close();
                ivars.gesture.set(Gesture::Momentum);
                true
            }
            (Some((row, index)), _) => {
                ivars.gesture.set(Gesture::Swiping);
                self.start(row, index, event.timestamp());
                let dx = dx * if event.isDirectionInvertedFromDevice() { 1. } else { -1. };
                self.track(dx as f32, event.timestamp());
                true
            }
            (None, _) => {
                ivars.gesture.set(Gesture::Passing);
                if open.is_some() {
                    self.close();
                }
                if let Some(began) = began.filter(|b| !std::ptr::eq(&**b, event)) {
                    self.pass(&began);
                }
                false
            }
        }
    }

    fn row_under(&self, event: &NSEvent) -> Option<(RowId, NSInteger)> {
        let point = self.convertPoint_fromView(event.locationInWindow(), None);
        let index = self.rowAtPoint(point);
        let shared = self.shared();
        let row = *shared.snap.borrow().ids.get(usize::try_from(index).ok()?)?;
        let has = shared.actions.borrow().get(&row).is_some_and(|a| !a.leading.is_empty() || !a.trailing.is_empty());
        has.then_some((row, index))
    }

    fn start(&self, row: RowId, index: NSInteger, time: f64) {
        let mut swipe = self.ivars().swipe.borrow_mut();
        if swipe.as_ref().is_none_or(|s| s.row != row) {
            let Some(row_view) = self.rowViewAtRow_makeIfNecessary(index, false) else { return };
            let spec = self.shared().actions.borrow().get(&row).cloned().unwrap_or_default();
            let width = |actions: &[RowActionSpec]| actions.len() as f32 * ROW_ACTION_WIDTH;
            let config = RowSwipeConfig {
                leading: width(&spec.leading),
                trailing: width(&spec.trailing),
                full_swipe: spec.full_swipe,
            };
            let motion = RowSwipe::new(config, row_view.bounds().size.width as f32);
            let actions = NSView::new(self.mtm());
            row_view.addSubview(&actions);
            OPEN.set(Some(Weak::from(self)));
            *swipe = Some(Swipe { row, spec, motion, row_view, actions, built: None });
        }
        if let Some(s) = swipe.as_mut() {
            s.motion.begin(time);
        }
    }

    fn track(&self, finger_dx: f32, time: f64) {
        let haptic = {
            let mut swipe = self.ivars().swipe.borrow_mut();
            let Some(s) = swipe.as_mut() else { return };
            let (_, haptic) = s.motion.update(finger_dx, time);
            self.apply(s);
            haptic
        };
        if let Some(h) = haptic {
            crate::gestures::play(h);
        }
    }

    /// Fingers lifted: the row settles open, closed, or runs its full-swipe action and closes.
    fn release(&self, time: f64) {
        let invoked = {
            let mut swipe = self.ivars().swipe.borrow_mut();
            let Some(s) = swipe.as_mut() else { return };
            let event = s.motion.end(time);
            let invoked = event.full.then_some(event.open).flatten().map(|edge| (s.row, edge));
            if invoked.is_some() {
                s.motion.close(time);
            }
            invoked
        };
        if let Some((row, edge)) = invoked {
            self.invoke(row, edge, 0);
        }
        self.animate();
    }

    fn invoke(&self, row: RowId, edge: RowEdge, index: usize) {
        let shared = self.shared();
        shared.emit.emit(RowActionInvoked { row, edge, index });
        shared.wake.wake();
    }

    /// Slides an open row closed.
    pub(super) fn close(&self) {
        if let Some(s) = self.ivars().swipe.borrow_mut().as_mut() {
            s.motion.close(CACurrentMediaTime());
        }
        self.animate();
    }

    /// Puts an open row back at once, for a row going away.
    pub(super) fn forget(&self, row: RowId) {
        let mut swipe = self.ivars().swipe.borrow_mut();
        if swipe.as_ref().is_some_and(|s| s.row == row)
            && let Some(s) = swipe.take()
        {
            Self::reset(&s);
        }
    }

    fn reset(s: &Swipe) {
        OPEN.set(None);
        s.actions.removeFromSuperview();
        s.row_view.setBoundsOrigin(NSPoint::new(0., 0.));
    }

    /// A click while a row is open runs the action under it, and closes the row.
    fn swipe_click(&self, event: &NSEvent) {
        let hit = {
            let swipe = self.ivars().swipe.borrow();
            let Some(s) = swipe.as_ref() else { return };
            if s.motion.open_edge().is_none() {
                drop(swipe);
                self.close();
                return;
            }
            let point = s.actions.convertPoint_fromView(event.locationInWindow(), None);
            let slots = s.actions.subviews();
            let index = slots.iter().position(|slot| {
                let f = slot.frame();
                point.x >= f.origin.x
                    && point.x < f.origin.x + f.size.width
                    && point.y >= f.origin.y
                    && point.y < f.origin.y + f.size.height
            });
            s.built.zip(index).map(|(edge, index)| (s.row, edge, index))
        };
        if let Some((row, edge, index)) = hit {
            self.invoke(row, edge, index);
        }
        self.close();
    }

    fn animate(&self) {
        let clock = self.ivars().clock.get_or_init(|| {
            let clock = FrameClock::new(self.mtm(), self, false);
            let weak = Weak::from(self);
            clock.on_frame(move |tick| {
                if let Some(table) = weak.load() {
                    table.step(tick.target_timestamp);
                }
            });
            clock
        });
        clock.set_animating(true);
    }

    fn step(&self, time: f64) {
        let ivars = self.ivars();
        let mut swipe = ivars.swipe.borrow_mut();
        let Some(s) = swipe.as_mut() else {
            drop(swipe);
            ivars.clock.get().inspect(|c| c.set_animating(false));
            return;
        };
        if s.motion.step(time).is_some() {
            self.apply(s);
        }
        if s.motion.is_animating() {
            return;
        }
        if s.motion.open_edge().is_none()
            && let Some(s) = swipe.take()
        {
            Self::reset(&s);
        }
        drop(swipe);
        ivars.clock.get().inspect(|c| c.set_animating(false));
    }

    /// Moves the row's cells by the swipe's offset and lays the revealed actions out beside them,
    /// each at its full width, so they slide rather than shrink.
    fn apply(&self, s: &mut Swipe) {
        let offset = f64::from(s.motion.offset());
        s.row_view.setBoundsOrigin(NSPoint::new(-offset, 0.));
        let edge = if offset > 0. {
            Some(RowEdge::Leading)
        } else if offset < 0. {
            Some(RowEdge::Trailing)
        } else {
            None
        };
        if edge != s.built {
            for v in s.actions.subviews().iter() {
                v.removeFromSuperview();
            }
            let specs = match edge {
                Some(RowEdge::Leading) => s.spec.leading.as_slice(),
                Some(RowEdge::Trailing) => s.spec.trailing.as_slice(),
                None => &[],
            };
            for spec in specs {
                s.actions.addSubview(&slot(self.mtm(), spec));
            }
            s.built = edge;
        }
        let Some(edge) = edge else { return };
        let bounds = s.row_view.bounds().size;
        let slots = s.actions.subviews();
        let n = slots.len().max(1) as f64;
        let w = f64::from(ROW_ACTION_WIDTH);
        let reveal = (n * w).max(offset.abs());
        let x = if edge == RowEdge::Leading { -reveal } else { bounds.width };
        s.actions.setFrame(NSRect::new(NSPoint::new(x, 0.), NSSize::new(reveal, bounds.height)));
        let each = reveal / n;
        for (i, v) in slots.iter().enumerate() {
            // Actions run in order from the row's edge.
            let at = if edge == RowEdge::Leading { i as f64 * each } else { reveal - (i as f64 + 1.) * each };
            v.setFrame(NSRect::new(NSPoint::new(at, 0.), NSSize::new(each, bounds.height)));
            // The icon and title keep beside the row's content as a full swipe widens the slot.
            let inner = if edge == RowEdge::Leading { each - w } else { 0. };
            let content = v.subviews();
            let mid = bounds.height / 2.;
            let flipped = v.isFlipped();
            for c in content.iter() {
                let size = c.frame().size;
                let centre_x = inner + w / 2.;
                // Icon above the title; a title alone sits in the middle.
                let dy = match (content.len(), c.downcast_ref::<NSImageView>().is_some()) {
                    (1, _) => 0.,
                    (_, true) => -10.,
                    _ => 11.,
                };
                let y = if flipped { mid + dy - size.height / 2. } else { mid - dy - size.height / 2. };
                c.setFrameOrigin(NSPoint::new(centre_x - size.width / 2., y));
            }
        }
    }
}

/// One action's slot: its fill, its icon over its title.
fn slot(mtm: MainThreadMarker, spec: &RowActionSpec) -> Retained<NSView> {
    let view = NSView::new(mtm);
    view.setWantsLayer(true);
    let clear = spec.tint.is_some_and(|t| t.a < 0.01);
    let fill = match spec.tint {
        Some(t) => {
            let c = t.to_rgb();
            NSColor::colorWithSRGBRed_green_blue_alpha(c.r.into(), c.g.into(), c.b.into(), c.a.into())
        }
        None if spec.destructive => NSColor::systemRedColor(),
        None => NSColor::systemGrayColor(),
    };
    if let Some(layer) = view.layer() {
        layer.setBackgroundColor(Some(&fill.CGColor()));
    }
    let ink = if clear { NSColor::secondaryLabelColor() } else { NSColor::whiteColor() };
    let label = NSTextField::labelWithString(&NSString::from_str(&spec.title), mtm);
    label.setFont(Some(&NSFont::systemFontOfSize(NSFont::smallSystemFontSize())));
    label.setTextColor(Some(&ink));
    label.setAlignment(NSTextAlignment::Center);
    label.sizeToFit();
    view.addSubview(&label);
    if let Some(image) = spec.icon.as_deref().and_then(crate::icons::symbol) {
        let config = NSImageSymbolConfiguration::configurationWithPointSize_weight(NSFont::systemFontSize(), unsafe {
            NSFontWeightRegular
        });
        let image = image.imageWithSymbolConfiguration(&config).unwrap_or(image);
        let icon = NSImageView::imageViewWithImage(&image, mtm);
        icon.setImageScaling(NSImageScaling::ScaleNone);
        icon.setContentTintColor(Some(&ink));
        icon.sizeToFit();
        view.addSubview(&icon);
    }
    view
}
