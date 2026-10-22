//! The primitive views behind retained nodes: [`NdRootView`] (a window's content root),
//! [`NdLayerView`] (`div`) and [`NdTextView`] (`text`). All are flipped, so frames from `nd-ui`
//! (top-left origin) apply unchanged.

use std::cell::{Cell, OnceCell, RefCell};
use std::rc::Rc;

use nd_components::ContextMenuRequested;
use nd_core::Size;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{DefinedClass, MainThreadOnly, Message, define_class, msg_send};
use objc2_app_kit::{
    NSAccessibilityStaticTextRole, NSAttributedStringNSExtendedStringDrawing, NSEvent, NSEventModifierFlags, NSFont,
    NSFontAttributeName, NSForegroundColorAttributeName, NSResponder, NSStringDrawingOptions, NSView,
    NSViewLayerContentsRedrawPolicy,
};
use objc2_foundation::{MainThreadMarker, NSAttributedString, NSDictionary, NSPoint, NSRect, NSSize, NSString};

use nd_gesture::PointerPhase;

use crate::gestures::GestureHost;
use crate::look::{Color, TextLook, ViewLook};

pub(crate) type LayoutHook = Rc<dyn Fn(Size)>;

#[derive(Default)]
pub struct RootIvars {
    on_layout: RefCell<Option<LayoutHook>>,
    content: OnceCell<Retained<NdLayerView>>,
    /// Content covers the whole window, title bar included: a scaffold (whose split view
    /// insets its panes itself) or a custom title bar.
    full_size: Cell<bool>,
    /// Lies over a scaffold's split view: a click that lands on no nd view of its own falls
    /// through to the panes below.
    passes_through: Cell<bool>,
    /// Containers that pass clicks through with it (the scaffold's host view).
    pass_views: RefCell<Vec<Retained<NSView>>>,
}

define_class!(
    // SAFETY: NSView has no subclassing requirements beyond calling super in the overrides
    // below, and this type does not implement Drop.
    #[unsafe(super(NSView, NSResponder))]
    #[thread_kind = MainThreadOnly]
    #[ivars = RootIvars]
    pub struct NdRootView;

    impl NdRootView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method_id(hitTest:))]
        fn hit_test(&self, point: NSPoint) -> Option<Retained<NSView>> {
            let hit: Option<Retained<NSView>> = unsafe { msg_send![super(self), hitTest: point] };
            if self.ivars().passes_through.get() && hit.as_ref().is_some_and(|h| self.passes(h)) { None } else { hit }
        }

        #[unsafe(method(setFrameSize:))]
        fn set_frame_size(&self, size: NSSize) {
            let _: () = unsafe { msg_send![super(self), setFrameSize: size] };
            self.setNeedsLayout(true);
        }

        // AppKit runs this in the display cycle, before drawing and inside the same Core
        // Animation transaction, including on every frame of a live resize. Laying out here is
        // what makes the presented frame the frame laid out.
        #[unsafe(method(layout))]
        fn layout(&self) {
            let _: () = unsafe { msg_send![super(self), layout] };
            let safe = self.safe_rect();
            let content = self.ivars().content.get().expect("content view");
            if content.frame() != safe {
                content.setFrame(safe);
            }
            let hook = self.ivars().on_layout.borrow().clone();
            if let Some(hook) = hook {
                hook(Size { width: safe.size.width as f32, height: safe.size.height as f32 });
            }
        }
    }
);

