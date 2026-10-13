//! `label` as an `NSTextField` label, `link` as a borderless `NSButton`, and `progress` /
//! `spinner` as `NSProgressIndicator`.
//!
//! A link is a button rather than a text field with `NSLinkAttributeName`: a field routes link
//! clicks through its field editor's delegate, while a button has the target and action every
//! other control here reports through, and `perform(Click)` can send that action directly.

use std::cell::RefCell;
use std::rc::Rc;

use nd_components::{LabelProps, LabelTone, LinkActivated, LinkProps, ProgressProps, ProgressStyle, TextStyle};
use nd_core::Size;
use nd_ui::backend::Props;
use nd_ui::semantics::{ActionError, Probe, SemanticAction};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSAccessibility, NSAccessibilityLinkRole, NSButton, NSColor, NSCursor, NSFont, NSFontAttributeName,
    NSFontTextStyle, NSFontTextStyleBody, NSFontTextStyleCallout, NSFontTextStyleCaption1, NSFontTextStyleCaption2,
    NSFontTextStyleFootnote, NSFontTextStyleHeadline, NSFontTextStyleLargeTitle, NSFontTextStyleSubheadline,
    NSFontTextStyleTitle1, NSFontTextStyleTitle2, NSFontTextStyleTitle3, NSForegroundColorAttributeName,
    NSLineBreakMode, NSProgressIndicator, NSProgressIndicatorStyle, NSTextField, NSUnderlineStyleAttributeName, NSView,
    NSWorkspace,
};
use objc2_foundation::{MainThreadMarker, NSAttributedString, NSDictionary, NSNumber, NSRect, NSSize, NSString, NSURL};

use super::{Ctx, FormControl, NdControlTarget, apply_common, fitting, props, send_action, set_accessibility_label};
use crate::look::{Color, ns_font};

fn text_style(style: TextStyle) -> &'static NSFontTextStyle {
    unsafe {
        match style {
            TextStyle::LargeTitle => NSFontTextStyleLargeTitle,
            TextStyle::Title1 => NSFontTextStyleTitle1,
            TextStyle::Title2 => NSFontTextStyleTitle2,
            TextStyle::Title3 => NSFontTextStyleTitle3,
            TextStyle::Headline => NSFontTextStyleHeadline,
            TextStyle::Subheadline => NSFontTextStyleSubheadline,
            TextStyle::Body => NSFontTextStyleBody,
            TextStyle::Callout => NSFontTextStyleCallout,
            TextStyle::Footnote => NSFontTextStyleFootnote,
            TextStyle::Caption1 => NSFontTextStyleCaption1,
            TextStyle::Caption2 => NSFontTextStyleCaption2,
        }
    }
}

/// The system's font for `style`, before any element override.
pub(crate) fn preferred_font(style: TextStyle) -> Retained<NSFont> {
    unsafe { NSFont::preferredFontForTextStyle_options(text_style(style), &NSDictionary::new()) }
}

pub(crate) struct LabelControl {
    field: Retained<NSTextField>,
}

impl LabelControl {
    pub fn new(cx: Ctx, props: &LabelProps) -> Self {
        let field = NSTextField::labelWithString(&NSString::from_str(&props.text), cx.mtm);
        let this = Self { field };
        this.apply(None, props);
        this
    }

