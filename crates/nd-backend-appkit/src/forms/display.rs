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
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send};
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
use crate::look::{Color, SystemText, ns_font};

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

#[derive(Default)]
pub(crate) struct LabelIvars {
    /// The runs of rich text, as drawn; empty for plain text.
    runs: RefCell<Vec<crate::rich::RunLook>>,
    /// Reports link clicks; `.1` opens the link with the system too.
    links: RefCell<Option<(Ctx, bool)>>,
}

define_class!(
    // SAFETY: NSTextField has no subclassing requirements beyond calling super, and this type
    // does not implement Drop.
    #[unsafe(super(NSTextField))]
    #[thread_kind = MainThreadOnly]
    #[ivars = LabelIvars]
    pub(crate) struct NdLabelField;

    impl NdLabelField {
        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &objc2_app_kit::NSEvent) {
            if !self.click_link(event) {
                let _: () = unsafe { msg_send![super(self), mouseDown: event] };
            }
        }

        #[unsafe(method(resetCursorRects))]
        fn reset_cursor_rects(&self) {
            let runs = self.ivars().runs.borrow();
            if runs.iter().all(|r| r.link.is_none()) {
                return;
            }
            let (origin, width) = self.text_frame();
            let text = self.attributedStringValue();
            for mut rect in crate::rich::link_rects(&runs, &text, width) {
                rect.origin.x += origin.x;
                rect.origin.y += origin.y;
                self.addCursorRect_cursor(rect, &NSCursor::pointingHandCursor());
            }
        }
    }
);

impl NdLabelField {
    pub(crate) fn new(mtm: MainThreadMarker, text: &str) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(LabelIvars::default());
        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] };
        // What `labelWithString:` sets up.
        this.setStringValue(&NSString::from_str(text));
        this.setBezeled(false);
        this.setDrawsBackground(false);
        this.setEditable(false);
        this.setSelectable(false);
        this.setLineBreakMode(NSLineBreakMode::ByClipping);
        this
    }

    /// A label that wraps onto as many lines as its width needs, as `wrappingLabelWithString:`.
    pub(crate) fn new_wrapping(mtm: MainThreadMarker, text: &str) -> Retained<Self> {
        let this = Self::new(mtm, text);
        this.setSelectable(true);
        this.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
        this.setMaximumNumberOfLines(0);
        if let Some(cell) = this.cell() {
            cell.setUsesSingleLineMode(false);
            cell.setWraps(true);
        }
        this
    }

    /// Draws `runs` as the label's text, runs without a colour of their own in `base`. Empty
    /// runs leave the plain string value alone.
    pub(crate) fn set_runs(&self, runs: Vec<crate::rich::RunLook>, base: &NSColor, family: Option<&str>) {
        if !runs.is_empty() {
            self.setAttributedStringValue(&crate::rich::attributed(&runs, false, base, family));
        }
        *self.ivars().runs.borrow_mut() = runs;
        if let Some(window) = self.window() {
            window.invalidateCursorRectsForView(self);
        }
    }

    /// Where clicks on link runs are reported, and whether the system opens the link too.
    pub(crate) fn set_link_target(&self, target: Option<(Ctx, bool)>) {
        *self.ivars().links.borrow_mut() = target;
    }

    /// Where the cell draws the text in the field, and the width it wraps at.
    fn text_frame(&self) -> (objc2_foundation::NSPoint, f64) {
        let bounds = self.bounds();
        let rect = self.cell().map_or(bounds, |c| c.drawingRectForBounds(bounds));
        // The cell's text container keeps its default line fragment padding on each side.
        (objc2_foundation::NSPoint::new(rect.origin.x + 2., rect.origin.y), (rect.size.width - 4.).max(1.))
    }

    fn click_link(&self, event: &objc2_app_kit::NSEvent) -> bool {
        if self.ivars().runs.borrow().iter().all(|r| r.link.is_none()) {
            return false;
        }
        let (origin, width) = self.text_frame();
        let at = self.convertPoint_fromView(event.locationInWindow(), None);
        let at = objc2_foundation::NSPoint::new(at.x - origin.x, at.y - origin.y);
        let Some(link) = crate::rich::link_at(&self.attributedStringValue(), width, at) else { return false };
        if let Some((cx, open)) = self.ivars().links.borrow().as_ref() {
            cx.send(nd_ui::LinkClicked { link: link.clone().into() });
            if *open {
                crate::rich::open(&link);
            }
        }
        true
    }
}

