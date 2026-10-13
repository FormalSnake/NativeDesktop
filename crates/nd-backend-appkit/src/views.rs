//! The primitive views behind retained nodes: [`NdRootView`] (a window's content root),
//! [`NdLayerView`] (`div`) and [`NdTextView`] (`text`). All are flipped, so frames from `nd-ui`
//! (top-left origin) apply unchanged.

use std::cell::{OnceCell, RefCell};
use std::rc::Rc;

use nd_core::Size;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{DefinedClass, MainThreadOnly, Message, define_class, msg_send};
use objc2_app_kit::{
    NSAccessibilityStaticTextRole, NSAttributedStringNSExtendedStringDrawing, NSFont, NSFontAttributeName,
    NSForegroundColorAttributeName, NSResponder, NSStringDrawingOptions, NSView, NSViewLayerContentsRedrawPolicy,
};
use objc2_foundation::{MainThreadMarker, NSAttributedString, NSDictionary, NSPoint, NSRect, NSSize, NSString};

use crate::look::{TextLook, ViewLook};

pub(crate) type LayoutHook = Rc<dyn Fn(Size)>;

#[derive(Default)]
pub struct RootIvars {
    on_layout: RefCell<Option<LayoutHook>>,
    content: OnceCell<Retained<NdLayerView>>,
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

    /// The window's `contentLayoutRect` in this (full-size, flipped) view.
    fn safe_rect(&self) -> NSRect {
        let bounds = self.bounds();
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

#[derive(Default)]
pub struct LayerIvars {
    props: RefCell<ViewLook>,
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

        // Runs under the view's effective appearance, so system colours resolve for it here and
        // again whenever the appearance changes.
        #[unsafe(method(updateLayer))]
        fn update_layer(&self) {
            let Some(layer) = self.layer() else { return };
            let props = self.ivars().props.borrow();
            let background = props.background.map(|c| c.ns_color().CGColor());
            layer.setBackgroundColor(background.as_deref());
            layer.setCornerRadius(props.corner_radius.into());
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
