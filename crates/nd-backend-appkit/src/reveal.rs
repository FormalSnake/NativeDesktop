//! `Pane::edge_reveal()` on AppKit, a port of the old `NDShell/SplitReveal.swift`: while the
//! sidebar is collapsed, the pointer touching the window's leading edge slides the sidebar in as
//! a floating panel over the content, and it slides away again 250 ms after the pointer leaves.
//! The content pane is not resized, so a page under it does not relayout.
//!
//! The split item stays collapsed. What moves is the pane's nd content: it is lifted out of the
//! pane's host into the panel while shown and put back when the panel goes, and nd lays it out
//! at the panel's size meanwhile. Embedded traffic lights in the sidebar ride along: placed where
//! the panel lands, drawn along the same slide by an additive animation on their layers, and
//! hidden while the sidebar is.
//!
//! `ND_REVEAL_TRACE=1` prints the panel's and the close button's leading edges on every display
//! frame of a slide (`ND_REVEAL_FRAME` lines on stderr), from their presentation layers, so a
//! test can hold the two together frame by frame. `ND_ANIMATION_SLOWDOWN` stretches the slides.

use std::cell::{Cell, RefCell};
use std::ptr::NonNull;
use std::rc::{Rc, Weak};
use std::time::Instant;

use block2::RcBlock;
use nd_components::SidebarRevealed;
use nd_ui::events::Emit;
use objc2::rc::Retained;
use objc2::{AllocAnyThread, DefinedClass, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAnimatablePropertyContainer, NSAnimationContext, NSAutoresizingMaskOptions, NSColor, NSEvent, NSGlassEffectView,
    NSResponder, NSSplitViewItem, NSTrackingArea, NSTrackingAreaOptions, NSView, NSViewController,
    NSVisualEffectBlendingMode, NSVisualEffectMaterial, NSVisualEffectState, NSVisualEffectView, NSWindow,
    NSWindowOrderingMode,
};
use objc2_foundation::{
    MainThreadMarker, NSNumber, NSObject, NSPoint, NSRect, NSRunLoop, NSRunLoopCommonModes, NSSize, NSString, NSTimer,
};
use objc2_quartz_core::{CABasicAnimation, CADisplayLink, CAMediaTiming, CAMediaTimingFunction, kCAFillModeBoth};

use crate::events::Wake;
use crate::scaffold::Hook;

/// How close to the leading edge the pointer has to come. Narrow enough that it never sits
/// over anything the content draws.
const EDGE: f64 = nd_components::REVEAL_EDGE as f64;
/// The panel's gap to the window edges, matching the floating sidebar's own.
const INSET: f64 = 8.; // nd-lint: allow, AppKit exposes no floating sidebar inset to read
/// Concentric with the window's 16 pt corner at the inset.
const RADIUS: f64 = 16.; // nd-lint: allow, AppKit exposes no window corner radius to read
const IN: f64 = 0.2;
const OUT: f64 = 0.15;
/// Grace before a pointer that left the panel hides it: crossing a gap between two controls
/// or overshooting the edge should not flicker it.
const LEAVE: f64 = 0.25;
const CURVE: [f32; 4] = [0.23, 1., 0.32, 1.];
const RIDE: &str = "nd-ride";

fn slowdown() -> f64 {
    std::env::var("ND_ANIMATION_SLOWDOWN").ok().and_then(|v| v.parse::<f64>().ok()).filter(|v| *v >= 1.).unwrap_or(1.)
}

fn reduce_motion() -> bool {
    crate::media::reduce_motion()
}

fn curve() -> Retained<CAMediaTimingFunction> {
    let [a, b, c, d] = CURVE;
    CAMediaTimingFunction::functionWithControlPoints(a, b, c, d)
}

pub struct HookIvars {
    reveal: RefCell<Weak<Reveal>>,
}

fn track(view: &NSView, owner: &NSObject) {
    for area in view.trackingAreas() {
        view.removeTrackingArea(&area);
    }
    let options = NSTrackingAreaOptions::MouseEnteredAndExited
        | NSTrackingAreaOptions::ActiveInKeyWindow
        | NSTrackingAreaOptions::InVisibleRect;
    let area = unsafe {
        NSTrackingArea::initWithRect_options_owner_userInfo(
            NSTrackingArea::alloc(),
            NSRect::ZERO,
            options,
            Some(owner),
            None,
        )
    };
    view.addTrackingArea(&area);
}

