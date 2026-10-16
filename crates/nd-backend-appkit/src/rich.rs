//! Rich text on AppKit: a `RichText`'s runs as one `NSAttributedString`, and which link a point
//! in drawn text lands on. Text nodes, labels and chat bubbles all draw through here.

use nd_components::rich::text_color;
use nd_theme::Theme;
use nd_ui::prelude::Hsla;
use nd_ui::{RichText, SpanStyle, TextColor};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{AnyThread, Message};
use objc2_app_kit::{
    NSColor, NSFont, NSFontManager, NSFontTraitMask, NSFontAttributeName, NSFontDescriptorSymbolicTraits, NSFontWeight, NSFontWeightBold,
    NSFontWeightMedium, NSFontWeightRegular, NSFontWeightSemibold, NSForegroundColorAttributeName, NSLayoutManager,
    NSStrikethroughStyleAttributeName, NSTextContainer, NSTextStorage, NSUnderlineStyleAttributeName,
    NSWorkspace,
};
use objc2_foundation::{
    NSAttributedString, NSAttributedStringKey, NSDictionary, NSMutableAttributedString, NSNumber, NSPoint, NSRange,
    NSRect, NSSize, NSString, NSURL,
};

use crate::look::{Color, SystemText};

/// The attribute a link run carries its target under. Not `NSLinkAttributeName`, which makes
/// AppKit draw the run in its own link colour and handle the click itself.
fn link_key() -> Retained<NSString> {
    NSString::from_str("NDLink")
}

/// One run as the backend draws and reports it: its text, resolved colour, size and weight.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RunLook {
    pub text: String,
    /// `None` draws in the text's own colour.
    pub color: Option<Color>,
    pub size: f32,
    pub weight: f32,
    pub italic: bool,
    pub mono: bool,
    pub underline: bool,
    pub strikethrough: bool,
    pub link: Option<String>,
}

/// Each run of `rich` resolved under `theme`: `size` and `weight` are the text's own, which a
/// run's style changes. `ink` is the ink of the fill the text sits on, if any.
pub(crate) fn resolve(rich: &RichText, theme: &Theme, size: f32, weight: f32, ink: Option<Hsla>) -> Vec<RunLook> {
    runs(rich, size, weight, |c| Color::from(text_color(theme, c, ink)))
}

/// Each run of `rich` with its colour token as the system's own colour for it, legible on the
/// window: what a native label draws, which follows the appearance without a theme. `accent`
/// is a theme override of the accent at the label.
pub(crate) fn resolve_system(rich: &RichText, size: f32, weight: f32, accent: Option<Hsla>) -> Vec<RunLook> {
    runs(rich, size, weight, |c| match c {
        TextColor::Primary => Color::Label,
        TextColor::Secondary => Color::Legible(SystemText::Secondary),
        TextColor::Tertiary => Color::Legible(SystemText::Tertiary),
        TextColor::Accent => accent.map_or(Color::Legible(SystemText::Accent), Color::from),
        TextColor::Danger => Color::Legible(SystemText::Danger),
        TextColor::Success => Color::Legible(SystemText::Success),
        TextColor::Warning => Color::Legible(SystemText::Warning),
        TextColor::Custom(c) => Color::from(c),
    })
}

fn runs(rich: &RichText, size: f32, weight: f32, color: impl Fn(TextColor) -> Color) -> Vec<RunLook> {
    rich.spans()
        .iter()
        .map(|s| {
            let st: &SpanStyle = &s.style;
            RunLook {
                text: s.text.to_string(),
                color: st.color().map(&color),
                size: size * st.scale(),
                weight: st.weight(weight),
                italic: st.italic,
                mono: st.mono,
                underline: st.underlined(),
                strikethrough: st.strikethrough,
                link: st.link.as_ref().map(|l| l.to_string()),
            }
        })
        .collect()
}

pub(crate) fn ns_weight(weight: f32) -> NSFontWeight {
    unsafe {
        if weight >= 700. {
            NSFontWeightBold
        } else if weight >= 600. {
            NSFontWeightSemibold
        } else if weight >= 500. {
            NSFontWeightMedium
        } else {
            NSFontWeightRegular
        }
    }
}

/// `family` at `size` in the face nearest `weight` (CSS) and slant; `None` when the family is
/// not installed. A family the theme names in place of the system's (`ThemeRefinement::font`).
pub(crate) fn family_font(family: &str, size: f64, weight: f32, italic: bool) -> Option<Retained<NSFont>> {
    let mtm = objc2_foundation::MainThreadMarker::new()?;
    // NSFontManager's weights run 0 to 15, 5 regular and 9 bold.
    let weight = match weight as i32 {
        ..=150 => 2,
        151..=250 => 3,
        251..=350 => 4,
        351..=450 => 5,
        451..=550 => 6,
        551..=650 => 8,
        651..=750 => 9,
        751..=850 => 10,
        _ => 11,
    };
    let traits = if italic { NSFontTraitMask::ItalicFontMask } else { NSFontTraitMask(0) };
    NSFontManager::sharedFontManager(mtm).fontWithFamily_traits_weight_size(
        &NSString::from_str(family),
        traits,
        weight,
        size,
    )
}

