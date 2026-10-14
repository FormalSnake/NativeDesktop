//! What the AppKit views draw, resolved from `nd-ui` props. Layout fields of a style arrive as
//! frames; only the visual part is read here.

use nd_gpui::gpui::Visibility;
use nd_ui::StyleRefinement;
use nd_ui::prelude::{AbsoluteLength, FontWeight, Hsla, px};
use objc2::rc::Retained;
use objc2_app_kit::{
    NSColor, NSFontWeight, NSFontWeightBold, NSFontWeightMedium, NSFontWeightRegular, NSFontWeightSemibold,
};

/// A colour as the views resolve it: system colours are looked up at draw time, under the
/// view's effective appearance, so light/dark changes need no update.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Color {
    Rgba(f32, f32, f32, f32),
    WindowBackground,
    Label,
}

impl Color {
    pub fn ns_color(self) -> Retained<NSColor> {
        match self {
            Color::Rgba(r, g, b, a) => {
                NSColor::colorWithSRGBRed_green_blue_alpha(r.into(), g.into(), b.into(), a.into())
            }
            Color::WindowBackground => NSColor::windowBackgroundColor(),
            Color::Label => NSColor::labelColor(),
        }
    }
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
}

impl Default for ViewLook {
    fn default() -> Self {
        Self { background: None, corner_radius: 0., border: None, opacity: 1., hidden: false }
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
        }
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
}

impl TextLook {
    pub fn from_props(props: &nd_ui::TextProps) -> Self {
        let style = &props.style;
        Self {
            text: props.text.to_string(),
            size: style.text.font_size.map(length),
            weight: style.text.font_weight,
            color: style.text.color.map_or(Color::Label, Color::from),
            tabular_nums: props.tabular_nums,
            opacity: style.opacity.unwrap_or(1.),
        }
    }

    pub fn ns_weight(&self) -> NSFontWeight {
        let w = self.weight.map_or(400., |w| w.0);
        unsafe {
            if w >= 700. {
                NSFontWeightBold
            } else if w >= 600. {
                NSFontWeightSemibold
            } else if w >= 500. {
                NSFontWeightMedium
            } else {
                NSFontWeightRegular
            }
        }
    }
}

/// The `NSFont` for an element's text style: nd's system families (`nd_theme::preset`) become
/// the system font with that design, any other family is looked up by name. `None` when the
/// style names no font, size or weight, so the control keeps its own.
pub(crate) fn ns_font(style: &StyleRefinement, default_size: f64) -> Option<Retained<objc2_app_kit::NSFont>> {
    use objc2_app_kit::{NSFont, NSFontDescriptorSystemDesignMonospaced, NSFontDescriptorSystemDesignRounded};
    use objc2_foundation::NSString;
    let text = &style.text;
    if text.font_family.is_none() && text.font_size.is_none() && text.font_weight.is_none() {
        return None;
    }
    let size = text.font_size.map_or(default_size, |s| f64::from(length(s)));
    let weight = TextLook {
        text: String::new(),
        size: None,
        weight: text.font_weight,
        color: Color::Label,
        tabular_nums: false,
        opacity: 1.,
    }
    .ns_weight();
    let system = NSFont::systemFontOfSize_weight(size, weight);
    let design = match text.font_family.as_deref() {
        None | Some(nd_theme::preset::SYSTEM_FONT) => return Some(system),
        Some(nd_theme::preset::SYSTEM_FONT_ROUNDED) => unsafe { NSFontDescriptorSystemDesignRounded },
        Some(nd_theme::preset::SYSTEM_FONT_MONO) => unsafe { NSFontDescriptorSystemDesignMonospaced },
        Some(family) => return NSFont::fontWithName_size(&NSString::from_str(family), size).or(Some(system)),
    };
    let descriptor = system.fontDescriptor().fontDescriptorWithDesign(design)?;
    NSFont::fontWithDescriptor_size(&descriptor, size).or(Some(system))
}
