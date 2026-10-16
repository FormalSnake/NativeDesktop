//! Rich text on GPUI: a `RichText`'s runs as `StyledText` highlights, or, when runs differ in
//! size, as a wrapping row of words per line, since one text layout holds one size. Text
//! nodes, labels and chat bubbles all draw through here.

use std::ops::Range;
use std::rc::Rc;

use nd_components::rich::text_color;
use nd_gpui::gpui::{
    self, AnyElement, App, ElementId, FontStyle, FontWeight, HighlightStyle, Hsla, InteractiveElement, InteractiveText,
    IntoElement, ParentElement, Pixels, SharedString, StrikethroughStyle, Styled, StyledText, UnderlineStyle, div, px,
};
use nd_theme::Theme;
use nd_ui::{RichText, SpanStyle};

/// Called with the link of a clicked run.
pub(crate) type OnLink = Rc<dyn Fn(SharedString, &mut gpui::Window, &mut App)>;

/// What the text around the runs is drawn in.
#[derive(Clone, Copy)]
pub(crate) struct Base<'a> {
    pub theme: &'a Theme,
    pub size: Pixels,
    /// Line height as a multiple of the size.
    pub leading: f32,
    pub weight: f32,
    pub color: Hsla,
    /// The ink of the fill the text sits on (a sent bubble), which colour tokens resolve to.
    pub ink: Option<Hsla>,
}

impl Base<'_> {
    /// The colour a run draws in.
    pub fn color(&self, style: &SpanStyle) -> Hsla {
        style.color().map_or(self.color, |c| text_color(self.theme, c, self.ink))
    }

    fn highlight(&self, style: &SpanStyle) -> HighlightStyle {
        let color = self.color(style);
        let weight = style.weight(self.weight);
        HighlightStyle {
            color: Some(color),
            font_weight: (weight != self.weight).then_some(FontWeight(weight)),
            font_style: style.italic.then_some(FontStyle::Italic),
            underline: style.underlined().then_some(UnderlineStyle { thickness: px(1.), color: Some(color), wavy: false }),
            strikethrough: style.strikethrough.then_some(StrikethroughStyle { thickness: px(1.), color: Some(color) }),
            ..HighlightStyle::default()
        }
    }
}

/// The runs as one `StyledText`, with links clickable when `on_link` is set.
fn styled(
    id: ElementId,
    text: SharedString,
    runs: &[(Range<usize>, &SpanStyle)],
    base: &Base<'_>,
    on_link: Option<&OnLink>,
) -> AnyElement {
    let highlights: Vec<(Range<usize>, HighlightStyle)> =
        runs.iter().map(|(r, s)| (r.clone(), base.highlight(s))).collect();
    let mono = base.theme.fonts.mono.family.clone();
    let families: Vec<(Range<usize>, SharedString)> =
        runs.iter().filter(|(_, s)| s.mono).map(|(r, _)| (r.clone(), mono.clone())).collect();
    let styled = StyledText::new(text).with_highlights(highlights).with_font_family_overrides(families);
    let links: Vec<(Range<usize>, SharedString)> =
        runs.iter().filter_map(|(r, s)| Some((r.clone(), s.link.clone()?))).collect();
    match on_link {
        Some(on_link) if !links.is_empty() => {
            let on_link = on_link.clone();
            let ranges = links.iter().map(|(r, _)| r.clone()).collect();
            InteractiveText::new(id, styled)
                .on_click(ranges, move |ix, window, cx| on_link(links[ix].1.clone(), window, cx))
                .into_any_element()
        }
        _ => styled.into_any_element(),
    }
}

/// `segment` split into words, each keeping the space after it.
fn words(segment: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut after_space = true;
    for (index, ch) in segment.char_indices() {
        let space = ch.is_whitespace();
        if !space && after_space && index > start {
            out.push(start..index);
            start = index;
        }
        after_space = space;
    }
    if start < segment.len() {
        out.push(start..segment.len());
    }
    out
}

/// The element drawing `rich` in `base`. `id` keys the element GPUI keeps click state under.
pub(crate) fn element(id: ElementId, rich: &RichText, base: &Base<'_>, on_link: Option<OnLink>) -> AnyElement {
    if let Some(scale) = rich.uniform_scale() {
        let size = base.size * scale;
        let runs: Vec<_> = rich.ranges().map(|(r, s)| (r, &s.style)).collect();
        return div()
            .text_size(size)
            .line_height(size * base.leading)
            .text_color(base.color)
            .child(styled(id, rich.text().clone(), &runs, base, on_link.as_ref()))
            .into_any_element();
    }
    // Runs of different sizes: one row of words per line, each word laid out alone.
    let mut lines: Vec<Vec<AnyElement>> = vec![Vec::new()];
    let mut word = 0usize;
    for (_, span) in rich.ranges() {
        let size = base.size * span.style.scale();
        for (index, segment) in span.text.split('\n').enumerate() {
            if index > 0 {
                lines.push(Vec::new());
            }
            for range in words(segment) {
                let text: SharedString = segment[range.clone()].to_owned().into();
                let runs = [(0..text.len(), &span.style)];
                let piece = styled(
                    ElementId::NamedInteger("word".into(), word as u64),
                    text,
                    &runs,
                    base,
                    on_link.as_ref(),
                );
                word += 1;
                lines.last_mut().expect("a line").push(
                    div().text_size(size).line_height(size * base.leading).child(piece).into_any_element(),
                );
            }
        }
    }
    div()
        .id(id)
        .flex()
        .flex_col()
        .text_color(base.color)
        .children(lines.into_iter().map(|line| div().flex().flex_row().flex_wrap().items_baseline().children(line)))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::words;

    #[test]
    fn words_keep_the_space_after_them() {
        let s = "big  news now";
        let got: Vec<&str> = words(s).into_iter().map(|r| &s[r]).collect();
        assert_eq!(got, ["big  ", "news ", "now"]);
    }
}
