//! What a presenter draws: the viewport as rows of cells with their text and resolved colours,
//! copied out of libghostty's render state. Rows are cached by libghostty's row id and rebuilt
//! only when it marks them dirty.

use crate::theme::Rgb;

/// Cell attributes the presenters draw.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Attrs(u16);

impl Attrs {
    pub const BOLD: Attrs = Attrs(1);
    pub const ITALIC: Attrs = Attrs(1 << 1);
    pub const FAINT: Attrs = Attrs(1 << 2);
    pub const STRIKETHROUGH: Attrs = Attrs(1 << 3);
    pub const OVERLINE: Attrs = Attrs(1 << 4);
    pub const INVISIBLE: Attrs = Attrs(1 << 5);
    pub const SELECTED: Attrs = Attrs(1 << 6);
    /// An OSC 8 hyperlink.
    pub const LINK: Attrs = Attrs(1 << 7);
    /// The first cell of a two-cell character.
    pub const WIDE: Attrs = Attrs(1 << 8);
    /// The second cell of a two-cell character, drawn by the first.
    pub const SPACER: Attrs = Attrs(1 << 9);
    pub const BLINK: Attrs = Attrs(1 << 10);

    pub fn contains(self, other: Attrs) -> bool {
        self.0 & other.0 == other.0
    }

    pub fn insert(&mut self, other: Attrs) {
        self.0 |= other.0;
    }

    pub fn set(&mut self, other: Attrs, on: bool) {
        if on {
            self.0 |= other.0;
        } else {
            self.0 &= !other.0;
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Underline {
    #[default]
    None,
    Single,
    Double,
    Curly,
    Dotted,
    Dashed,
}

/// One grid cell. Colours are `None` where the cell uses the terminal's default, which the
/// presenter draws with the theme token itself.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Cell {
    /// Byte range of the cell's grapheme in its row's `text`; empty for a blank cell.
    pub start: u32,
    pub len: u16,
    pub fg: Option<Rgb>,
    pub bg: Option<Rgb>,
    pub underline: Underline,
    pub underline_color: Option<Rgb>,
    pub attrs: Attrs,
}

impl Cell {
    pub fn is_blank(&self) -> bool {
        self.len == 0
    }

    /// Cells the glyph spans.
    pub fn width(&self) -> u16 {
        if self.attrs.contains(Attrs::WIDE) { 2 } else { 1 }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Row {
    pub text: String,
    pub cells: Vec<Cell>,
}

impl Row {
    pub fn grapheme(&self, cell: &Cell) -> &str {
        let start = cell.start as usize;
        &self.text[start..start + usize::from(cell.len)]
    }

    /// The row's text with blank cells as spaces and wide spacers skipped, trailing blanks
    /// trimmed.
    pub fn plain_text(&self) -> String {
        let mut out = String::with_capacity(self.cells.len());
        for cell in &self.cells {
            if cell.attrs.contains(Attrs::SPACER) {
                continue;
            }
            if cell.is_blank() {
                out.push(' ');
            } else {
                out.push_str(self.grapheme(cell));
            }
        }
        out.truncate(out.trim_end().len());
        out
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CursorShape {
    #[default]
    Block,
    BlockHollow,
    Bar,
    Underline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cursor {
    pub col: u16,
    pub row: u16,
    pub shape: CursorShape,
    /// The colour a program set (OSC 12); `None` draws the theme's cursor colour.
    pub color: Option<Rgb>,
    /// On the right half of a wide character: the block spans both cells from `col - 1`.
    pub wide_tail: bool,
    pub blinking: bool,
}

/// The viewport as of the last update.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Frame {
    pub cols: u16,
    pub rows: Vec<Row>,
    pub cursor: Option<Cursor>,
    /// The colours in effect, which programs can change (OSC 10/11).
    pub background: Rgb,
    pub foreground: Rgb,
    /// Bumped by every update that changed anything; presenters compare it to skip work.
    pub generation: u64,
    /// Rows above the viewport in the scrollback, and the scrollback plus viewport length.
    pub scroll_offset: u64,
    pub scroll_total: u64,
}

impl Frame {
    /// The viewport's text, one line per row, trailing blanks trimmed.
    pub fn text(&self) -> String {
        let mut lines: Vec<String> = self.rows.iter().map(Row::plain_text).collect();
        while lines.last().is_some_and(String::is_empty) {
            lines.pop();
        }
        lines.join("\n")
    }

    pub fn row_text(&self, row: usize) -> String {
        self.rows.get(row).map(Row::plain_text).unwrap_or_default()
    }

    pub fn cell(&self, col: u16, row: u16) -> Option<(&Row, &Cell)> {
        let r = self.rows.get(usize::from(row))?;
        Some((r, r.cells.get(usize::from(col))?))
    }
}
