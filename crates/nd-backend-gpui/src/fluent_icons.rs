//! nd's icons as Segoe Fluent Icons glyphs, the icon font Windows 11's own apps draw with, for
//! the Fluent preset. Codepoints from Microsoft's Segoe Fluent Icons reference.

use nd_components::icon::Icon;
use nd_gpui::gpui::{Div, Hsla, ParentElement, Pixels, Styled, div};

pub(crate) const FONT: &str = "Segoe Fluent Icons";
/// GlobalNavigationButton: NavigationView's pane toggle.
pub(crate) const HAMBURGER: char = '\u{E700}';

pub(crate) fn codepoint(icon: Icon) -> char {
    match icon {
        Icon::Archive => '\u{E7B8}',
        Icon::Check => '\u{E73E}',
        Icon::ChevronLeft => '\u{E76B}',
        Icon::ChevronRight => '\u{E76C}',
        Icon::CircleCheck => '\u{E930}',
        Icon::FileText => '\u{E8A5}',
        Icon::Folder => '\u{E8B7}',
        Icon::Inbox => '\u{E8F1}',
        Icon::Info => '\u{E946}',
        Icon::MapPin => '\u{E81D}',
        Icon::Minus => '\u{E738}',
        Icon::Notebook => '\u{E70B}',
        Icon::PanelLeft => '\u{E90C}',
        Icon::PanelRight => '\u{E90D}',
        Icon::Pin => '\u{E718}',
        Icon::Plus => '\u{E710}',
        Icon::Search => '\u{E721}',
        Icon::Settings => '\u{E713}',
        Icon::Share => '\u{E72D}',
        Icon::SquarePen => '\u{E70F}',
        Icon::Star => '\u{E734}',
        Icon::Trash => '\u{E74D}',
        Icon::TriangleAlert => '\u{E7BA}',
        Icon::Calendar => '\u{E787}',
        Icon::Clock => '\u{E823}',
        Icon::Eye => '\u{E890}',
        Icon::EyeOff => '\u{ED1A}',
        Icon::Globe => '\u{E774}',
        Icon::Lock => '\u{E72E}',
        Icon::Mail => '\u{E715}',
        Icon::User => '\u{E77B}',
        Icon::X => '\u{E711}',
        Icon::CircleX => '\u{EA39}',
        Icon::RotateCw => '\u{E72C}',
        Icon::Code => '\u{E943}',
        Icon::Download => '\u{E896}',
        Icon::ArrowUp => '\u{E74A}',
        Icon::Image => '\u{EB9F}',
        Icon::LocateFixed => '\u{E81D}',
        Icon::Music => '\u{E8D6}',
        Icon::Paperclip => '\u{E723}',
        Icon::Pause => '\u{E769}',
        Icon::Play => '\u{E768}',
        Icon::SkipBack => '\u{E892}',
        Icon::SkipForward => '\u{E893}',
        Icon::Smile => '\u{E76E}',
        Icon::Volume => '\u{E767}',
        Icon::Volume1 => '\u{E993}',
        Icon::Volume2 => '\u{E995}',
        Icon::VolumeX => '\u{E74F}',
    }
}

/// `glyph` drawn `size` square in the icon font.
pub(crate) fn glyph(glyph: char, size: Pixels, color: Hsla) -> Div {
    div()
        .flex_none()
        .size(size)
        .flex()
        .items_center()
        .justify_center()
        .font_family(FONT)
        .text_size(size)
        .line_height(size)
        .text_color(color)
        .child(glyph.to_string())
}