define_class!(
    // SAFETY: NSView has no subclassing requirements beyond calling super in the overrides
    // below, and this type does not implement Drop.
    #[unsafe(super(NSView, NSResponder))]
    #[thread_kind = MainThreadOnly]
    #[ivars = HookIvars]
    pub struct NdRevealEdge;

    impl NdRevealEdge {
        #[unsafe(method(updateTrackingAreas))]
        fn update_tracking_areas(&self) {
            let _: () = unsafe { msg_send![super(self), updateTrackingAreas] };
            track(self, self);
        }

        #[unsafe(method(mouseEntered:))]
        fn mouse_entered(&self, _event: &NSEvent) {
            if let Some(reveal) = self.ivars().reveal.borrow().upgrade() {
                reveal.reveal(true);
            }
        }

        // Off the strip without reaching the panel hides it as leaving the panel does.
        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, _event: &NSEvent) {
            if let Some(reveal) = self.ivars().reveal.borrow().upgrade() {
                reveal.schedule_hide();
            }
        }

        // Transparent to clicks: only the pointer's presence matters here.
        #[unsafe(method(hitTest:))]
        fn hit_test(&self, _point: NSPoint) -> *mut NSView {
            std::ptr::null_mut()
        }
    }
);

define_class!(
    // SAFETY: as above.
    #[unsafe(super(NSView, NSResponder))]
    #[thread_kind = MainThreadOnly]
    #[ivars = HookIvars]
    pub struct NdRevealPanel;

    impl NdRevealPanel {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(updateTrackingAreas))]
        fn update_tracking_areas(&self) {
            let _: () = unsafe { msg_send![super(self), updateTrackingAreas] };
            track(self, self);
        }

        #[unsafe(method(mouseEntered:))]
        fn mouse_entered(&self, _event: &NSEvent) {
            if let Some(reveal) = self.ivars().reveal.borrow().upgrade() {
                reveal.cancel_hide();
            }
        }

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, _event: &NSEvent) {
            if let Some(reveal) = self.ivars().reveal.borrow().upgrade() {
                reveal.schedule_hide();
            }
        }
    }
);

fn hook(reveal: &Rc<Reveal>) -> HookIvars {
    HookIvars { reveal: RefCell::new(Rc::downgrade(reveal)) }
}

impl NdRevealEdge {
    fn new(mtm: MainThreadMarker, reveal: &Rc<Reveal>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(hook(reveal));
        unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] }
    }
}

impl NdRevealPanel {
    fn new(mtm: MainThreadMarker, reveal: &Rc<Reveal>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(hook(reveal));
        unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] }
    }
}

/// What the reveal needs from the backend.
pub(crate) struct RevealHooks {
    pub emit: Emit,
    pub wake: Wake,
    /// Lays the pane's content out again (at the panel's size while revealed).
    pub relayout: Hook,
    /// Places embedded traffic lights on their slots now.
    pub place_lights: Hook,
}

struct Shown {
    panel: Retained<NdRevealPanel>,
}

pub(crate) struct Reveal {
    mtm: MainThreadMarker,
    item: Retained<NSSplitViewItem>,
    controller: Retained<NSViewController>,
    host: Retained<NSView>,
    content: Retained<NSView>,
    hooks: RevealHooks,
    enabled: Cell<bool>,
    edge: RefCell<Option<Retained<NdRevealEdge>>>,
    shown: RefCell<Option<Shown>>,
    revealed: Cell<bool>,
    hide: RefCell<Option<Retained<NSTimer>>>,
    /// The sidebar's width the last time it was open: the panel comes in at it.
    width: Cell<f64>,
    min_width: Cell<f64>,
}

impl Reveal {
    pub fn new(
        mtm: MainThreadMarker,
        item: Retained<NSSplitViewItem>,
        controller: Retained<NSViewController>,
        host: Retained<NSView>,
        content: Retained<NSView>,
        hooks: RevealHooks,
    ) -> Rc<Self> {
        Rc::new(Self {
            mtm,
            item,
            controller,
            host,
            content,
            hooks,
            enabled: Cell::new(false),
            edge: RefCell::default(),
            shown: RefCell::default(),
            revealed: Cell::new(false),
            hide: RefCell::default(),
            width: Cell::new(0.),
            min_width: Cell::new(0.),
        })
    }

