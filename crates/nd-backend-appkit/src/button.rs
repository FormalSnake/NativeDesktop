//! The native push button: AppKit's own `NSButton`, its action routed into the event queue.
//! Variants and element styles map to what `NSButton` can express (default button, destructive
//! action, bezel and content tint, font, SF Symbol image); anything else (a corner radius) is
//! ignored with one dev warning naming `.drawn()`.

use std::cell::{Cell, RefCell};

use nd_core::Size;
use nd_ui::events::Emit;
use nd_ui::prelude::Hsla;
use nd_ui::{Bezel, ButtonProps, ButtonVariant, ClickEvent};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::runtime::Sel;
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSBezelStyle, NSButton, NSCellImagePosition, NSColor, NSFont, NSFontAttributeName, NSForegroundColorAttributeName,
};
use objc2_foundation::{MainThreadMarker, NSAttributedString, NSDictionary, NSObject, NSObjectProtocol, NSString};

use crate::events::Wake;
use crate::look::{Color, ns_font};

pub struct TargetIvars {
    emit: Emit,
    wake: Wake,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements and this type does not implement Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = TargetIvars]
    pub struct NdButtonTarget;

    unsafe impl NSObjectProtocol for NdButtonTarget {}

    impl NdButtonTarget {
        #[unsafe(method(press:))]
        fn press(&self, _sender: &NSObject) {
            self.ivars().emit.emit(ClickEvent);
            self.ivars().wake.wake();
        }
    }
);

thread_local! {
    static WARNED_RADIUS: Cell<bool> = const { Cell::new(false) };
}

/// An `NSButton` and its target, which AppKit holds weakly.
pub(crate) struct NativeButtonView {
    pub button: Retained<NSButton>,
    target: Retained<NdButtonTarget>,
    props: RefCell<ButtonProps>,
    /// The accent a theme override sets for the button's subtree.
    accent: Cell<Option<Hsla>>,
}

impl NativeButtonView {
    pub fn new(mtm: MainThreadMarker, emit: Emit, wake: Wake, props: &ButtonProps) -> Self {
        let target = NdButtonTarget::alloc(mtm).set_ivars(TargetIvars { emit, wake });
        let target: Retained<NdButtonTarget> = unsafe { msg_send![super(target), init] };
        let button = unsafe {
            NSButton::buttonWithTitle_target_action(
                &NSString::from_str(&props.label),
                Some(&target),
                Some(sel!(press:)),
                mtm,
            )
        };
        let this = Self { button, target, props: RefCell::new(props.clone()), accent: Cell::new(None) };
        this.apply(None, props);
        this
    }

    /// The target and action a toolbar image item uses to press this button.
    pub fn action(&self) -> (&NSObject, Sel) {
        (&self.target, sel!(press:))
    }

    pub fn props(&self) -> ButtonProps {
        self.props.borrow().clone()
    }

    /// Icon only: shown as the toolbar's own image item when inside a toolbar.
    pub fn is_icon_only(&self) -> bool {
        let p = self.props.borrow();
        p.label.is_empty() && p.icon.is_some()
    }

    pub fn update(&self, old: &ButtonProps, new: &ButtonProps) {
        self.apply(Some(old), new);
        *self.props.borrow_mut() = new.clone();
    }

    /// The theme's accent at this button, when a theme override set one.
    pub fn set_accent(&self, accent: Option<Hsla>) {
        if self.accent.replace(accent) != accent {
            self.apply_colors(&self.props.borrow());
        }
    }