impl NdRootView {
    pub(crate) fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(RootIvars::default());
        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] };
        this.setWantsLayer(true);
        let content = NdLayerView::new(mtm, &ViewLook::default());
        this.addSubview(&content);
        let _ = this.ivars().content.set(content);
        this
    }

    /// The view the window's root node is: the content view minus the title bar band, so
    /// app content never sits under the OS title bar.
    pub(crate) fn content(&self) -> &NdLayerView {
        self.ivars().content.get().expect("content view")
    }

    /// Lets clicks that hit only the root, its content view or `via` reach the views below.
    /// `None` makes the root take its clicks again.
    pub(crate) fn set_passes_through(&self, via: Option<&NSView>) {
        self.ivars().passes_through.set(via.is_some());
        *self.ivars().pass_views.borrow_mut() = via.map(|v| v.retain()).into_iter().collect();
    }

    /// `view` is the root, its content view, or a pass-through container.
    fn passes(&self, view: &NSView) -> bool {
        let ptr: *const NSView = view;
        let content = self.ivars().content.get().map(|c| Retained::as_ptr(c).cast::<NSView>());
        std::ptr::eq(ptr, (self as *const Self).cast())
            || content.is_some_and(|c| std::ptr::eq(ptr, c))
            || self.ivars().pass_views.borrow().iter().any(|v| std::ptr::eq(ptr, Retained::as_ptr(v)))
    }

    pub(crate) fn set_full_size(&self, full: bool) {
        if self.ivars().full_size.replace(full) != full {
            self.setNeedsLayout(true);
        }
    }

    /// The window's `contentLayoutRect` in this (full-size, flipped) view.
    fn safe_rect(&self) -> NSRect {
        let bounds = self.bounds();
        if self.ivars().full_size.get() {
            return bounds;
        }
        let Some(window) = self.window() else { return bounds };
        let layout = window.contentLayoutRect();
        let top = (bounds.size.height - (layout.origin.y + layout.size.height)).max(0.);
        NSRect::new(NSPoint::new(0., top), NSSize::new(bounds.size.width, (bounds.size.height - top).max(0.)))
    }

    pub(crate) fn set_layout_hook(&self, hook: Option<LayoutHook>) {
        *self.ivars().on_layout.borrow_mut() = hook;
        self.setNeedsLayout(true);
    }
}

pub struct LayerIvars {
    props: RefCell<ViewLook>,
    /// Set once nd focuses the node; a plain `div` refuses first responder.
    focusable: Cell<bool>,
    /// Multiplies the corner radius: the theme's radius scale at this node.
    radius_scale: Cell<f32>,
    /// A press on the view's own area moves the window (a title bar).
    drag: Cell<bool>,
    /// Right clicks report a context menu request to the node.
    menu: RefCell<Option<(nd_ui::events::Emit, crate::events::Wake)>>,
    /// Painted when the style sets no background: the window background, under a subtree
    /// scope that flips the appearance, so text inside reads on it.
    surface: Cell<Option<Color>>,
    /// The gestures the node listens for.
    gestures: RefCell<Option<GestureHost>>,
    /// The last Force Touch stage seen, so a force click fires once per press.
    stage: Cell<isize>,
    /// Pointer enter and exit are reported here (`.on_hover`).
    hover_emit: RefCell<Option<(nd_ui::events::Emit, crate::events::Wake)>>,
    /// The pointer is over the view, which paints its hover look then.
    hovered: Cell<bool>,
    tracking: RefCell<Option<Retained<objc2_app_kit::NSTrackingArea>>>,
    /// Where a hero in flight lives in the tree while it is drawn on the window's frame view.
    lifted: RefCell<Option<Home>>,
}

/// A lifted hero's place in the tree: its parent, and the sibling above it.
struct Home {
    parent: Retained<NSView>,
    above: Option<Retained<NSView>>,
}

impl Default for LayerIvars {
    fn default() -> Self {
        Self {
            props: RefCell::default(),
            focusable: Cell::new(false),
            radius_scale: Cell::new(1.),
            drag: Cell::new(false),
            menu: RefCell::new(None),
            surface: Cell::new(None),
            gestures: RefCell::new(None),
            stage: Cell::new(0),
            hover_emit: RefCell::new(None),
            hovered: Cell::new(false),
            tracking: RefCell::new(None),
            lifted: RefCell::new(None),
        }
    }
}