    /// The view the edge strip and the panel float in: the split view controller's view, above
    /// every pane and above nd's root.
    fn stage(&self) -> Option<Retained<NSView>> {
        self.controller.parentViewController().and_then(|p| p.viewIfLoaded())
    }

    fn window(&self) -> Option<Retained<NSWindow>> {
        self.stage().and_then(|s| s.window())
    }

    /// The size nd lays the content out at while it is lifted.
    pub fn boundary(&self) -> Option<NSSize> {
        self.shown.borrow().as_ref().map(|s| s.panel.frame().size)
    }

    /// Re-derives what should be installed from the props and the item's collapsed state.
    /// Cheap enough to run on every pane layout.
    pub fn update(self: &Rc<Self>, enabled: bool, width: Option<f32>, min_width: Option<f32>) {
        self.enabled.set(enabled);
        self.min_width.set(f64::from(min_width.unwrap_or(0.)));
        if self.width.get() == 0.
            && let Some(w) = width
        {
            self.width.set(f64::from(w));
        }
        let collapsed = self.item.isCollapsed();
        if !collapsed {
            let w = self.host.frame().size.width;
            if w > 0. {
                self.width.set(w);
            }
        }
        let wanted = enabled && collapsed;
        if !wanted {
            if self.revealed.get() || self.shown.borrow().is_some() {
                self.conceal(false);
            }
            if let Some(edge) = self.edge.borrow_mut().take() {
                edge.removeFromSuperview();
                if let Some(window) = self.window() {
                    lights_alpha(&window, 1.);
                }
            }
            return;
        }
        let Some(stage) = self.stage() else { return };
        if self.edge.borrow().is_none() {
            *self.edge.borrow_mut() = Some(NdRevealEdge::new(self.mtm, self));
        }
        let edge = self.edge.borrow().clone().expect("edge strip");
        let panel = self.shown.borrow().as_ref().map(|s| s.panel.clone());
        let top: Option<Retained<NSView>> = stage.subviews().lastObject();
        let on_top = match &panel {
            Some(p) => top.is_some_and(|t| std::ptr::eq::<NSView>(&*t, &***p)),
            None => top.is_some_and(|t| std::ptr::eq::<NSView>(&*t, &**edge)),
        };
        if unsafe { edge.superview() }.is_none_or(|s| s != stage) || !on_top {
            stage.addSubview_positioned_relativeTo(&edge, NSWindowOrderingMode::Above, None);
            if let Some(panel) = &panel {
                stage.addSubview_positioned_relativeTo(panel, NSWindowOrderingMode::Above, None);
            }
        }
        edge.setFrame(NSRect::new(NSPoint::ZERO, NSSize::new(EDGE, stage.bounds().size.height)));
        edge.setAutoresizingMask(NSAutoresizingMaskOptions::ViewHeightSizable);
        if !self.revealed.get()
            && let Some(window) = stage.window()
        {
            lights_alpha(&window, 0.);
        }
    }

    fn set_revealed(&self, on: bool) {
        if self.revealed.replace(on) != on {
            self.hooks.emit.emit(SidebarRevealed { revealed: on });
            self.hooks.wake.wake();
        }
    }

    fn frame(stage: &NSView, x: f64, width: f64) -> NSRect {
        let height = (stage.bounds().size.height - INSET * 2.).max(0.);
        NSRect::new(NSPoint::new(x, INSET), NSSize::new(width, height))
    }

