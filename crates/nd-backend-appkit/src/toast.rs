//! `toast_overlay()` and `banner()` on AppKit. AppKit has no toast, so the overlay draws one: a
//! Liquid Glass capsule (vibrancy below macOS 26) with the title and an optional borderless
//! button, bottom-centre in the node's own window, sliding up as it fades in. It sits in an
//! in-window layer over the window's frame view that passes every click it does not cover
//! through, so the content below keeps working (ARCHITECTURE.md 6.3). Queue rules are
//! [`ToastQueue`]'s. VoiceOver announces each toast as it appears.
//!
//! The banner is a bar across its container: the title and an optional button on the system's
//! fill colour.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use nd_components::{
    BANNER, BannerActionClicked, BannerProps, TOAST, Toast, ToastChange, ToastOverlayProps, ToastQueue,
};
use nd_core::Size;
use nd_ui::events::Emit;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAccessibility, NSAccessibilityAnnouncementKey, NSAccessibilityAnnouncementRequestedNotification,
    NSAccessibilityGroupRole, NSAccessibilityPostNotificationWithUserInfo, NSAnimatablePropertyContainer,
    NSAnimationContext, NSAutoresizingMaskOptions, NSButton, NSColor, NSControlSize, NSEvent, NSGlassEffectView,
    NSLineBreakMode, NSResponder, NSTextField, NSView, NSVisualEffectBlendingMode, NSVisualEffectMaterial,
    NSVisualEffectState, NSVisualEffectView, NSWorkspace,
};
use objc2_foundation::{MainThreadMarker, NSDictionary, NSPoint, NSRect, NSSize, NSString, NSTimer};
use objc2_quartz_core::{CAMediaTimingFunction, kCAMediaTimingFunctionEaseOut};

use crate::events::{Wake, defer};

pub struct LayerIvars {
    overlay: RefCell<Weak<Overlay>>,
}

define_class!(
    /// The window-sized layer toasts sit in. Flipped; a click that misses every toast falls
    /// through to the window below.
    // SAFETY: NSView has no subclassing requirements beyond calling super; no Drop.
    #[unsafe(super(NSView, NSResponder))]
    #[thread_kind = MainThreadOnly]
    #[ivars = LayerIvars]
    pub struct NdToastLayer;

    impl NdToastLayer {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method_id(hitTest:))]
        fn hit_test(&self, point: NSPoint) -> Option<Retained<NSView>> {
            let hit: Option<Retained<NSView>> = unsafe { msg_send![super(self), hitTest: point] };
            hit.filter(|h| !std::ptr::eq::<NSView>(&**h, (self as *const Self).cast()))
        }

        #[unsafe(method(layout))]
        fn layout(&self) {
            let _: () = unsafe { msg_send![super(self), layout] };
            if let Some(o) = self.ivars().overlay.borrow().upgrade() {
                o.place();
            }
        }
    }
);

define_class!(
    /// One toast: a click on it dismisses it; its button reports and dismisses.
    // SAFETY: as above.
    #[unsafe(super(NSView, NSResponder))]
    #[thread_kind = MainThreadOnly]
    #[ivars = LayerIvars]
    pub struct NdToastView;

    impl NdToastView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, _event: &NSEvent) {
            if let Some(o) = self.ivars().overlay.borrow().upgrade() {
                let changes = o.queue.borrow_mut().clicked();
                o.apply(changes);
            }
        }

        #[unsafe(method(actionClicked:))]
        fn action_clicked(&self, _sender: &NSButton) {
            if let Some(o) = self.ivars().overlay.borrow().upgrade() {
                let changes = o.queue.borrow_mut().action();
                o.apply(changes);
            }
        }
    }
);

struct Shown {
    toast: Toast,
    view: Retained<NdToastView>,
    material: Retained<NSView>,
    label: Retained<NSTextField>,
    button: Option<Retained<NSButton>>,
    timer: Option<Retained<NSTimer>>,
}

pub(crate) struct Overlay {
    mtm: MainThreadMarker,
    emit: Emit,
    wake: Wake,
    handle: Retained<NSView>,
    queue: RefCell<ToastQueue>,
    layer: RefCell<Option<Retained<NdToastLayer>>>,
    /// The overlay window over the node's window that the layer fills.
    window_layer: RefCell<Option<crate::overlay::WindowLayer>>,
    shown: RefCell<Option<Shown>>,
    /// Changes waiting for the node to reach a window.
    waiting: RefCell<Vec<ToastChange>>,
    retrying: Cell<bool>,
    weak: Weak<Overlay>,
}