define_class!(
    // SAFETY: as for NdRootView.
    #[unsafe(super(NSView, NSResponder))]
    #[thread_kind = MainThreadOnly]
    #[ivars = LayerIvars]
    pub struct NdLayerView;

    impl NdLayerView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(wantsUpdateLayer))]
        fn wants_update_layer(&self) -> bool {
            true
        }

        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool {
            self.ivars().focusable.get()
        }

        // Content under a transparent title bar must not move the window unless it is a drag
        // region: a click on a card there is a click.
        #[unsafe(method(mouseDownCanMoveWindow))]
        fn mouse_down_can_move_window(&self) -> bool {
            false
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            if event.modifierFlags().contains(NSEventModifierFlags::Control) && self.request_menu(event) {
                return;
            }
            if self.ivars().drag.get() && let Some(window) = self.window() {
                if event.clickCount() == 2 {
                    crate::titlebar::double_click(&window);
                } else {
                    window.performWindowDragWithEvent(event);
                }
                return;
            }
            if crate::gestures::mouse(self, event, PointerPhase::Down) {
                return;
            }
            let _: () = unsafe { msg_send![super(self), mouseDown: event] };
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, event: &NSEvent) {
            if !crate::gestures::mouse(self, event, PointerPhase::Move) {
                let _: () = unsafe { msg_send![super(self), mouseDragged: event] };
            }
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, event: &NSEvent) {
            if !crate::gestures::mouse(self, event, PointerPhase::Up) {
                let _: () = unsafe { msg_send![super(self), mouseUp: event] };
            }
        }

        #[unsafe(method(magnifyWithEvent:))]
        fn magnify_with_event(&self, event: &NSEvent) {
            if !crate::gestures::magnify(self, self.ivars().gestures.borrow().as_ref(), event) {
                let _: () = unsafe { msg_send![super(self), magnifyWithEvent: event] };
            }
        }

        #[unsafe(method(rotateWithEvent:))]
        fn rotate_with_event(&self, event: &NSEvent) {
            if !crate::gestures::rotate(self, self.ivars().gestures.borrow().as_ref(), event) {
                let _: () = unsafe { msg_send![super(self), rotateWithEvent: event] };
            }
        }

        #[unsafe(method(smartMagnifyWithEvent:))]
        fn smart_magnify_with_event(&self, event: &NSEvent) {
            if !crate::gestures::smart_magnify(self, self.ivars().gestures.borrow().as_ref(), event) {
                let _: () = unsafe { msg_send![super(self), smartMagnifyWithEvent: event] };
            }
        }

        #[unsafe(method(pressureChangeWithEvent:))]
        fn pressure_change_with_event(&self, event: &NSEvent) {
            let mut stage = self.ivars().stage.get();
            let handled = crate::gestures::pressure(self, self.ivars().gestures.borrow().as_ref(), event, &mut stage);
            self.ivars().stage.set(stage);
            if !handled {
                let _: () = unsafe { msg_send![super(self), pressureChangeWithEvent: event] };
            }
        }

        #[unsafe(method(scrollWheel:))]
        fn scroll_wheel(&self, event: &NSEvent) {
            let host = self.ivars().gestures.borrow().clone();
            if !crate::gestures::scroll_wheel(host.as_ref(), event) {
                let _: () = unsafe { msg_send![super(self), scrollWheel: event] };
            }
        }

        #[unsafe(method(rightMouseDown:))]
        fn right_mouse_down(&self, event: &NSEvent) {
            if !self.request_menu(event) {
                let _: () = unsafe { msg_send![super(self), rightMouseDown: event] };
            }
        }

        #[unsafe(method(updateTrackingAreas))]
        fn update_tracking_areas(&self) {
            if let Some(old) = self.ivars().tracking.borrow_mut().take() {
                self.removeTrackingArea(&old);
            }
            if self.tracks_hover() {
                use objc2::AnyThread;
                use objc2_app_kit::{NSTrackingArea, NSTrackingAreaOptions};
                let options =
                    NSTrackingAreaOptions::MouseEnteredAndExited | NSTrackingAreaOptions::ActiveInKeyWindow;
                // Not InVisibleRect: inside a scroll view's document, visibleRect can reach past
                // the view, and a card lit up while the pointer was over its neighbour.
                let area = unsafe {
                    NSTrackingArea::initWithRect_options_owner_userInfo(
                        NSTrackingArea::alloc(),
                        self.hover_rect(),
                        options,
                        Some(self),
                        None,
                    )
                };
                self.addTrackingArea(&area);
                *self.ivars().tracking.borrow_mut() = Some(area);
            }
            let _: () = unsafe { msg_send![super(self), updateTrackingAreas] };
        }

        #[unsafe(method(mouseEntered:))]
        fn mouse_entered(&self, _event: &NSEvent) {
            self.set_hovered(true);
        }

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, _event: &NSEvent) {
            self.set_hovered(false);
        }

        // nd frames the view in its parent's space, which a lifted hero has left.
        #[unsafe(method(setFrame:))]
        fn set_frame(&self, frame: NSRect) {
            let frame = match (&*self.ivars().lifted.borrow(), unsafe { self.superview() }) {
                (Some(home), Some(host)) => host.convertRect_fromView(frame, Some(&home.parent)),
                _ => frame,
            };
            let _: () = unsafe { msg_send![super(self), setFrame: frame] };
        }

        #[unsafe(method(setFrameSize:))]
        fn set_frame_size(&self, size: NSSize) {
            let _: () = unsafe { msg_send![super(self), setFrameSize: size] };
            if !self.ivars().props.borrow().transform.is_identity() {
                self.apply_transform();
            }
            // A `rounded_full()` radius is clamped to the size, so it follows the size.
            if self.ivars().props.borrow().corner_radius > 0. {
                self.setNeedsDisplay(true);
            }
        }

        // Runs under the view's effective appearance, so system colours resolve for it here and
        // again whenever the appearance changes.
        #[unsafe(method(updateLayer))]
        fn update_layer(&self) {
            let Some(layer) = self.layer() else { return };
            let props = self.current_look();
            let background = props.background.or(self.ivars().surface.get()).map(|c| c.ns_color().CGColor());
            layer.setBackgroundColor(background.as_deref());
            // `rounded_full()` asks for an unbounded radius; Core Animation draws a layer with one
            // past half its size, and everything in it, as nothing.
            let size = self.bounds().size;
            let radius = f64::from(props.corner_radius * self.ivars().radius_scale.get())
                .min(size.width.min(size.height) / 2.)
                .max(0.);
            layer.setCornerRadius(radius);
            layer.setMasksToBounds(props.clips);
            match props.border {
                Some((width, color)) => {
                    layer.setBorderWidth(width.into());
                    layer.setBorderColor(Some(&color.ns_color().CGColor()));
                }
                None => layer.setBorderWidth(0.),
            }
        }
    }
);