    fn apply(&self, old: Option<&LabelProps>, new: &LabelProps) {
        let f = &self.field;
        if old.is_none_or(|o| o.text != new.text) {
            f.setStringValue(&NSString::from_str(&new.text));
        }
        if old.is_none_or(|o| o.text_style != new.text_style || o.style.text != new.style.text) {
            let preferred = preferred_font(new.text_style);
            f.setFont(Some(&ns_font(&new.style, preferred.pointSize()).unwrap_or(preferred)));
        }
        if old.is_none_or(|o| o.tone != new.tone || o.style.text.color != new.style.text.color) {
            let color = match (new.style.text.color, new.tone) {
                (Some(c), _) => Color::from(c).ns_color(),
                (None, LabelTone::Primary) => NSColor::labelColor(),
                (None, LabelTone::Secondary) => NSColor::secondaryLabelColor(),
                (None, LabelTone::Danger) => NSColor::systemRedColor(),
            };
            f.setTextColor(Some(&color));
        }
        if old.is_none_or(|o| o.selectable != new.selectable) {
            f.setSelectable(new.selectable);
        }
        if old.is_none_or(|o| o.wraps != new.wraps) {
            if new.wraps {
                f.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
                f.setMaximumNumberOfLines(0);
            } else {
                f.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
                f.setMaximumNumberOfLines(1);
            }
            if let Some(cell) = f.cell() {
                cell.setUsesSingleLineMode(!new.wraps);
                cell.setWraps(new.wraps);
            }
        }
        apply_common(Some(f), f, old.map(|o| (o.enabled, &o.tooltip)), new.enabled, &new.tooltip);
    }
}

impl FormControl for LabelControl {
    fn view(&self) -> &NSView {
        &self.field
    }

    fn update(&mut self, old: &Props, new: &Props) -> bool {
        self.apply(Some(props(old)), props(new));
        true
    }

    fn measure(&self, width: Option<f32>) -> Size {
        if let (Some(width), true) = (width, self.field.maximumNumberOfLines() == 0)
            && let Some(cell) = self.field.cell()
        {
            let bounds = NSRect::new(objc2_foundation::NSPoint::ZERO, NSSize::new(f64::from(width), 100_000.));
            let size = cell.cellSizeForBounds(bounds);
            return Size { width: size.width.ceil() as f32, height: size.height.ceil() as f32 };
        }
        fitting(&self.field)
    }

    fn probe(&self, probe: &mut Probe) {
        probe.enabled = Some(self.field.isEnabled());
    }
}

define_class!(
    // SAFETY: NSButton has no subclassing requirements beyond calling super, and this type does
    // not implement Drop.
    #[unsafe(super(NSButton))]
    #[thread_kind = MainThreadOnly]
    pub(crate) struct NdLinkButton;

    impl NdLinkButton {
        #[unsafe(method(resetCursorRects))]
        fn reset_cursor_rects(&self) {
            self.addCursorRect_cursor(self.bounds(), &NSCursor::pointingHandCursor());
        }
    }
);

impl NdLinkButton {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        unsafe { msg_send![Self::alloc(mtm), init] }
    }
}

pub(crate) struct LinkControl {
    button: Retained<NdLinkButton>,
    _target: Retained<NdControlTarget>,
    props: Rc<RefCell<LinkProps>>,
}

impl LinkControl {
    pub fn new(cx: Ctx, props: &LinkProps) -> Self {
        let shared = Rc::new(RefCell::new(props.clone()));
        let current = shared.clone();
        let target = NdControlTarget::new(cx.mtm, move |_| {
            let p = current.borrow();
            if p.open_external
                && let Some(url) = p.url.as_deref().and_then(|u| NSURL::URLWithString(&NSString::from_str(u)))
            {
                NSWorkspace::sharedWorkspace().openURL(&url);
            }
            cx.send(LinkActivated { url: p.url.clone() });
        });
        let button = NdLinkButton::new(MainThreadMarker::new().expect("main thread"));
        button.setBordered(false);
        target.wire(&button);
        button.setAccessibilityRole(Some(unsafe { NSAccessibilityLinkRole }));
        let this = Self { button, _target: target, props: shared };
        this.apply(None, props);
        this
    }