pub(crate) struct ToastNative(Rc<Overlay>);

impl ToastNative {
    pub fn new(mtm: MainThreadMarker, emit: Emit, wake: Wake, props: &ToastOverlayProps) -> Self {
        let handle = NSView::new(mtm);
        handle.setHidden(true);
        let overlay = Rc::new_cyclic(|weak| Overlay {
            mtm,
            emit,
            wake,
            handle,
            queue: RefCell::default(),
            layer: RefCell::default(),
            window_layer: RefCell::default(),
            shown: RefCell::default(),
            waiting: RefCell::default(),
            retrying: Cell::new(false),
            weak: weak.clone(),
        });
        let this = Self(overlay);
        this.update(props);
        this
    }

    pub fn view(&self) -> &NSView {
        &self.0.handle
    }

    pub fn update(&self, props: &ToastOverlayProps) {
        let changes = self.0.queue.borrow_mut().sync(&props.toasts);
        // Showing waits a turn: never inside the transaction, and the node needs its window.
        self.0.waiting.borrow_mut().extend(changes);
        self.0.flush_later();
    }

    /// The toast on screen: its title, button label and frame in its layer.
    pub fn shown(&self) -> Option<(String, Option<String>, NSRect)> {
        let shown = self.0.shown.borrow();
        let s = shown.as_ref()?;
        Some((s.label.stringValue().to_string(), s.button.as_ref().map(|b| b.title().to_string()), s.view.frame()))
    }

    /// The toast's view, while one is on screen.
    pub fn toast_view(&self) -> Option<Retained<NSView>> {
        self.0.shown.borrow().as_ref().map(|s| Retained::into_super(s.view.clone()))
    }

    /// Clicks the shown toast's button, or its body, as the user would.
    pub fn click(&self, button: bool) {
        let changes = if button { self.0.queue.borrow_mut().action() } else { self.0.queue.borrow_mut().clicked() };
        self.0.apply(changes);
    }

    pub fn material_class(&self) -> Option<String> {
        self.0.shown.borrow().as_ref().map(|s| s.material.class().name().to_str().unwrap_or("?").to_owned())
    }
}

impl Drop for ToastNative {
    fn drop(&mut self) {
        let changes = self.0.queue.borrow_mut().clear();
        self.0.waiting.borrow_mut().clear();
        self.0.apply(changes);
        if let Some(layer) = self.0.layer.borrow_mut().take() {
            layer.removeFromSuperview();
        }
        self.0.window_layer.borrow_mut().take();
    }
}

impl Overlay {
    fn flush_later(&self) {
        if self.waiting.borrow().is_empty() || self.retrying.replace(true) {
            return;
        }
        let weak = self.weak.clone();
        defer(move || {
            if let Some(o) = weak.upgrade() {
                o.retrying.set(false);
                o.flush();
            }
        });
    }

    fn flush(&self) {
        if self.handle.window().is_none() {
            // The node is not in a window yet: try again shortly.
            if !self.retrying.replace(true) {
                let weak = self.weak.clone();
                crate::events::after(0.05, move || {
                    if let Some(o) = weak.upgrade() {
                        o.retrying.set(false);
                        o.flush();
                    }
                });
            }
            return;
        }
        let changes = std::mem::take(&mut *self.waiting.borrow_mut());
        self.apply(changes);
    }

    fn apply(&self, changes: Vec<ToastChange>) {
        let mut reported = false;
        for change in changes {
            match change {
                ToastChange::Show(toast) => self.show(toast),
                ToastChange::Refresh(toast) => self.refresh(toast),
                ToastChange::Hide => self.hide(),
                ToastChange::Dismissed(event) => {
                    self.emit.emit(event);
                    reported = true;
                }
                ToastChange::Action(event) => {
                    self.emit.emit(event);
                    reported = true;
                }
            }
        }
        if reported {
            self.wake.wake();
        }
    }

    /// The layer over this node's own window, in the overlay window above it (6.3), made on
    /// first use. The overlay window never takes key status, and a click that misses every toast
    /// lands on its transparent area, which the window server passes through to the window.
    fn layer(&self) -> Option<Retained<NdToastLayer>> {
        let window = self.handle.window()?;
        let current = self.window_layer.borrow().as_ref().is_some_and(|l| *l.owner() == *window);
        if !current {
            let layer = crate::overlay::WindowLayer::new(self.mtm, &window, false);
            layer.show();
            *self.window_layer.borrow_mut() = Some(layer);
        }
        let host = self.window_layer.borrow().as_ref()?.host();
        let mut slot = self.layer.borrow_mut();
        if let Some(layer) = slot.as_ref()
            && unsafe { layer.superview() }.is_some_and(|s| s == host)
        {
            return Some(layer.clone());
        }
        let layer = NdToastLayer::alloc(self.mtm).set_ivars(LayerIvars { overlay: RefCell::new(self.weak.clone()) });
        let layer: Retained<NdToastLayer> = unsafe { msg_send![super(layer), initWithFrame: host.bounds()] };
        layer.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        host.addSubview(&layer);
        *slot = Some(layer.clone());
        Some(layer)
    }