impl NdLayerView {
    pub(crate) fn new(mtm: MainThreadMarker, props: &ViewLook) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(LayerIvars::default());
        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] };
        this.setWantsLayer(true);
        this.setLayerContentsRedrawPolicy(NSViewLayerContentsRedrawPolicy::OnSetNeedsDisplay);
        this.set_props(props);
        this
    }

    /// What the view paints, the scope surface included.
    pub(crate) fn look(&self) -> ViewLook {
        let mut look = self.current_look();
        look.background = look.background.or(self.ivars().surface.get());
        look
    }

    pub(crate) fn set_surface(&self, surface: Option<Color>) {
        if self.ivars().surface.replace(surface) != surface {
            self.setNeedsDisplay(true);
        }
    }

    pub(crate) fn set_focusable(&self) {
        self.ivars().focusable.set(true);
    }

    pub(crate) fn set_radius_scale(&self, scale: f32) {
        if self.ivars().radius_scale.replace(scale) != scale {
            self.setNeedsDisplay(true);
        }
    }

    pub(crate) fn set_drag_region(&self, drag: bool) {
        self.ivars().drag.set(drag);
    }

    /// Right clicks (and control-clicks) on this view emit [`ContextMenuRequested`] through
    /// `emit`; `None` gives them back to AppKit.
    pub(crate) fn set_menu(&self, menu: Option<(nd_ui::events::Emit, crate::events::Wake)>) {
        *self.ivars().menu.borrow_mut() = menu;
    }

    /// The gestures this view's node listens for; `None` when it listens for none.
    pub(crate) fn set_gestures(&self, host: Option<GestureHost>) {
        *self.ivars().gestures.borrow_mut() = host;
    }

    pub(crate) fn gesture_host(&self) -> Option<GestureHost> {
        self.ivars().gestures.borrow().clone()
    }

    fn request_menu(&self, event: &NSEvent) -> bool {
        let menu = self.ivars().menu.borrow();
        let Some((emit, wake)) = menu.as_ref() else { return false };
        let at = self.convertPoint_fromView(event.locationInWindow(), None);
        emit.emit(ContextMenuRequested { node: emit.node(), x: at.x as f32, y: at.y as f32 });
        wake.wake();
        true
    }

    pub(crate) fn set_props(&self, props: &ViewLook) {
        // AppKit skips a hidden view and its subviews in hit testing and the key view loop, and
        // moves first responder out of it.
        if self.isHidden() != props.hidden {
            self.setHidden(props.hidden);
        }
        let had_hover = self.tracks_hover();
        let moved = self.ivars().props.borrow().transform != props.transform;
        *self.ivars().props.borrow_mut() = props.clone();
        self.setAlphaValue(self.current_look().opacity.into());
        if moved {
            self.lift(props.hero && !props.transform.is_identity());
            self.apply_transform();
        }
        if had_hover != self.tracks_hover() {
            self.updateTrackingAreas();
        }
        self.setNeedsDisplay(true);
    }

    /// Reports pointer enter and exit through `emit` (`.on_hover`); `None` stops.
    pub(crate) fn set_hover_target(&self, target: Option<(nd_ui::events::Emit, crate::events::Wake)>) {
        let had = self.tracks_hover();
        *self.ivars().hover_emit.borrow_mut() = target;
        if had != self.tracks_hover() {
            self.updateTrackingAreas();
        }
    }

    /// The part of the view on screen, where the pointer hovers it.
    fn hover_rect(&self) -> NSRect {
        let (b, v) = (self.bounds(), self.visibleRect());
        let x0 = b.origin.x.max(v.origin.x);
        let y0 = b.origin.y.max(v.origin.y);
        let x1 = (b.origin.x + b.size.width).min(v.origin.x + v.size.width);
        let y1 = (b.origin.y + b.size.height).min(v.origin.y + v.size.height);
        NSRect::new(NSPoint::new(x0, y0), NSSize::new((x1 - x0).max(0.), (y1 - y0).max(0.)))
    }

    fn tracks_hover(&self) -> bool {
        self.ivars().props.borrow().hover.is_some() || self.ivars().hover_emit.borrow().is_some()
    }

    fn set_hovered(&self, hovered: bool) {
        if self.ivars().hovered.replace(hovered) == hovered {
            return;
        }
        if self.ivars().props.borrow().hover.is_some() {
            self.setAlphaValue(self.current_look().opacity.into());
            self.setNeedsDisplay(true);
        }
        if let Some((emit, wake)) = self.ivars().hover_emit.borrow().as_ref() {
            emit.emit(nd_ui::HoverChanged { hovered });
            wake.wake();
        }
    }

    /// The look in effect: the hover look while the pointer is over the view.
    fn current_look(&self) -> ViewLook {
        let props = self.ivars().props.borrow();
        match (&props.hover, self.ivars().hovered.get()) {
            (Some(hover), true) => (**hover).clone(),
            _ => props.clone(),
        }
    }

    /// Takes a lifted hero off the frame view, for a node going away.
    pub(crate) fn land(&self) {
        if self.ivars().lifted.borrow_mut().take().is_some() {
            self.removeFromSuperview();
        }
    }

    /// Moves a hero in flight onto the window's frame view, and back into its place once it has
    /// landed. On the content view a Liquid Glass bar it lands in would still draw over it.
    fn lift(&self, up: bool) {
        if up == self.ivars().lifted.borrow().is_some() {
            return;
        }
        if up {
            let Some(home) = (unsafe { self.superview() }) else { return };
            let Some(content) = self.window().and_then(|w| w.contentView()) else { return };
            let Some(host) = (unsafe { content.superview() }) else { return };
            let siblings = home.subviews();
            let above = siblings
                .iter()
                .position(|v| std::ptr::eq(&*v, &**self as &NSView))
                .and_then(|i| siblings.iter().nth(i + 1));
            let frame = host.convertRect_fromView(self.frame(), Some(&home));
            self.removeFromSuperview();
            *self.ivars().lifted.borrow_mut() = Some(Home { parent: home, above });
            host.addSubview_positioned_relativeTo(self, objc2_app_kit::NSWindowOrderingMode::Above, None);
            let _: () = unsafe { msg_send![super(self), setFrame: frame] };
        } else {
            let Some(Home { parent: home, above }) = self.ivars().lifted.borrow_mut().take() else { return };
            let frame = unsafe { self.superview() }
                .map_or(self.frame(), |host| home.convertRect_fromView(self.frame(), Some(&host)));
            self.removeFromSuperview();
            match above.filter(|v| unsafe { v.superview() }.is_some_and(|p| p == home)) {
                Some(above) => home.addSubview_positioned_relativeTo(
                    self,
                    objc2_app_kit::NSWindowOrderingMode::Below,
                    Some(&above),
                ),
                None => home.addSubview(self),
            }
            let _: () = unsafe { msg_send![super(self), setFrame: frame] };
        }
    }

    /// The layer's transform: the offset, and the scale about the view's centre (the layer's
    /// anchor is its origin, as AppKit sets it for a layer-backed view).
    fn apply_transform(&self) {
        let Some(layer) = self.layer() else { return };
        let t = self.ivars().props.borrow().transform;
        let size = self.bounds().size;
        let s = f64::from(t.scale);
        // A lifted hero sits on the window's frame view, whose y axis points up.
        let down = if unsafe { self.superview() }.is_none_or(|p| p.isFlipped()) { 1. } else { -1. };
        layer.setAffineTransform(objc2_core_foundation::CGAffineTransform {
            a: s,
            b: 0.,
            c: 0.,
            d: s,
            tx: f64::from(t.offset.0) + size.width * (1. - s) / 2.,
            ty: down * f64::from(t.offset.1) + size.height * (1. - s) / 2.,
        });
    }
}

