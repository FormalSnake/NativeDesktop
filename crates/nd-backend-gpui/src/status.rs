//! The drawn status page header on GPUI, in libadwaita's `AdwStatusPage` look: a large dimmed
//! icon, the title in the title-1 style, the description in body text, centred and wrapped to the
//! readable width. `compact` is AdwStatusPage's `.compact`: smaller icon, title-2, tighter margins.
//! The actions under it are ordinary buttons the component lays out.

use nd_components::StatusPageProps;
use nd_components::forms::TextStyle;
use nd_core::NodeId;
use nd_gpui::gpui::{AnyElement, FontWeight, IntoElement, ParentElement, Refineable, Styled, div, px};
use nd_theme::Theme;

use crate::backend::Node;
use crate::forms::FormLook;
use crate::forms::glyph::{Glyph, glyph};
use crate::nav::Marks;
use crate::render::Build;

/// AdwStatusPage's icon sizes, regular and compact.
const ICON: f32 = 128.;
const ICON_COMPACT: f32 = 64.;

/// The header: icon, title and description, centred in a column no wider than the theme's
/// readable width.
pub fn status_page_view(props: &StatusPageProps, theme: &Theme, marks: &Marks) -> AnyElement {
    let look = FormLook::adwaita(theme);
    let compact = props.compact;
    let (title_size, title_weight, title_line) =
        look.type_of(if compact { TextStyle::Title2 } else { TextStyle::Title1 });
    let (body_size, body_weight, body_line) = look.type_of(TextStyle::Body);
    let s = &theme.spacing;
    let mut column = div()
        .flex()
        .flex_col()
        .items_center()
        .w_full()
        .max_w(s.content_width)
        .px(s.md)
        .py(if compact { s.xl } else { s.xl + s.md })
        .gap(s.md)
        .text_center();
    if let Some(icon) = props.icon {
        let side = px(if compact { ICON_COMPACT } else { ICON });
        // Decorative: the title names the state, so the icon only needs to be seen, not read.
        let color = theme.text.tertiary;
        marks.fill("status page icon", color, &[]);
        let stroke = look.glyph_stroke * (side / look.icon_size);
        column = column.child(div().flex_none().mb(if compact { s.md } else { s.xl }).child(glyph(
            Glyph::of(icon),
            side,
            color,
            stroke,
        )));
    }
    marks.text(&props.title, theme.text.primary, &[], title_size, title_weight);
    column = column.child(
        div()
            .w_full()
            .text_size(title_size)
            .line_height(title_line)
            .font_weight(title_weight)
            .text_color(theme.text.primary)
            .child(props.title.clone()),
    );
    if let Some(description) = &props.description {
        marks.text(description, theme.text.primary, &[], body_size, body_weight);
        column = column.child(
            div()
                .w_full()
                .text_size(body_size)
                .line_height(body_line)
                .font_weight(FontWeight::NORMAL)
                .text_color(theme.text.primary)
                .child(description.clone()),
        );
    }
    column.style().refine(&props.style);
    column.into_any_element()
}

impl Build<'_> {
    pub(crate) fn status_page(&self, id: NodeId, node: &Node) -> Option<AnyElement> {
        let props = node.props.downcast_ref::<StatusPageProps>()?;
        let marks = Marks::default();
        let element = status_page_view(props, &self.theme(), &marks);
        self.record_marks(id, marks.take());
        Some(element)
    }
}