    fn show(&self, toast: Toast) {
        let Some(layer) = self.layer() else { return };
        let mtm = self.mtm;
        let view = NdToastView::alloc(mtm).set_ivars(LayerIvars { overlay: RefCell::new(self.weak.clone()) });
        let view: Retained<NdToastView> = unsafe { msg_send![super(view), initWithFrame: NSRect::ZERO] };
        view.setWantsLayer(true);
        let content = crate::views::NdLayerView::new(mtm, &crate::look::ViewLook::default());
        let content: Retained<NSView> = Retained::into_super(content);
        let radius = f64::from(TOAST.radius);
        let material: Retained<NSView> = if crate::glass::has_liquid_glass() {
            let glass = NSGlassEffectView::new(mtm);
            glass.setCornerRadius(radius);
            glass.setContentView(Some(&content));
            Retained::into_super(glass)
        } else {
            let effect = NSVisualEffectView::new(mtm);
            effect.setMaterial(NSVisualEffectMaterial::HUDWindow);
            effect.setBlendingMode(NSVisualEffectBlendingMode::WithinWindow);
            effect.setState(NSVisualEffectState::Active);
            effect.setWantsLayer(true);
            if let Some(l) = effect.layer() {
                l.setCornerRadius(radius);
                l.setMasksToBounds(true);
            }
            effect.addSubview(&content);
            Retained::into_super(effect)
        };
        view.addSubview(&material);
        let label = NSTextField::labelWithString(&NSString::from_str(&toast.title), mtm);
        label.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
        content.addSubview(&label);
        let button = toast.action.as_ref().map(|title| {
            let target: &AnyObject = &view;
            let b = unsafe {
                NSButton::buttonWithTitle_target_action(
                    &NSString::from_str(title),
                    Some(target),
                    Some(sel!(actionClicked:)),
                    mtm,
                )
            };
            // A bezel on the glass would read as a separate control, not part of the toast.
            b.setBordered(false);
            b.setControlSize(NSControlSize::Small);
            content.addSubview(&b);
            b
        });
        view.setAccessibilityElement(true);
        view.setAccessibilityRole(Some(unsafe { NSAccessibilityGroupRole }));
        view.setAccessibilityLabel(Some(&NSString::from_str(&toast.title)));
        layer.addSubview(&view);
        let timer = toast.timeout.map(|timeout| {
            let weak = self.weak.clone();
            let id = toast.id.clone();
            crate::events::after(timeout.as_secs_f64(), move || {
                if let Some(o) = weak.upgrade() {
                    let changes = o.queue.borrow_mut().expire(&id);
                    o.apply(changes);
                }
            })
        });
        *self.shown.borrow_mut() =
            Some(Shown { toast: toast.clone(), view: view.clone(), material, label, button, timer });
        self.place();
        self.enter(&view);
        announce(&view, &toast.title);
    }

    fn refresh(&self, toast: Toast) {
        let mut shown = self.shown.borrow_mut();
        let Some(s) = shown.as_mut() else { return };
        s.label.setStringValue(&NSString::from_str(&toast.title));
        if let (Some(b), Some(title)) = (&s.button, &toast.action) {
            b.setTitle(&NSString::from_str(title));
        }
        s.view.setAccessibilityLabel(Some(&NSString::from_str(&toast.title)));
        s.toast = toast;
        drop(shown);
        self.place();
    }

    fn hide(&self) {
        let Some(shown) = self.shown.borrow_mut().take() else { return };
        if let Some(timer) = &shown.timer {
            timer.invalidate();
        }
        exit(shown.view);
    }