    pub fn reveal(self: &Rc<Self>, animated: bool) {
        self.cancel_hide();
        if self.revealed.get() || !self.enabled.get() || !self.item.isCollapsed() {
            return;
        }
        let Some(stage) = self.stage() else { return };
        let Some(window) = stage.window() else { return };
        // A conceal still sliding out hands its panel over.
        if let Some(old) = self.shown.borrow_mut().take() {
            old.panel.removeFromSuperview();
        }
        let width = if self.width.get() > 0. { self.width.get() } else { self.min_width.get().max(240.) };
        let shown = Self::frame(&stage, INSET, width);
        let start = Self::frame(&stage, -width - INSET, width);

        let panel = NdRevealPanel::new(self.mtm, self);
        panel.setWantsLayer(true);
        if let Some(layer) = panel.layer() {
            layer.setShadowColor(Some(&NSColor::blackColor().CGColor()));
            layer.setShadowOpacity(0.28);
            layer.setShadowRadius(18.);
            layer.setShadowOffset(objc2_core_foundation::CGSize::new(0., 0.));
        }
        // Sized before the material goes in: autoresizing would add the panel's growth from
        // zero to the material's own size.
        panel.setFrame(NSRect::new(NSPoint::ZERO, shown.size));
        let body: Retained<NSView> =
            Retained::into_super(crate::views::NdLayerView::new(self.mtm, &crate::look::ViewLook::default()));
        let material: Retained<NSView> = if crate::glass::has_liquid_glass() {
            let glass = NSGlassEffectView::new(self.mtm);
            glass.setCornerRadius(RADIUS);
            glass.setContentView(Some(&body));
            Retained::into_super(glass)
        } else {
            let effect = NSVisualEffectView::new(self.mtm);
            effect.setMaterial(NSVisualEffectMaterial::Sidebar);
            effect.setBlendingMode(NSVisualEffectBlendingMode::WithinWindow);
            effect.setState(NSVisualEffectState::Active);
            effect.setWantsLayer(true);
            if let Some(l) = effect.layer() {
                l.setCornerRadius(RADIUS);
                l.setMasksToBounds(true);
            }
            body.setFrame(NSRect::new(NSPoint::ZERO, shown.size));
            body.setAutoresizingMask(
                NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
            );
            effect.addSubview(&body);
            Retained::into_super(effect)
        };
        material.setFrame(NSRect::new(NSPoint::ZERO, shown.size));
        material.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        panel.addSubview(&material);

        self.content.removeFromSuperview();
        self.content.setTranslatesAutoresizingMaskIntoConstraints(true);
        self.content.setFrame(NSRect::new(NSPoint::ZERO, shown.size));
        body.addSubview(&self.content);
        panel.setFrame(shown);
        panel.setAutoresizingMask(NSAutoresizingMaskOptions::ViewHeightSizable);
        stage.addSubview_positioned_relativeTo(&panel, NSWindowOrderingMode::Above, None);
        *self.shown.borrow_mut() = Some(Shown { panel: panel.clone() });
        self.set_revealed(true);
        (self.hooks.relayout)();
        (self.hooks.place_lights)();
        lights_alpha(&window, 1.);

        if !animated {
            return;
        }
        let duration = IN * slowdown();
        let reduce = reduce_motion();
        if reduce {
            panel.setAlphaValue(0.);
        } else {
            panel.setFrame(start);
        }
        let trace = Trace::start(&panel, "in");
        let (p, w) = (panel.clone(), window.clone());
        let offset = start.origin.x - shown.origin.x;
        let changes = RcBlock::new(move |ctx: NonNull<NSAnimationContext>| {
            let ctx = unsafe { ctx.as_ref() };
            ctx.setDuration(duration);
            ctx.setTimingFunction(Some(&curve()));
            ctx.setAllowsImplicitAnimation(true);
            if reduce {
                p.animator().setAlphaValue(1.);
                fade(&w, 0., 1.);
            } else {
                p.animator().setFrame(shown);
                slide(&w, offset, 0., duration);
            }
        });
        let weak = Rc::downgrade(self);
        let (p, w) = (panel.clone(), window.clone());
        let done = RcBlock::new(move || {
            if let Some(trace) = &trace {
                trace.stop();
            }
            // A conceal that began mid-slide owns the ride now.
            let Some(this) = weak.upgrade() else { return };
            let current = this.shown.borrow().as_ref().is_some_and(|s| std::ptr::eq(&*s.panel, &*p));
            if current && this.revealed.get() {
                end_ride(&w);
            }
        });
        NSAnimationContext::runAnimationGroup_completionHandler(&changes, Some(&done));
    }

    fn cancel_hide(&self) {
        if let Some(timer) = self.hide.borrow_mut().take() {
            timer.invalidate();
        }
    }

