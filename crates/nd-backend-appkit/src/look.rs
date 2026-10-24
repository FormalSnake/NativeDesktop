//! What the AppKit views draw, resolved from `nd-ui` props. Layout fields of a style arrive as
//! frames; only the visual part is read here.

use nd_gpui::gpui::{Overflow, Visibility};
use nd_ui::StyleRefinement;
use nd_ui::prelude::{AbsoluteLength, FontWeight, Hsla, px};
use objc2::rc::Retained;
use objc2_app_kit::{NSColor, NSFontWeight};

/// A colour as the views resolve it: system colours are looked up at draw time, under the
/// view's effective appearance, so light/dark changes need no update.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Color {
    Rgba(f32, f32, f32, f32),
    WindowBackground,
    Label,
    /// A system text colour made legible: see [`legible`].
    Legible(SystemText),
}

/// The system colours behind the text tokens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SystemText {
    Secondary,
    Tertiary,
    Accent,
    Danger,
    Success,
    Warning,
}

impl SystemText {
    fn ns_color(self) -> Retained<NSColor> {
        match self {
            SystemText::Secondary => NSColor::secondaryLabelColor(),
            SystemText::Tertiary => NSColor::tertiaryLabelColor(),
            SystemText::Accent => NSColor::controlAccentColor(),
            SystemText::Danger => NSColor::systemRedColor(),
            SystemText::Success => NSColor::systemGreenColor(),
            SystemText::Warning => NSColor::systemOrangeColor(),
        }
    }
}

impl Color {
    pub fn ns_color(self) -> Retained<NSColor> {
        match self {
            Color::Rgba(r, g, b, a) => {
                NSColor::colorWithSRGBRed_green_blue_alpha(r.into(), g.into(), b.into(), a.into())
            }
            Color::WindowBackground => NSColor::windowBackgroundColor(),
            Color::Label => NSColor::labelColor(),
            Color::Legible(text) => legible(text),
        }
    }
}

/// `text`'s system colour, darkened or lightened until it reads at 4.5:1 on the window and
/// control backgrounds: `secondaryLabelColor` and `systemRed` alone fall short on white (3.95:1,
/// 3.57:1), and the theme holds its text tokens to that ratio. A dynamic colour, resolved under
/// whatever appearance draws it, so a label made before its window keeps the window's scheme.
/// It resolves under that appearance's plain Aqua or Dark Aqua, as the theme's tokens do: under
/// a sidebar's vibrant appearance the system label colours turn opaque greys.
fn legible(text: SystemText) -> Retained<NSColor> {
    let provider = block2::RcBlock::new(move |appearance: std::ptr::NonNull<objc2_app_kit::NSAppearance>| {
        // SAFETY: AppKit hands the provider a live appearance for the duration of the call.
        let drawing = unsafe { appearance.as_ref() };
        let base = crate::system_colors::appearance_for(crate::system_colors::scheme_of(drawing));
        let resolved = std::cell::Cell::new(None);
        let block = block2::RcBlock::new(|| {
            let under = [NSColor::windowBackgroundColor(), NSColor::controlBackgroundColor()];
            let under: Vec<_> = under.into_iter().filter_map(crate::system_colors::hsla).collect();
            let color = crate::system_colors::hsla(text.ns_color());
            resolved.set(color.map(|c| nd_theme::color::legible(c, &under, 4.5)));
        });
        base.as_deref().unwrap_or(drawing).performAsCurrentDrawingAppearance(&block);
        let color = resolved.take().map_or_else(NSColor::labelColor, |c| Color::from(c).ns_color());
        std::ptr::NonNull::new(Retained::autorelease_ptr(color)).expect("a colour")
    });
    unsafe { NSColor::colorWithName_dynamicProvider(None, &provider) }
}

impl From<Hsla> for Color {
    fn from(color: Hsla) -> Self {
        let c = color.to_rgb();
        Color::Rgba(c.r, c.g, c.b, c.a)
    }
}

/// gpui's default rem, which `rems(..)` lengths resolve against until the theme sets one.
const REM: f32 = 16.;