#[derive(Default)]
pub struct TextIvars {
    props: RefCell<Option<TextLook>>,
    /// The family the theme sets text in, in place of the system font.
    family: RefCell<Option<String>>,
    /// Where a click on a link run is reported, when the text has links.
    links: RefCell<Option<(nd_ui::events::Emit, crate::events::Wake)>>,
}

define_class!(
    // SAFETY: as for NdRootView.
    #[unsafe(super(NSView, NSResponder))]
    #[thread_kind = MainThreadOnly]
    #[ivars = TextIvars]
    pub struct NdTextView;

    impl NdTextView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty: NSRect) {
            if let Some(props) = self.ivars().props.borrow().as_ref() {
                attributed(props, self.family().as_deref()).drawWithRect_options_context(
                    self.bounds(),
                    draw_options(),
                    None,
                );
            }
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            if !self.click_link(event) {
                let _: () = unsafe { msg_send![super(self), mouseDown: event] };
            }
        }

        #[unsafe(method(resetCursorRects))]
        fn reset_cursor_rects(&self) {
            let props = self.ivars().props.borrow();
            let Some(props) = props.as_ref().filter(|p| p.has_links()) else { return };
            let text = attributed(props, self.family().as_deref());
            for rect in crate::rich::link_rects(&props.runs, &text, self.bounds().size.width) {
                self.addCursorRect_cursor(rect, &objc2_app_kit::NSCursor::pointingHandCursor());
            }
        }

        #[unsafe(method(isAccessibilityElement))]
        fn is_accessibility_element(&self) -> bool {
            true
        }

        #[unsafe(method_id(accessibilityRole))]
        fn accessibility_role(&self) -> Option<Retained<NSString>> {
            Some(unsafe { NSAccessibilityStaticTextRole }.retain())
        }

        #[unsafe(method_id(accessibilityValue))]
        fn accessibility_value(&self) -> Option<Retained<AnyObject>> {
            let props = self.ivars().props.borrow();
            props.as_ref().map(|p| Retained::into_super(Retained::into_super(NSString::from_str(&p.text))))
        }
    }
);