    /// Sizes the shown toast to its content and places it bottom-centre in the window.
    fn place(&self) {
        let shown = self.shown.borrow();
        let Some(s) = shown.as_ref() else { return };
        let Some(layer) = self.layer.borrow().clone() else { return };
        let f = f64::from;
        let label = crate::palette::natural_size(&s.label);
        let button = s.button.as_ref().map(|b| b.fittingSize());
        let trailing = if button.is_some() { f(TOAST.padding_action) } else { f(TOAST.padding_x) };
        let extra = button.map_or(0., |b| f(TOAST.spacing) + b.width);
        let bounds = layer.bounds();
        let max = f(TOAST.max_width).min(bounds.size.width - 2. * f(TOAST.padding_x));
        let label_w = label.width.ceil().min((max - f(TOAST.padding_x) - trailing - extra).max(0.));
        let height = label.height.max(button.map_or(0., |b| b.height)).ceil() + 2. * f(TOAST.padding_y);
        let width = f(TOAST.padding_x) + label_w + extra + trailing;
        let origin =
            NSPoint::new(((bounds.size.width - width) / 2.).round(), bounds.size.height - f(TOAST.bottom) - height);
        // An entering toast animates its own origin; only its size is set here then.
        if s.view.alphaValue() >= 1. || s.view.frame().size.width == 0. {
            s.view.setFrame(NSRect::new(origin, NSSize::new(width, height)));
        } else {
            s.view.setFrameSize(NSSize::new(width, height));
        }
        let inner = NSRect::new(NSPoint::new(0., 0.), NSSize::new(width, height));
        s.material.setFrame(inner);
        if let Some(content) = s.material.subviews().iter().next().filter(|_| !crate::glass::has_liquid_glass()) {
            content.setFrame(inner);
        }
        s.label.setFrame(NSRect::new(
            NSPoint::new(f(TOAST.padding_x), ((height - label.height) / 2.).round()),
            NSSize::new(label_w, label.height),
        ));
        if let (Some(b), Some(size)) = (&s.button, button) {
            b.setFrame(NSRect::new(
                NSPoint::new(width - trailing - size.width, ((height - size.height) / 2.).round()),
                size,
            ));
        }
    }

    fn enter(&self, view: &NdToastView) {
        let reduce = NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion();
        let end = view.frame().origin;
        if reduce {
            return;
        }
        view.setAlphaValue(0.);
        view.setFrameOrigin(NSPoint::new(end.x, end.y + f64::from(TOAST.rise)));
        let view = view.retain();
        let changes = block2::RcBlock::new(move |context: std::ptr::NonNull<NSAnimationContext>| {
            let context = unsafe { context.as_ref() };
            context.setDuration(TOAST.enter.as_secs_f64());
            let ease = unsafe { kCAMediaTimingFunctionEaseOut };
            context.setTimingFunction(Some(&CAMediaTimingFunction::functionWithName(ease)));
            let animator = view.animator();
            animator.setAlphaValue(1.);
            animator.setFrameOrigin(end);
        });
        NSAnimationContext::runAnimationGroup(&changes);
    }
}

use objc2::Message;

fn exit(view: Retained<NdToastView>) {
    let reduce = NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion();
    if reduce {
        view.removeFromSuperview();
        return;
    }
    let fading = view.clone();
    let changes = block2::RcBlock::new(move |context: std::ptr::NonNull<NSAnimationContext>| {
        unsafe { context.as_ref() }.setDuration(TOAST.exit.as_secs_f64());
        fading.animator().setAlphaValue(0.);
    });
    let done = block2::RcBlock::new(move || view.removeFromSuperview());
    NSAnimationContext::runAnimationGroup_completionHandler(&changes, Some(&done));
}

/// Asks VoiceOver to read the toast without moving focus to it.
fn announce(view: &NSView, text: &str) {
    let key: &NSString = unsafe { NSAccessibilityAnnouncementKey };
    let value: Retained<AnyObject> = Retained::into_super(Retained::into_super(NSString::from_str(text)));
    let info: Retained<NSDictionary<NSString, AnyObject>> = NSDictionary::from_slices(&[key], &[&*value]);
    let element: &AnyObject =
        view.window().map_or_else(|| view.as_ref(), |w| unsafe { &*Retained::as_ptr(&w).cast::<AnyObject>() });
    unsafe {
        NSAccessibilityPostNotificationWithUserInfo(
            element,
            NSAccessibilityAnnouncementRequestedNotification,
            Some(&info),
        )
    };
}

pub struct BannerIvars {
    emit: Emit,
    wake: Wake,
    label: Retained<NSTextField>,
    button: RefCell<Option<Retained<NSButton>>>,
}