pub(crate) fn length(l: AbsoluteLength) -> f32 {
    l.to_pixels(px(REM)).as_f32()
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ViewLook {
    pub background: Option<Color>,
    pub corner_radius: f32,
    pub border: Option<(f32, Color)>,
    pub opacity: f32,
    /// `visibility: hidden`: the view keeps its frame but shows nothing and takes no clicks or
    /// focus, its subviews included.
    pub hidden: bool,
    /// A move and scale about the centre that layout never sees.
    pub transform: nd_ui::Transform,
    /// `overflow_hidden()`: subviews are clipped to the view, rounded corners included.
    pub clips: bool,
    /// What it paints while the pointer is over it.
    pub hover: Option<Box<ViewLook>>,
    /// A shared element (`hero_from`): while its transform moves it, it is drawn above the
    /// window's content.
    pub hero: bool,
}

impl Default for ViewLook {
    fn default() -> Self {
        Self {
            background: None,
            corner_radius: 0.,
            border: None,
            opacity: 1.,
            hidden: false,
            transform: nd_ui::Transform::default(),
            clips: false,
            hover: None,
            hero: false,
        }
    }
}

impl ViewLook {
    /// `expose` paints a div without a background in the window background colour, so with
    /// `ND_DEBUG_EXPOSE` only area no node covers shows the window's own (magenta) background.
    pub fn from_style(style: &StyleRefinement, expose: bool) -> Self {
        let background = style.background.as_ref().and_then(|f| f.color()).and_then(|b| b.as_solid()).map(Color::from);
        let radius = style.corner_radii.top_left.map(length).unwrap_or(0.);
        let border_width = style.border_widths.top.map(length).unwrap_or(0.);
        let border = (border_width > 0.).then(|| (border_width, style.border_color.map_or(Color::Label, Color::from)));
        Self {
            background: background.or(expose.then_some(Color::WindowBackground)),
            corner_radius: radius,
            border,
            opacity: style.opacity.unwrap_or(1.),
            hidden: style.visibility == Some(Visibility::Hidden),
            transform: nd_ui::Transform::default(),
            clips: style.overflow.x == Some(Overflow::Hidden) && style.overflow.y == Some(Overflow::Hidden),
            hover: None,
            hero: false,
        }
    }

    /// A `div`'s look: its style, transform and hover style.
    pub fn of_div(div: &nd_ui::DivProps, expose: bool) -> Self {
        let mut look = Self::from_style(&div.style, expose);
        look.transform = div.transform;
        look.hero = div.hero.is_some();
        look.hover = div.hover.as_ref().map(|hover| {
            let mut style = div.style.clone();
            nd_gpui::gpui::Refineable::refine(&mut style, hover);
            Box::new(Self::from_style(&style, expose))
        });
        look
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TextLook {
    pub text: String,
    /// Points; `None` is the system body size.
    pub size: Option<f32>,
    pub weight: Option<FontWeight>,
    pub color: Color,
    pub tabular_nums: bool,
    pub opacity: f32,
    /// The runs of rich text, resolved under the node's theme; empty for plain text.
    pub runs: Vec<crate::rich::RunLook>,
    pub opens_links: bool,
}

impl TextLook {
    /// `theme` is the node's, which rich text resolves its colour tokens under; plain text
    /// needs none.
    pub fn from_props(props: &nd_ui::TextProps, theme: Option<&nd_theme::Theme>) -> Self {
        let style = &props.style;
        let mut look = Self {
            text: props.text.to_string(),
            size: style.text.font_size.map(length),
            weight: style.text.font_weight,
            color: style.text.color.map_or(Color::Label, Color::from),
            tabular_nums: props.tabular_nums,
            opacity: style.opacity.unwrap_or(1.),
            runs: Vec::new(),
            opens_links: props.opens_links,
        };
        if let (Some(rich), Some(theme)) = (&props.rich, theme) {
            let size = look.size.unwrap_or(objc2_app_kit::NSFont::systemFontSize() as f32);
            let weight = look.weight.map_or(400., |w| w.0);
            look.runs = crate::rich::resolve(rich, theme, size, weight, None);
        }
        look
    }

    pub fn has_links(&self) -> bool {
        self.runs.iter().any(|r| r.link.is_some())
    }

    pub fn ns_weight(&self) -> NSFontWeight {
        crate::rich::ns_weight(self.weight.map_or(400., |w| w.0))
    }
}

/// The `NSFont` for an element's text style: the system's families become the system font with
/// that design, any other family is looked up by name. `None` when the
/// style names no font, size or weight, so the control keeps its own.
pub(crate) fn ns_font(style: &StyleRefinement, default_size: f64) -> Option<Retained<objc2_app_kit::NSFont>> {
    use objc2_app_kit::{NSFont, NSFontDescriptorSystemDesignMonospaced, NSFontDescriptorSystemDesignRounded};
    use objc2_foundation::NSString;
    let text = &style.text;
    if text.font_family.is_none() && text.font_size.is_none() && text.font_weight.is_none() {
        return None;
    }
    let size = text.font_size.map_or(default_size, |s| f64::from(length(s)));
    let weight = crate::rich::ns_weight(text.font_weight.map_or(400., |w| w.0));
    let system = NSFont::systemFontOfSize_weight(size, weight);
    // The system's families are private (`.AppleSystemUIFont`, `..Rounded`, `..Monospaced`, as
    // the theme's font tokens name them): NSFont builds them from a design, never by name.
    let design = match text.font_family.as_deref() {
        None => return Some(system),
        Some(family) => match family.strip_prefix(".AppleSystemUIFont") {
            Some("") => return Some(system),
            Some("Rounded") => unsafe { NSFontDescriptorSystemDesignRounded },
            Some("Monospaced") => unsafe { NSFontDescriptorSystemDesignMonospaced },
            _ => return NSFont::fontWithName_size(&NSString::from_str(family), size).or(Some(system)),
        },
    };
    let descriptor = system.fontDescriptor().fontDescriptorWithDesign(design)?;
    NSFont::fontWithDescriptor_size(&descriptor, size).or(Some(system))
}
