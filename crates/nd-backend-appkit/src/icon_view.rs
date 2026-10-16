//! `icon` on its own: an `NSImageView` showing the icon's SF Symbol (or its drawn Lucide
//! shape), scaled into the node's square and tinted with the element's text colour or the
//! theme's secondary text.

use nd_components::{IconKind, IconProps};
use nd_core::Size;
use nd_theme::Theme;
use objc2::rc::Retained;
use objc2::{MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSImageScaling, NSImageView, NSView};
use objc2_foundation::{MainThreadMarker, NSPoint};

use crate::custom::{CustomCx, CustomNative, CustomPaint, CustomWidget};
use crate::look::Color;

pub(crate) fn register() {
    crate::custom::register::<IconKind>(IconWidget);
}

struct IconWidget;

define_class!(
    // SAFETY: NSImageView has no subclassing requirements beyond calling super, and this type
    // does not implement Drop.
    #[unsafe(super(NSImageView))]
    #[thread_kind = MainThreadOnly]
    struct NdIconView;

    impl NdIconView {
        /// A mark, not a control: presses and hovers go to whatever it sits on (a tappable
        /// video poster, a row), as a label's would.
        #[unsafe(method(hitTest:))]
        fn hit_test(&self, _point: NSPoint) -> *mut NSView {
            std::ptr::null_mut()
        }
    }
);

impl NdIconView {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        unsafe { msg_send![Self::alloc(mtm), init] }
    }
}

impl CustomWidget for IconWidget {
    fn create(&self, cx: &CustomCx<'_>) -> Box<dyn CustomNative> {
        let view = NdIconView::new(cx.mtm());
        view.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
        let mut native = IconNative { view, props: props(cx).clone(), theme: cx.theme().clone() };
        native.apply(true);
        Box::new(native)
    }
}

fn props<'a>(cx: &'a CustomCx<'_>) -> &'a IconProps {
    cx.props().downcast_ref::<IconProps>().expect("icon node needs IconProps")
}

struct IconNative {
    view: Retained<NdIconView>,
    props: IconProps,
    theme: Theme,
}

impl IconNative {
    fn tint(&self) -> nd_ui::prelude::Hsla {
        self.props.style.text.color.unwrap_or(self.theme.text.secondary)
    }

    fn apply(&mut self, image: bool) {
        if image {
            self.view.setImage(crate::icons::symbol(self.props.icon.lucide()).as_deref());
        }
        self.view.setContentTintColor(Some(&Color::from(self.tint()).ns_color()));
        self.view.setEnabled(self.props.enabled);
        self.view.setToolTip(self.props.tooltip.as_deref().map(objc2_foundation::NSString::from_str).as_deref());
    }
}

impl CustomNative for IconNative {
    fn view(&self) -> &NSView {
        &self.view
    }

    fn update(&mut self, cx: &CustomCx<'_>) {
        let new = props(cx).clone();
        let image = new.icon != self.props.icon;
        self.props = new;
        self.apply(image);
    }

    fn set_theme(&mut self, theme: &Theme) {
        self.theme = theme.clone();
        self.apply(false);
    }

    fn measure(&self, _width: Option<f32>) -> Size {
        Size { width: self.props.size, height: self.props.size }
    }

    fn paints(&self) -> Vec<CustomPaint> {
        vec![CustomPaint { label: format!("icon {}", self.props.icon.lucide()), color: self.tint(), under: Vec::new(), text: None }]
    }
}