/// The font for a run: in `family` when the theme names one, else the system font in its
/// weight, design (monospaced) and slant.
pub(crate) fn run_font(run: &RunLook, tabular: bool, family: Option<&str>) -> Retained<NSFont> {
    let size = f64::from(run.size);
    if let Some(font) = family.filter(|_| !run.mono).and_then(|f| family_font(f, size, run.weight, run.italic)) {
        return font;
    }
    let weight = ns_weight(run.weight);
    let font = if run.mono {
        NSFont::monospacedSystemFontOfSize_weight(size, weight)
    } else if tabular {
        NSFont::monospacedDigitSystemFontOfSize_weight(size, weight)
    } else {
        NSFont::systemFontOfSize_weight(size, weight)
    };
    if !run.italic {
        return font;
    }
    let descriptor = font.fontDescriptor();
    let traits = descriptor.symbolicTraits() | NSFontDescriptorSymbolicTraits::TraitItalic;
    NSFont::fontWithDescriptor_size(&descriptor.fontDescriptorWithSymbolicTraits(traits), size).unwrap_or(font)
}

/// The runs as one attributed string; runs without a colour of their own take `base`, and
/// text is set in `family` when the theme names one.
pub(crate) fn attributed(
    runs: &[RunLook],
    tabular: bool,
    base: &NSColor,
    family: Option<&str>,
) -> Retained<NSAttributedString> {
    let out = NSMutableAttributedString::new();
    for run in runs {
        let font = run_font(run, tabular, family);
        let color = run.color.map_or_else(|| base.retain(), Color::ns_color);
        let mut keys: Vec<&NSAttributedStringKey> = unsafe { vec![NSFontAttributeName, NSForegroundColorAttributeName] };
        let mut values: Vec<Retained<AnyObject>> =
            vec![Retained::into_super(Retained::into_super(font)), Retained::into_super(Retained::into_super(color))];
        let one = NSNumber::new_isize(1);
        if run.underline {
            keys.push(unsafe { NSUnderlineStyleAttributeName });
            values.push(Retained::into_super(Retained::into_super(Retained::into_super(one.clone()))));
        }
        if run.strikethrough {
            keys.push(unsafe { NSStrikethroughStyleAttributeName });
            values.push(Retained::into_super(Retained::into_super(Retained::into_super(one.clone()))));
        }
        let key = link_key();
        if let Some(link) = &run.link {
            keys.push(&key);
            values.push(Retained::into_super(Retained::into_super(NSString::from_str(link))));
        }
        let values: Vec<&AnyObject> = values.iter().map(|v| &**v).collect();
        let attrs = NSDictionary::from_slices(&keys, &values);
        let piece = unsafe { NSAttributedString::new_with_attributes(&NSString::from_str(&run.text), &attrs) };
        out.appendAttributedString(&piece);
    }
    Retained::into_super(out)
}

/// `text` laid out from the origin, wrapped at `width`, as `NSStringDrawing` draws it.
struct Laid {
    _storage: Retained<NSTextStorage>,
    layout: Retained<NSLayoutManager>,
    container: Retained<NSTextContainer>,
}

fn lay_out(text: &NSAttributedString, width: f64) -> Option<Laid> {
    let storage = NSTextStorage::new();
    storage.appendAttributedString(text);
    let layout = NSLayoutManager::new();
    let container = NSTextContainer::initWithSize(NSTextContainer::alloc(), NSSize::new(width.max(1.), 1e7));
    container.setLineFragmentPadding(0.);
    layout.addTextContainer(&container);
    storage.addLayoutManager(&layout);
    layout.ensureLayoutForTextContainer(&container);
    Some(Laid { _storage: storage, layout, container })
}

/// Where each link run of `runs` (drawn as `text`) sits, wrapped at `width`: one rect per run,
/// bounding every line it spans.
pub(crate) fn link_rects(runs: &[RunLook], text: &NSAttributedString, width: f64) -> Vec<NSRect> {
    let Some(laid) = lay_out(text, width) else { return Vec::new() };
    let mut at = 0;
    let mut out = Vec::new();
    for run in runs {
        let len = run.text.encode_utf16().count();
        if run.link.is_some() {
            let chars = NSRange::new(at, len);
            let glyphs =
                unsafe { laid.layout.glyphRangeForCharacterRange_actualCharacterRange(chars, std::ptr::null_mut()) };
            out.push(laid.layout.boundingRectForGlyphRange_inTextContainer(glyphs, &laid.container));
        }
        at += len;
    }
    out
}

/// The link under `point`, in the coordinates `text` is laid out in from the origin, wrapped
/// at `width`.
pub(crate) fn link_at(text: &NSAttributedString, width: f64, point: NSPoint) -> Option<String> {
    let Laid { layout, container, .. } = lay_out(text, width)?;
    if !objc2_foundation::NSPointInRect(point, layout.usedRectForTextContainer(&container)) {
        return None;
    }
    let mut fraction = 0.;
    let index = unsafe {
        layout.characterIndexForPoint_inTextContainer_fractionOfDistanceBetweenInsertionPoints(
            point,
            &container,
            &mut fraction,
        )
    };
    if index >= text.length() {
        return None;
    }
    let value = unsafe { text.attribute_atIndex_effectiveRange(&link_key(), index, std::ptr::null_mut()) }?;
    value.downcast::<NSString>().ok().map(|s| s.to_string())
}

/// Opens `link` with the system: a URL in its app, anything else as a path.
pub(crate) fn open(link: &str) {
    let url = NSURL::URLWithString(&NSString::from_str(link)).filter(|u| u.scheme().is_some());
    let url = url.unwrap_or_else(|| NSURL::fileURLWithPath(&NSString::from_str(link)));
    NSWorkspace::sharedWorkspace().openURL(&url);
}
