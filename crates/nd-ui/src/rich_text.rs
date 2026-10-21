//! Attributed text: runs of one string, each with its own weight, slant, family, decoration,
//! link, colour token and size. `text`, `label` and `chat_bubble` take it; AppKit draws it as
//! an `NSAttributedString`, GPUI as `StyledText` runs.

use std::ops::Range;
use std::rc::Rc;

use nd_gpui::gpui::{Hsla, SharedString};

/// A colour named by its theme token, resolved by the backend under the theme in effect where
/// the text sits. On a filled surface (a sent chat bubble) `Accent` resolves to the fill's ink,
/// since the accent on the accent would not read.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TextColor {
    Primary,
    Secondary,
    Tertiary,
    /// `text.accent`: the accent, made legible as text.
    Accent,
    Danger,
    Success,
    Warning,
    /// A colour the app resolved itself, from `cx.theme()`.
    Custom(Hsla),
}

/// How one run differs from the text around it. The default is the text's own style.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SpanStyle {
    pub bold: bool,
    /// Between the text's weight and bold: a now-playing title, a list row's name.
    pub semibold: bool,
    pub italic: bool,
    /// The theme's monospaced font.
    pub mono: bool,
    pub underline: bool,
    pub strikethrough: bool,
    /// Clicking the run reports [`LinkClicked`] with this target. Drawn in the accent and
    /// underlined unless the run sets a colour.
    pub link: Option<SharedString>,
    /// Someone named in the text: semibold in the accent.
    pub mention: bool,
    pub color: Option<TextColor>,
    /// The run's font size as a multiple of the text's (iMessage's Big and Small effects).
    pub scale: Option<f32>,
}

impl SpanStyle {
    /// The weight the run draws at, given the text's own.
    pub fn weight(&self, base: f32) -> f32 {
        if self.bold {
            base.max(700.)
        } else if self.mention || self.semibold {
            base.max(600.)
        } else {
            base
        }
    }

    /// The colour the run draws in: its own, the accent for a link or mention, else `None`
    /// for the text's.
    pub fn color(&self) -> Option<TextColor> {
        self.color.or((self.link.is_some() || self.mention).then_some(TextColor::Accent))
    }

    pub fn underlined(&self) -> bool {
        self.underline || (self.link.is_some() && self.color.is_none())
    }

    pub fn scale(&self) -> f32 {
        self.scale.unwrap_or(1.)
    }
}

/// One run of rich text.
#[derive(Clone, Debug, PartialEq)]
pub struct Span {
    pub text: SharedString,
    pub style: SpanStyle,
}

/// A run of `text` in the text's own style; chain modifiers to change it.
pub fn span(text: impl Into<SharedString>) -> Span {
    Span { text: text.into(), style: SpanStyle::default() }
}

impl Span {
    pub fn bold(mut self) -> Self {
        self.style.bold = true;
        self
    }

    pub fn semibold(mut self) -> Self {
        self.style.semibold = true;
        self
    }

    pub fn italic(mut self) -> Self {
        self.style.italic = true;
        self
    }

    pub fn mono(mut self) -> Self {
        self.style.mono = true;
        self
    }

    pub fn underline(mut self) -> Self {
        self.style.underline = true;
        self
    }

    pub fn strikethrough(mut self) -> Self {
        self.style.strikethrough = true;
        self
    }

    pub fn link(mut self, target: impl Into<SharedString>) -> Self {
        self.style.link = Some(target.into());
        self
    }

    pub fn mention(mut self) -> Self {
        self.style.mention = true;
        self
    }

    pub fn color(mut self, color: TextColor) -> Self {
        self.style.color = Some(color);
        self
    }

    pub fn scale(mut self, scale: f32) -> Self {
        self.style.scale = Some(scale);
        self
    }

    pub fn style(mut self, style: SpanStyle) -> Self {
        self.style = style;
        self
    }
}

impl From<&'static str> for Span {
    fn from(text: &'static str) -> Self {
        span(text)
    }
}