impl NdTextView {
    pub(crate) fn new(mtm: MainThreadMarker, props: &TextLook) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(TextIvars::default());
        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] };
        this.setWantsLayer(true);
        this.setLayerContentsRedrawPolicy(NSViewLayerContentsRedrawPolicy::OnSetNeedsDisplay);
        this.set_props(props);
        this
    }

    pub(crate) fn look(&self) -> Option<TextLook> {
        self.ivars().props.borrow().clone()
    }

    pub(crate) fn set_props(&self, props: &TextLook) {
        self.setAlphaValue(props.opacity.into());
        let links = props.has_links();
        *self.ivars().props.borrow_mut() = Some(props.clone());
        if links && let Some(window) = self.window() {
            window.invalidateCursorRectsForView(self);
        }
        self.setNeedsDisplay(true);
    }

    fn family(&self) -> Option<String> {
        self.ivars().family.borrow().clone()
    }

    /// Sets the text in `family` (a theme's font override); `None` is the system font.
    pub(crate) fn set_family(&self, family: Option<&str>) {
        let family = family.map(str::to_owned);
        if *self.ivars().family.borrow() != family {
            *self.ivars().family.borrow_mut() = family;
            self.setNeedsDisplay(true);
        }
    }

    /// Clicks on link runs report [`nd_ui::LinkClicked`] through `emit`.
    pub(crate) fn set_link_target(&self, target: Option<(nd_ui::events::Emit, crate::events::Wake)>) {
        *self.ivars().links.borrow_mut() = target;
    }

    fn click_link(&self, event: &NSEvent) -> bool {
        let Some(props) = self.ivars().props.borrow().clone().filter(TextLook::has_links) else { return false };
        let at = self.convertPoint_fromView(event.locationInWindow(), None);
        let text = attributed(&props, self.family().as_deref());
        let Some(link) = crate::rich::link_at(&text, self.bounds().size.width, at) else {
            return false;
        };
        if let Some((emit, wake)) = self.ivars().links.borrow().as_ref() {
            emit.emit(nd_ui::LinkClicked { link: link.clone().into() });
            wake.wake();
        }
        if props.opens_links {
            crate::rich::open(&link);
        }
        true
    }

    /// Measures in `family` when the caller knows the theme's (`Some`), else in the view's own.
    pub(crate) fn measure_in(&self, width: Option<f32>, family: Option<Option<&str>>) -> Size {
        let Some(props) = self.ivars().props.borrow().clone() else { return Size::default() };
        match family {
            Some(family) => measure_text(&props, family, width),
            None => measure_text(&props, self.family().as_deref(), width),
        }
    }
}