    fn schedule_hide(self: &Rc<Self>) {
        self.cancel_hide();
        let weak = Rc::downgrade(self);
        let timer = crate::events::after(LEAVE * slowdown(), move || {
            let Some(this) = weak.upgrade() else { return };
            this.hide.borrow_mut().take();
            let panel = this.shown.borrow().as_ref().map(|s| s.panel.clone());
            let Some(panel) = panel else { return };
            // Still over the panel: a tracking area can report an exit into a popover or menu
            // of the panel's own.
            if let Some(window) = panel.window() {
                let p = panel.convertPoint_fromView(window.mouseLocationOutsideOfEventStream(), None);
                let b = panel.bounds();
                if p.x >= 0. && p.y >= 0. && p.x <= b.size.width && p.y <= b.size.height {
                    return;
                }
            }
            this.conceal(true);
        });
        *self.hide.borrow_mut() = Some(timer);
    }

    pub fn conceal(self: &Rc<Self>, animated: bool) {
        self.cancel_hide();
        if !self.revealed.get() {
            return;
        }
        let Some((panel, stage)) = self
            .shown
            .borrow()
            .as_ref()
            .and_then(|s| unsafe { s.panel.superview() }.map(|stage| (s.panel.clone(), stage)))
        else {
            return;
        };
        self.set_revealed(false);
        let Some(window) = panel.window().filter(|_| animated) else {
            self.finish(&panel);
            return;
        };
        let width = panel.frame().size.width;
        let from = panel.layer().and_then(|l| unsafe { l.presentationLayer() }).map_or(panel.frame().origin.x, |l| {
            let model = panel.layer().map_or(l.position().x, |m| m.position().x);
            panel.frame().origin.x + l.position().x - model
        });
        let end = Self::frame(&stage, -width - INSET, width);
        let duration = OUT * slowdown();
        let reduce = reduce_motion();
        let trace = Trace::start(&panel, "out");
        let (p, w) = (panel.clone(), window.clone());
        let place = Rc::downgrade(self);
        let changes = RcBlock::new(move |ctx: NonNull<NSAnimationContext>| {
            let ctx = unsafe { ctx.as_ref() };
            ctx.setDuration(duration);
            ctx.setTimingFunction(Some(&curve()));
            ctx.setAllowsImplicitAnimation(true);
            if reduce {
                p.animator().setAlphaValue(0.);
                fade(&w, 1., 0.);
            } else {
                p.animator().setFrame(end);
                // The lights follow the panel's model frame whenever nd lays the window out,
                // so they go to where it rests now and are drawn from wherever it is, which
                // also turns a slide in still running round from where it got to. Moved
                // without implicit animation: the ride below is all their motion.
                ctx.setAllowsImplicitAnimation(false);
                if let Some(this) = place.upgrade() {
                    (this.hooks.place_lights)();
                }
                slide(&w, from - end.origin.x, 0., duration);
            }
        });
        let weak = Rc::downgrade(self);
        let done = RcBlock::new(move || {
            if let Some(trace) = &trace {
                trace.stop();
            }
            if let Some(this) = weak.upgrade() {
                this.finish(&panel);
            }
        });
        NSAnimationContext::runAnimationGroup_completionHandler(&changes, Some(&done));
    }

    /// Puts the lifted content back in its pane and drops the panel. A reveal that started
    /// while this one slid out owns a different panel, so only the one that finished goes.
    fn finish(&self, panel: &NdRevealPanel) {
        let window = panel.window();
        panel.removeFromSuperview();
        let current = self.shown.borrow().as_ref().is_some_and(|s| std::ptr::eq(&*s.panel, panel));
        if !current {
            return;
        }
        self.shown.borrow_mut().take();
        self.content.removeFromSuperview();
        self.content.setTranslatesAutoresizingMaskIntoConstraints(false);
        self.host.addSubview(&self.content);
        (self.hooks.relayout)();
        if let Some(window) = window {
            end_ride(&window);
            if self.enabled.get() && self.item.isCollapsed() {
                lights_alpha(&window, 0.);
            }
        }
    }
}

fn lights(window: &NSWindow) -> Vec<Retained<objc2_app_kit::NSButton>> {
    crate::traffic_lights::buttons(window).map(Vec::from).unwrap_or_default()
}