impl From<String> for Span {
    fn from(text: String) -> Self {
        span(text)
    }
}

impl From<SharedString> for Span {
    fn from(text: SharedString) -> Self {
        span(text)
    }
}

/// Rich text: its runs in order, and their concatenation, which is what semantics, automation
/// and copy see.
#[derive(Clone, Debug, PartialEq)]
pub struct RichText {
    text: SharedString,
    spans: Rc<[Span]>,
}

impl Default for RichText {
    fn default() -> Self {
        Self { text: SharedString::default(), spans: Rc::from([]) }
    }
}

impl RichText {
    pub fn new(spans: impl IntoIterator<Item = impl Into<Span>>) -> Self {
        let spans: Rc<[Span]> = spans.into_iter().map(Into::into).filter(|s: &Span| !s.text.is_empty()).collect();
        let text: String = spans.iter().map(|s| s.text.as_ref()).collect();
        Self { text: text.into(), spans }
    }

    /// The whole text, every run concatenated.
    pub fn text(&self) -> &SharedString {
        &self.text
    }

    pub fn spans(&self) -> &[Span] {
        &self.spans
    }

    /// Each run with its byte range in [`RichText::text`].
    pub fn ranges(&self) -> impl Iterator<Item = (Range<usize>, &Span)> + '_ {
        let mut at = 0;
        self.spans.iter().map(move |s| {
            let range = at..at + s.text.len();
            at = range.end;
            (range, s)
        })
    }

    /// The link of the run covering byte `index`.
    pub fn link_at(&self, index: usize) -> Option<&SharedString> {
        self.ranges().find(|(r, _)| r.contains(&index)).and_then(|(_, s)| s.style.link.as_ref())
    }

    pub fn has_links(&self) -> bool {
        self.spans.iter().any(|s| s.style.link.is_some())
    }

    /// The runs with text in them all share one scale, so one text layout holds them.
    pub fn uniform_scale(&self) -> Option<f32> {
        let mut scales = self.spans.iter().filter(|s| !s.text.trim().is_empty()).map(|s| s.style.scale());
        let first = scales.next().unwrap_or(1.);
        scales.all(|s| s == first).then_some(first)
    }

    /// True when no run changes anything: plain text.
    pub fn is_plain(&self) -> bool {
        self.spans.iter().all(|s| s.style == SpanStyle::default())
    }
}

impl<S: Into<Span>> FromIterator<S> for RichText {
    fn from_iter<I: IntoIterator<Item = S>>(iter: I) -> Self {
        RichText::new(iter)
    }
}

/// The user clicked a run with a link. Without a handler for it the backend opens the link
/// with the system.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkClicked {
    pub link: SharedString,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_concatenate_and_map_back_to_their_links() {
        let rich = RichText::new([span("see "), span("docs").link("https://nd.dev"), span(""), span(" now").bold()]);
        assert_eq!(rich.text().as_ref(), "see docs now");
        assert_eq!(rich.spans().len(), 3, "empty runs are dropped");
        assert_eq!(rich.link_at(5).map(|l| l.as_ref()), Some("https://nd.dev"));
        assert_eq!(rich.link_at(1), None);
        assert!(rich.has_links());
        assert!(!rich.is_plain());
    }

    #[test]
    fn links_and_mentions_take_the_accent_unless_coloured() {
        let link = span("x").link("y").style;
        assert_eq!(link.color(), Some(TextColor::Accent));
        assert!(link.underlined());
        let mention = span("@a").mention().style;
        assert_eq!(mention.weight(400.), 600.);
        let red = span("x").link("y").color(TextColor::Danger).style;
        assert_eq!(red.color(), Some(TextColor::Danger));
        assert!(!red.underlined());
    }

    #[test]
    fn mixed_sizes_need_more_than_one_layout() {
        assert_eq!(RichText::new([span("a").scale(2.), span(" ")]).uniform_scale(), Some(2.));
        assert_eq!(RichText::new([span("a").scale(2.), span("b")]).uniform_scale(), None);
    }
}