    fn apply(&self, old: Option<&LinkProps>, new: &LinkProps) {
        let b = &self.button;
        apply_common(Some(b), b, old.map(|o| (o.enabled, &o.tooltip)), new.enabled, &new.tooltip);
        if old.is_none_or(|o| o.title != new.title || o.enabled != new.enabled || o.style.text != new.style.text) {
            let size = NSFont::systemFontSize();
            let font = ns_font(&new.style, size).or_else(|| b.font()).unwrap_or_else(|| NSFont::systemFontOfSize(size));
            let color = match (new.enabled, new.style.text.color) {
                (false, _) => NSColor::disabledControlTextColor(),
                (true, Some(c)) => Color::from(c).ns_color(),
                (true, None) => NSColor::linkColor(),
            };
            let underline = NSNumber::new_isize(1);
            let keys = unsafe { [NSFontAttributeName, NSForegroundColorAttributeName, NSUnderlineStyleAttributeName] };
            let values: [&AnyObject; 3] = [font.as_ref(), color.as_ref(), underline.as_ref()];
            let attrs = NSDictionary::from_slices(&keys, &values);
            let title = unsafe { NSAttributedString::new_with_attributes(&NSString::from_str(&new.title), &attrs) };
            b.setAttributedTitle(&title);
        }
        if old.is_none_or(|o| o.title != new.title) {
            set_accessibility_label(b, Some(&new.title));
        }
        *self.props.borrow_mut() = new.clone();
    }
}

impl FormControl for LinkControl {
    fn view(&self) -> &NSView {
        &self.button
    }

    fn update(&mut self, old: &Props, new: &Props) -> bool {
        self.apply(Some(props(old)), props(new));
        true
    }

    fn measure(&self, _width: Option<f32>) -> Size {
        fitting(&self.button)
    }

    fn probe(&self, probe: &mut Probe) {
        probe.enabled = Some(self.button.isEnabled());
    }

    fn perform(&self, action: &SemanticAction) -> Option<Result<(), ActionError>> {
        (*action == SemanticAction::Click).then(|| send_action(&self.button))
    }
}

pub(crate) struct ProgressControl {
    indicator: Retained<NSProgressIndicator>,
    props: ProgressProps,
}

impl ProgressControl {
    pub fn new(cx: Ctx, props: &ProgressProps) -> Self {
        let this = Self { indicator: NSProgressIndicator::new(cx.mtm), props: props.clone() };
        this.apply(None, props);
        this
    }

    /// A bar is determinate when it has a value; a spinner never is, and a circular one needs a
    /// value to draw its pie.
    fn indeterminate(p: &ProgressProps) -> bool {
        p.value.is_none() || p.presentation == ProgressStyle::Spinner
    }

    fn apply(&self, old: Option<&ProgressProps>, new: &ProgressProps) {
        let i = &self.indicator;
        if old.is_none_or(|o| o.presentation != new.presentation) {
            i.setStyle(match new.presentation {
                ProgressStyle::Bar => NSProgressIndicatorStyle::Bar,
                ProgressStyle::Spinner | ProgressStyle::Circular => NSProgressIndicatorStyle::Spinning,
            });
        }
        let indeterminate = Self::indeterminate(new);
        i.setIndeterminate(indeterminate);
        if indeterminate {
            unsafe { i.startAnimation(None) };
        } else {
            unsafe { i.stopAnimation(None) };
            i.setMinValue(0.);
            i.setMaxValue(100.);
            i.setDoubleValue(f64::from(new.value.unwrap_or(0.).clamp(0., 1.)) * 100.);
        }
        apply_common(None, i, old.map(|o| (o.enabled, &o.tooltip)), new.enabled, &new.tooltip);
        if old.is_none_or(|o| o.enabled != new.enabled) {
            i.setAccessibilityEnabled(new.enabled);
        }
        if old.is_none_or(|o| o.label != new.label) {
            set_accessibility_label(i, new.label.as_deref());
        }
    }
}

impl FormControl for ProgressControl {
    fn view(&self) -> &NSView {
        &self.indicator
    }

    fn update(&mut self, old: &Props, new: &Props) -> bool {
        let new: &ProgressProps = props(new);
        self.apply(Some(props(old)), new);
        self.props = new.clone();
        true
    }

    fn measure(&self, _width: Option<f32>) -> Size {
        fitting(&self.indicator)
    }

    fn probe(&self, probe: &mut Probe) {
        probe.enabled = Some(self.props.enabled);
        if !Self::indeterminate(&self.props) {
            probe.value = Some(format!("{:.0}%", self.indicator.doubleValue()));
        }
    }
}