fn lights_alpha(window: &NSWindow, alpha: f64) {
    for b in lights(window) {
        b.setAlphaValue(alpha);
    }
}

/// Draws the buttons `from` points off their place, travelling to `to`, on the panel's timing.
/// Additive on the layers: the buttons' frames stay where the title bar allows, and added in
/// the same transaction as the panel's, it starts on the same frame.
fn slide(window: &NSWindow, from: f64, to: f64, duration: f64) {
    for b in lights(window) {
        b.setWantsLayer(true);
        let Some(layer) = b.layer() else { continue };
        let a = CABasicAnimation::animationWithKeyPath(Some(&NSString::from_str("position.x")));
        a.setAdditive(true);
        unsafe {
            a.setFromValue(Some(&NSNumber::new_f64(from)));
            a.setToValue(Some(&NSNumber::new_f64(to)));
            a.setFillMode(kCAFillModeBoth);
        }
        a.setDuration(duration);
        a.setTimingFunction(Some(&curve()));
        a.setRemovedOnCompletion(false);
        layer.addAnimation_forKey(&a, Some(&NSString::from_str(RIDE)));
    }
}

fn fade(window: &NSWindow, from: f64, to: f64) {
    for b in lights(window) {
        b.setAlphaValue(from);
        b.animator().setAlphaValue(to);
    }
}

fn end_ride(window: &NSWindow) {
    for b in lights(window) {
        if let Some(layer) = b.layer() {
            layer.removeAnimationForKey(&NSString::from_str(RIDE));
        }
    }
}

pub struct TraceIvars {
    panel: objc2::rc::Weak<NSView>,
    direction: &'static str,
    begin: Instant,
    link: RefCell<Option<Retained<CADisplayLink>>>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements and this type does not implement Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = TraceIvars]
    struct Trace;

    impl Trace {
        #[unsafe(method(tick:))]
        fn tick(&self, _link: &CADisplayLink) {
            self.print();
        }
    }
);

impl Trace {
    fn start(panel: &NSView, direction: &'static str) -> Option<Retained<Self>> {
        if std::env::var("ND_REVEAL_TRACE").as_deref() != Ok("1") {
            return None;
        }
        let mtm = MainThreadMarker::from(panel);
        let this = Self::alloc(mtm).set_ivars(TraceIvars {
            panel: objc2::rc::Weak::new(panel),
            direction,
            begin: Instant::now(),
            link: RefCell::default(),
        });
        let this: Retained<Self> = unsafe { msg_send![super(this), init] };
        let link = unsafe { panel.displayLinkWithTarget_selector(&this, sel!(tick:)) };
        unsafe { link.addToRunLoop_forMode(&NSRunLoop::mainRunLoop(), NSRunLoopCommonModes) };
        *this.ivars().link.borrow_mut() = Some(link);
        Some(this)
    }

    fn print(&self) {
        let Some(panel) = self.ivars().panel.load() else { return };
        let Some(window) = panel.window() else { return };
        let Some(close) = window.standardWindowButton(objc2_app_kit::NSWindowButton::CloseButton) else { return };
        let (Some(layer), Some(close_layer)) = (panel.layer(), close.layer()) else { return };
        let drift = |l: &objc2_quartz_core::CALayer| {
            unsafe { l.presentationLayer() }.map_or(0., |p| p.position().x - l.position().x)
        };
        let panel_x = panel.convertRect_toView(panel.bounds(), None).origin.x + drift(&layer);
        let close_x = close.convertRect_toView(close.bounds(), None).origin.x + drift(&close_layer);
        let alpha = unsafe { close_layer.presentationLayer() }.map_or(close_layer.opacity(), |p| p.opacity());
        eprintln!(
            "ND_REVEAL_FRAME dir={} t={:.4} panel={:.2} close={:.2} alpha={:.2} hidden={}",
            self.ivars().direction,
            self.ivars().begin.elapsed().as_secs_f64(),
            panel_x,
            close_x,
            alpha,
            u8::from(close.isHidden()),
        );
    }

    fn stop(&self) {
        self.print();
        if let Some(link) = self.ivars().link.borrow_mut().take() {
            link.invalidate();
        }
    }
}
