//! What both presenters agree on about colour: the colours a cell draws with, and the paint list
//! the appearance audit reads (the background, each distinct fill, each distinct text colour on
//! what it sits on, and the cursor).

use nd_theme::color::with_alpha;
use nd_ui::prelude::Hsla;

use crate::frame::{Attrs, Cell, Frame};
use crate::theme::{Rgb, TerminalTheme};

/// A fill or text run, with the fills under it inside the terminal (innermost last).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Mark {
    pub label: String,
    pub color: Hsla,
    pub under: Vec<Hsla>,
    /// Size in points and CSS weight for text, `None` for a fill.
    pub text: Option<(f32, f32)>,
}

/// The colours a cell draws with: its text, and its fill when it has one of its own. Colours
/// that resolve to a theme token draw with the token, alpha included.
pub(crate) fn cell_colors(cell: &Cell, theme: &TerminalTheme, frame: &Frame) -> (Hsla, Option<Hsla>) {
    if cell.attrs.contains(Attrs::SELECTED) {
        return (theme.foreground, Some(theme.selection));
    }
    let mut fg = match cell.fg {
        Some(rgb) if rgb == frame.foreground => theme.foreground,
        Some(rgb) => theme.ink(rgb),
        None if frame.foreground == theme.foreground_rgb => theme.foreground,
        None => frame.foreground.to_hsla(),
    };
    if cell.attrs.contains(Attrs::FAINT) {
        fg = with_alpha(fg, fg.a * 0.5);
    }
    let bg = cell.bg.map(|rgb| if rgb == theme.background_rgb { theme.background } else { theme.fill(rgb) });
    (fg, bg)
}

pub(crate) fn marks(frame: &Frame, theme: &TerminalTheme, font_size: f32) -> Vec<Mark> {
    let mut out = vec![Mark { label: "terminal".into(), color: theme.background, under: Vec::new(), text: None }];
    let mut fills: Vec<Hsla> = Vec::new();
    let mut texts: Vec<(Hsla, Option<Hsla>, bool)> = Vec::new();
    for row in &frame.rows {
        for cell in &row.cells {
            let (fg, bg) = cell_colors(cell, theme, frame);
            if let Some(bg) = bg
                && !fills.contains(&bg)
            {
                fills.push(bg);
                out.push(Mark {
                    label: "cell background".into(),
                    color: bg,
                    under: vec![theme.background],
                    text: None,
                });
            }
            if cell.is_blank() || cell.attrs.contains(Attrs::INVISIBLE) {
                continue;
            }
            let bold = cell.attrs.contains(Attrs::BOLD);
            if !texts.contains(&(fg, bg, bold)) {
                texts.push((fg, bg, bold));
                out.push(Mark {
                    label: row.grapheme(cell).to_owned(),
                    color: fg,
                    under: std::iter::once(theme.background).chain(bg).collect(),
                    text: Some((font_size, if bold { 700. } else { 400. })),
                });
            }
        }
    }
    if let Some(cursor) = frame.cursor {
        let color = cursor.color.map_or(theme.cursor, Rgb::to_hsla);
        out.push(Mark { label: "cursor".into(), color, under: vec![theme.background], text: None });
    }
    out
}
