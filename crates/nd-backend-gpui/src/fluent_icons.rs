//! Segoe Fluent Icons, the icon font Windows 11's own apps draw with, for the Fluent preset.
//! Each icon's codepoint is in `nd-icons` ([`nd_components::Icon::segoe_fluent`]).

use nd_gpui::gpui::{Div, Hsla, ParentElement, Pixels, Styled, div};

pub(crate) const FONT: &str = "Segoe Fluent Icons";
/// GlobalNavigationButton: NavigationView's pane toggle.
pub(crate) const HAMBURGER: char = '\u{E700}';

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
