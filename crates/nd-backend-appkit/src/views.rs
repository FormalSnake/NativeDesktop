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

        // Runs under the view's effective appearance, so system colours resolve for it here and
        // again whenever the appearance changes.
        #[unsafe(method(updateLayer))]
        fn update_layer(&self) {
            let Some(layer) = self.layer() else { return };
            let props = self.ivars().props.borrow();
            let background = props.background.or(self.ivars().surface.get()).map(|c| c.ns_color().CGColor());
            layer.setBackgroundColor(background.as_deref());
            let radius = props.corner_radius * self.ivars().radius_scale.get();
            layer.setCornerRadius(radius.into());
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
        let mut look = self.ivars().props.borrow().clone();
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
        self.setAlphaValue(props.opacity.into());
        *self.ivars().props.borrow_mut() = props.clone();
        self.setNeedsDisplay(true);
    }
}

#[derive(Default)]
pub struct TextIvars {
    props: RefCell<Option<TextLook>>,
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
                attributed(props).drawWithRect_options_context(self.bounds(), draw_options(), None);
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
        *self.ivars().props.borrow_mut() = Some(props.clone());
        self.setNeedsDisplay(true);
    }

    /// The size the text takes when wrapped at `width` (unbounded when `None`), in points,
    /// rounded up so the last glyph is never clipped.
    pub(crate) fn measure(&self, width: Option<f32>) -> Size {
        let Some(props) = self.ivars().props.borrow().clone() else { return Size::default() };
        measure_text(&props, width)
    }
}

pub(crate) fn measure_text(props: &TextLook, width: Option<f32>) -> Size {
    let bound = NSSize::new(width.map_or(f64::MAX, f64::from), f64::MAX);
    let rect = attributed(props).boundingRectWithSize_options_context(bound, draw_options(), None);
    Size { width: rect.size.width.ceil() as f32, height: rect.size.height.ceil() as f32 }
}

fn draw_options() -> NSStringDrawingOptions {
    NSStringDrawingOptions::UsesLineFragmentOrigin | NSStringDrawingOptions::UsesFontLeading
}

fn attributed(props: &TextLook) -> Retained<NSAttributedString> {
    let size = props.size.map_or_else(NSFont::systemFontSize, f64::from);
    let font = if props.tabular_nums {
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