define_class!(
    /// The banner's bar: the title and an optional button on the system's fill colour.
    // SAFETY: as above.
    #[unsafe(super(NSView, NSResponder))]
    #[thread_kind = MainThreadOnly]
    #[ivars = BannerIvars]
    pub struct NdBannerView;

    impl NdBannerView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(updateLayer))]
        fn update_layer(&self) {
            // Resolved on every appearance change, so the fill follows light and dark.
            if let Some(layer) = self.layer() {
                layer.setBackgroundColor(Some(&NSColor::quaternarySystemFillColor().CGColor()));
            }
        }

        #[unsafe(method(wantsUpdateLayer))]
        fn wants_update_layer(&self) -> bool {
            true
        }

        #[unsafe(method(layout))]
        fn layout(&self) {
            let _: () = unsafe { msg_send![super(self), layout] };
            self.place();
        }

        #[unsafe(method(actionClicked:))]
        fn action_clicked(&self, _sender: &NSButton) {
            self.fire();
        }
    }
);

impl NdBannerView {
    fn fire(&self) {
        self.ivars().emit.emit(BannerActionClicked);
        self.ivars().wake.wake();
    }

    fn place(&self) {
        let iv = self.ivars();
        let b = self.bounds();
        let f = f64::from;
        let button = iv.button.borrow().as_ref().map(|b| (b.clone(), b.fittingSize()));
        let label = crate::palette::natural_size(&iv.label);
        let mut end = b.size.width - f(BANNER.padding_x);
        if let Some((button, size)) = &button {
            let x = end - size.width;
            button.setFrame(NSRect::new(NSPoint::new(x, ((b.size.height - size.height) / 2.).round()), *size));
            end = x - f(BANNER.spacing);
        }
        iv.label.setFrame(NSRect::new(
            NSPoint::new(f(BANNER.padding_x), ((b.size.height - label.height) / 2.).round()),
            NSSize::new((end - f(BANNER.padding_x)).max(0.), label.height),
        ));
    }

    fn measure(&self, revealed: bool) -> Size {
        if !revealed {
            return Size::default();
        }
        let iv = self.ivars();
        let label = crate::palette::natural_size(&iv.label);
        let button = iv.button.borrow().as_ref().map_or(NSSize::new(0., 0.), |b| b.fittingSize());
        let height = label.height.max(button.height).ceil() + 2. * f64::from(BANNER.padding_y);
        Size { width: 0., height: height as f32 }
    }
}

pub(crate) struct BannerNative {
    view: Retained<NdBannerView>,
    props: BannerProps,
}

impl BannerNative {
    pub fn new(mtm: MainThreadMarker, emit: Emit, wake: Wake, props: &BannerProps) -> Self {
        let label = NSTextField::labelWithString(&NSString::new(), mtm);
        label.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
        let view = NdBannerView::alloc(mtm).set_ivars(BannerIvars { emit, wake, label, button: RefCell::new(None) });
        let view: Retained<NdBannerView> = unsafe { msg_send![super(view), initWithFrame: NSRect::ZERO] };
        view.setWantsLayer(true);
        view.addSubview(&view.ivars().label);
        view.setAccessibilityElement(true);
        view.setAccessibilityRole(Some(unsafe { NSAccessibilityGroupRole }));
        let mut this = Self { view, props: props.clone() };
        this.apply(props, true);
        this
    }

    pub fn view(&self) -> &NSView {
        &self.view
    }

    pub fn update(&mut self, props: &BannerProps) {
        self.apply(props, false);
    }

    fn apply(&mut self, props: &BannerProps, first: bool) {
        let iv = self.view.ivars();
        if first || props.title != self.props.title {
            iv.label.setStringValue(&NSString::from_str(&props.title));
            self.view.setAccessibilityLabel(Some(&NSString::from_str(&props.title)));
        }
        if first || props.action != self.props.action {
            if let Some(old) = iv.button.borrow_mut().take() {
                old.removeFromSuperview();
            }
            if let Some(title) = &props.action {
                let target: &AnyObject = &self.view;
                let b = unsafe {
                    NSButton::buttonWithTitle_target_action(
                        &NSString::from_str(title),
                        Some(target),
                        Some(sel!(actionClicked:)),
                        MainThreadMarker::from(&*self.view),
                    )
                };
                self.view.addSubview(&b);
                *iv.button.borrow_mut() = Some(b);
            }
        }
        self.view.setHidden(!props.revealed);
        self.props = props.clone();
        self.view.setNeedsLayout(true);
    }

    pub fn measure(&self) -> Size {
        self.view.measure(self.props.revealed)
    }

    pub fn press(&self) {
        if self.view.ivars().button.borrow().is_some() {
            self.view.fire();
        }
    }
}