pub(crate) fn measure_text(props: &TextLook, family: Option<&str>, width: Option<f32>) -> Size {
    let bound = NSSize::new(width.map_or(f64::MAX, f64::from), f64::MAX);
    let rect = attributed(props, family).boundingRectWithSize_options_context(bound, draw_options(), None);
    Size { width: rect.size.width.ceil() as f32, height: rect.size.height.ceil() as f32 }
}

fn draw_options() -> NSStringDrawingOptions {
    NSStringDrawingOptions::UsesLineFragmentOrigin | NSStringDrawingOptions::UsesFontLeading
}

fn attributed(props: &TextLook, family: Option<&str>) -> Retained<NSAttributedString> {
    if !props.runs.is_empty() {
        return crate::rich::attributed(&props.runs, props.tabular_nums, &props.color.ns_color(), family);
    }
    let size = props.size.map_or_else(NSFont::systemFontSize, f64::from);
    let weight = props.weight.map_or(400., |w| w.0);
    let font = if let Some(font) = family.and_then(|f| crate::rich::family_font(f, size, weight, false)) {
        font
    } else if props.tabular_nums {
        NSFont::monospacedDigitSystemFontOfSize_weight(size, props.ns_weight())
    } else {
        NSFont::systemFontOfSize_weight(size, props.ns_weight())
    };
    let color = props.color.ns_color();
    let keys = unsafe { [NSFontAttributeName, NSForegroundColorAttributeName] };
    let values: [&AnyObject; 2] = [font.as_ref(), color.as_ref()];
    let attrs = NSDictionary::from_slices(&keys, &values);
    unsafe { NSAttributedString::new_with_attributes(&NSString::from_str(&props.text), &attrs) }
}

pub(crate) fn ns_rect(rect: nd_core::Rect) -> NSRect {
    NSRect::new(
        NSPoint::new(rect.origin.x.into(), rect.origin.y.into()),
        NSSize::new(rect.size.width.into(), rect.size.height.into()),
    )
}
