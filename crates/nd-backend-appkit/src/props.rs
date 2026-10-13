//! Props the AppKit backend understands for each [`nd_ui::backend::NodeKind`]. They move into
//! `nd-ui` once its element vocabulary lands; until then the reconciler (or a caller driving the
//! seam directly) builds these.

use objc2::rc::Retained;
use objc2_app_kit::NSColor;

/// A colour as the backend resolves it: system colours are looked up at draw time, under the
/// view's effective appearance, so light/dark and accent changes need no update.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Color {
    Rgba(f32, f32, f32, f32),
    WindowBackground,
    ControlBackground,
    Label,
    SecondaryLabel,
    Separator,
    Accent,
}

impl Color {
    pub(crate) fn ns_color(self) -> Retained<NSColor> {
        match self {
            Color::Rgba(r, g, b, a) => {
                NSColor::colorWithSRGBRed_green_blue_alpha(r.into(), g.into(), b.into(), a.into())
            }
            Color::WindowBackground => NSColor::windowBackgroundColor(),
            Color::ControlBackground => NSColor::controlBackgroundColor(),
            Color::Label => NSColor::labelColor(),
            Color::SecondaryLabel => NSColor::secondaryLabelColor(),
            Color::Separator => NSColor::separatorColor(),
            Color::Accent => NSColor::controlAccentColor(),
        }
    }
}

/// A styled container (`NodeKind::View`), drawn by its CALayer.
#[derive(Clone, Debug, PartialEq)]
pub struct ViewProps {
    pub background: Option<Color>,
    pub corner_radius: f32,
    pub border: Option<(f32, Color)>,
    pub opacity: f32,
}

impl Default for ViewProps {
    fn default() -> Self {
        Self { background: None, corner_radius: 0., border: None, opacity: 1. }
    }
}

/// A text run (`NodeKind::Text`).
#[derive(Clone, Debug, PartialEq)]
pub struct TextProps {
    pub text: String,
    /// Points; `None` is the system body size.
    pub size: Option<f32>,
    pub color: Color,
    pub tabular_nums: bool,
    pub opacity: f32,
}

impl TextProps {
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into(), size: None, color: Color::Label, tabular_nums: false, opacity: 1. }
    }
}

/// Names the native push button for `NodeKind::Widget(WidgetKind::of::<NativeButton>())`.
pub struct NativeButton;

#[derive(Clone, Debug, PartialEq)]
pub struct ButtonProps {
    pub title: String,
    pub enabled: bool,
}

impl ButtonProps {
    pub fn new(title: impl Into<String>) -> Self {
        Self { title: title.into(), enabled: true }
    }
}