pub(crate) struct LabelControl {
    field: Retained<NdLabelField>,
    cx: Ctx,
    /// The tone and explicit colour last applied, to resolve again when the appearance changes.
    tone: std::cell::Cell<(LabelTone, Option<nd_ui::prelude::Hsla>)>,
    /// A theme override of the accent here, which link and mention runs take.
    accent: std::cell::Cell<Option<nd_ui::prelude::Hsla>>,
    /// A theme's font family in place of the system font, and the props the font came from.
    family: RefCell<Option<String>>,
    font_props: RefCell<Option<(TextStyle, nd_ui::StyleRefinement)>>,
    rich: RefCell<Option<(nd_ui::RichText, TextStyle, Option<nd_ui::prelude::FontWeight>)>>,
}

/// The tone's system colour, legible on the window and control backgrounds.
fn tone_color(tone: LabelTone) -> Retained<NSColor> {
    match tone {
        LabelTone::Primary => NSColor::labelColor(),
        LabelTone::Secondary => Color::Legible(SystemText::Secondary).ns_color(),
        LabelTone::Danger => Color::Legible(SystemText::Danger).ns_color(),
    }
}

impl LabelControl {
    pub fn new(cx: Ctx, props: &LabelProps) -> Self {
        let field = NdLabelField::new(cx.mtm, &props.text);
        let this = Self {
            field,
            cx,
            tone: std::cell::Cell::new((props.tone, props.style.text.color)),
            accent: std::cell::Cell::new(None),
            family: RefCell::new(None),
            font_props: RefCell::new(None),
            rich: RefCell::new(None),
        };
        this.apply(None, props);
        this
    }

    fn apply(&self, old: Option<&LabelProps>, new: &LabelProps) {
        let f = &self.field;
        if old.is_none_or(|o| o.text != new.text) {
            f.setStringValue(&NSString::from_str(&new.text));
        }
        if old.is_none_or(|o| o.text_style != new.text_style || o.style.text != new.style.text) {
            *self.font_props.borrow_mut() = Some((new.text_style, new.style.clone()));
            self.apply_font();
        }
        if old.is_none_or(|o| o.tone != new.tone || o.style.text.color != new.style.text.color) {
            self.tone.set((new.tone, new.style.text.color));
            self.apply_tone();
        }
        let rich_changed = old.is_none_or(|o| {
            o.rich != new.rich || o.text_style != new.text_style || o.style.text != new.style.text || o.tone != new.tone
        });
        if rich_changed {
            *self.rich.borrow_mut() = new.rich.clone().map(|r| (r, new.text_style, new.style.text.font_weight));
            self.field.set_link_target(Some((self.cx.clone(), new.opens_links)));
            self.apply_rich();
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

impl LabelControl {
    fn tone_ns_color(&self) -> Retained<NSColor> {
        match self.tone.get() {
            (_, Some(c)) => Color::from(c).ns_color(),
            (tone, None) => tone_color(tone),
        }
    }

    fn apply_tone(&self) {
        self.field.setTextColor(Some(&self.tone_ns_color()));
    }

    /// The text style's system font, or the theme's family at its size and weight, under any
    /// font the element's style sets.
    fn apply_font(&self) {
        let Some((style, element)) = self.font_props.borrow().clone() else { return };
        let preferred = preferred_font(style);
        let size = preferred.pointSize();
        let font = ns_font(&element, size).or_else(|| {
            let family = self.family.borrow().clone()?;
            let weight = if style == TextStyle::Headline { 700. } else { 400. };
            crate::rich::family_font(&family, size, weight, false)
        });
        self.field.setFont(Some(&font.unwrap_or(preferred)));
    }

    /// Draws the rich text as an attributed string in the field's font and tone; plain text
    /// keeps the string value `apply` set.
    fn apply_rich(&self) {
        let rich = self.rich.borrow();
        let Some((rich, style, weight)) = rich.as_ref() else {
            self.field.set_runs(Vec::new(), &self.tone_ns_color(), None);
            return;
        };
        let size = self.field.font().map_or(NSFont::systemFontSize(), |f| f.pointSize()) as f32;
        let base = weight.map_or(if *style == TextStyle::Headline { 700. } else { 400. }, |w| w.0);
        let runs = crate::rich::resolve_system(rich, size, base, self.accent.get());
        self.field.set_runs(runs, &self.tone_ns_color(), self.family.borrow().as_deref());
    }
}

impl FormControl for LabelControl {
    fn view(&self) -> &NSView {
        &self.field
    }

    fn appearance_changed(&self) {
        self.apply_tone();
        self.apply_rich();
    }

    fn set_accent(&self, accent: Option<nd_ui::prelude::Hsla>) {
        if self.accent.replace(accent) != accent {
            self.apply_rich();
        }
    }

    fn set_font_family(&self, family: Option<&str>) {
        let family = family.map(str::to_owned);
        if *self.family.borrow() != family {
            *self.family.borrow_mut() = family;
            self.apply_font();
            self.apply_rich();
        }
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