    fn apply(&self, old: Option<&ButtonProps>, new: &ButtonProps) {
        let b = &self.button;
        if old.is_none_or(|o| o.label != new.label) {
            b.setTitle(&NSString::from_str(&new.label));
        }
        if old.is_none_or(|o| o.icon != new.icon || o.label != new.label) {
            let image = new.icon.as_deref().and_then(crate::icons::symbol);
            b.setImage(image.as_deref());
            b.setImagePosition(match (&image, new.label.is_empty()) {
                (Some(_), true) => NSCellImagePosition::ImageOnly,
                (Some(_), false) => NSCellImagePosition::ImageLeading,
                (None, _) => NSCellImagePosition::NoImage,
            });
        }
        if old.is_none_or(|o| o.tooltip != new.tooltip) {
            b.setToolTip(new.tooltip.as_deref().map(NSString::from_str).as_deref());
        }
        if old.is_none_or(|o| o.bezel != new.bezel) {
            match new.bezel {
                Bezel::Glass if crate::glass::has_liquid_glass() => {
                    b.setBordered(true);
                    b.setBezelStyle(NSBezelStyle::Glass);
                }
                Bezel::Borderless => b.setBordered(false),
                Bezel::Toolbar => {
                    b.setBordered(true);
                    b.setBezelStyle(NSBezelStyle::Toolbar);
                }
                Bezel::Automatic | Bezel::Bordered | Bezel::Glass => {
                    b.setBordered(true);
                    b.setBezelStyle(NSBezelStyle::Push);
                }
            }
        }
        if old.is_none_or(|o| o.variant != new.variant) {
            let key = if new.variant == ButtonVariant::Primary { "\r" } else { "" };
            b.setKeyEquivalent(&NSString::from_str(key));
            b.setHasDestructiveAction(new.variant == ButtonVariant::Destructive);
        }
        if old.is_none_or(|o| o.style.text != new.style.text) {
            let font = ns_font(&new.style, NSFont::systemFontSize());
            b.setFont(Some(&font.unwrap_or_else(|| NSFont::systemFontOfSize(NSFont::systemFontSize()))));
        }
        if new.style.corner_radii.top_left.is_some() && cfg!(debug_assertions) && !WARNED_RADIUS.replace(true) {
            eprintln!("nd: NSButton cannot draw a custom corner radius; it is ignored. Use .drawn() for that look.");
        }
        self.apply_colors(new);
    }

    /// Background becomes the bezel colour, text colour the content tint; an accent from the
    /// theme fills the default and accent buttons when the element sets no colour itself.
    fn apply_colors(&self, props: &ButtonProps) {
        let explicit = props.style.background.as_ref().and_then(|f| f.color()).and_then(|b| b.as_solid());
        let themed = matches!(props.variant, ButtonVariant::Primary | ButtonVariant::Accent)
            .then(|| self.accent.get())
            .flatten();
        let bezel: Option<Retained<NSColor>> = match (explicit, themed, props.variant) {
            (Some(c), _, _) => Some(Color::from(c).ns_color()),
            (None, Some(c), _) => Some(Color::from(c).ns_color()),
            (None, None, ButtonVariant::Accent) => Some(NSColor::controlAccentColor()),
            _ => None,
        };
        self.button.setBezelColor(bezel.as_deref());
        let tint = props.style.text.color.map(|c| Color::from(c).ns_color());
        self.button.setContentTintColor(tint.as_deref());
        // A push button's title ignores the content tint; its attributed title carries the colour.
        match &tint {
            Some(color) => {
                let font = self.button.font().unwrap_or_else(|| NSFont::systemFontOfSize(NSFont::systemFontSize()));
                let keys = unsafe { [NSFontAttributeName, NSForegroundColorAttributeName] };
                let values: [&AnyObject; 2] = [font.as_ref(), color.as_ref()];
                let attrs = NSDictionary::from_slices(&keys, &values);
                let title =
                    unsafe { NSAttributedString::new_with_attributes(&NSString::from_str(&props.label), &attrs) };
                self.button.setAttributedTitle(&title);
            }
            None => self.button.setTitle(&NSString::from_str(&props.label)),
        }
    }

    /// The control's own size for its title and the system control size.
    pub fn measure(&self) -> Size {
        let size = self.button.fittingSize();
        Size { width: size.width as f32, height: size.height as f32 }
    }
}
