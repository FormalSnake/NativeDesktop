//! Drawn text entry: `text_field` (plain, secure, search), `text_area` and `combo_box`, plus the
//! single-line fields of the date picker, colour well and stepper, which commit a parsed value
//! on Return or when focus leaves instead of reporting every edit.
//!
//! The [`Editor`] holds a copy of the text while the user edits. A prop text it has not seen
//! replaces the copy (the app wrote it) and clamps the caret; the prop it already adopted leaves
//! the copy alone, so an app feeding the reported text back moves nothing. An app write during
//! IME composition waits until composition ends.

use std::any::Any;
use std::ops::Range;
use std::rc::Rc;

use nd_components::forms::*;
use nd_core::NodeId;
use nd_gpui::gpui::{
    self, App, Bounds, ClipboardItem, CursorStyle, ElementId, FocusHandle, GlobalElementId, InspectorElementId,
    InteractiveElement, IntoElement, KeyDownEvent, Keystroke, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent,
    ParentElement, Pixels, Point, ScrollWheelEvent, ShapedLine, SharedString, StatefulInteractiveElement, Style,
    Styled, TextRun, UTF16Selection, UnderlineStyle, Window, WrappedLine, anchored, deferred, div, fill, point, px,
    relative, size,
};
use unicode_segmentation::UnicodeSegmentation;

use super::glyph::{Glyph, glyph};
use super::{Ctl, FormLook, Sink, with_style};
use crate::render::Build;

/// Reads a parsed field's text as its value's event.
pub(crate) type Parser = Rc<dyn Fn(&str) -> Option<Box<dyn Any>>>;

/// What an edit produces.
pub(crate) enum Commit {
    /// `TextChanged` on every edit, `TextSubmitted` on Return.
    Text,
    /// The parsed value's event on Return or focus loss; text that does not parse reverts.
    Parse(Parser),
}

enum Layout {
    Line(Box<ShapedLine>),
    /// Each hard line with its top, relative to the text origin.
    Lines(Vec<(WrappedLine, Pixels)>),
}

pub(crate) struct Editor {
    text: String,
    /// The prop text last adopted.
    synced: Option<SharedString>,
    selected: Range<usize>,
    reversed: bool,
    marked: Option<Range<usize>>,
    scroll: Point<Pixels>,
    /// The caret moved or the text changed: scroll it into view on the next layout.
    reveal: bool,
    selecting: bool,
    layout: Option<Layout>,
    /// Where the text was last painted: the element's bounds and the text origin.
    painted: Option<(Bounds<Pixels>, Point<Pixels>)>,
    line_height: Pixels,
    multiline: bool,
    secure: bool,
    editable: bool,
    /// Combo box suggestions and whether typed text completes from them.
    options: Vec<SharedString>,
    completes: bool,
    open: bool,
    highlight: Option<usize>,
    commit: Commit,
    /// Edited since the last commit (parsed fields only).
    dirty: bool,
    sink: Option<Sink>,
    /// Return submits; Shift- or Alt-Return breaks the line.
    submit_on_return: bool,
    /// Pasted images go to the app as `ImagesPasted` instead of as text.
    accepts_media: bool,
    /// Height of the laid-out text, for an area that grows with it.
    content: Pixels,
    /// The emoji picker request last seen, and whether the picker is open.
    emoji: Option<u64>,
    emoji_open: bool,
}

impl Editor {
    pub fn new(multiline: bool, commit: Commit) -> Self {
        Self {
            text: String::new(),
            synced: None,
            selected: 0..0,
            reversed: false,
            marked: None,
            scroll: Point::default(),
            reveal: false,
            selecting: false,
            layout: None,
            painted: None,
            line_height: px(20.),
            multiline,
            secure: false,
            editable: true,
            options: Vec::new(),
            completes: false,
            open: false,
            highlight: None,
            commit,
            dirty: false,
            sink: None,
            submit_on_return: false,
            accepts_media: false,
            content: px(0.),
            emoji: None,
            emoji_open: false,
        }
    }

    /// Takes `prop` unless it is the text already adopted, or composition is under way.
    pub fn sync(&mut self, prop: &SharedString) {
        if self.synced.as_ref() == Some(prop) || self.marked.is_some() {
            return;
        }
        self.synced = Some(prop.clone());
        self.dirty = false;
        if self.text != prop.as_ref() {
            self.text = prop.to_string();
            let clamp = |o: usize| self.floor(o.min(self.text.len()));
            self.selected = clamp(self.selected.start)..clamp(self.selected.end);
        }
    }

    fn floor(&self, mut offset: usize) -> usize {
        while !self.text.is_char_boundary(offset) {
            offset -= 1;
        }
        offset
    }

    fn cursor(&self) -> usize {
        if self.reversed { self.selected.start } else { self.selected.end }
    }

    fn move_to(&mut self, offset: usize) {
        self.selected = offset..offset;
        self.reversed = false;
        self.reveal = true;
    }

    fn select_to(&mut self, offset: usize) {
        if self.reversed {
            self.selected.start = offset;
        } else {
            self.selected.end = offset;
        }
        if self.selected.end < self.selected.start {
            self.reversed = !self.reversed;
            self.selected = self.selected.end..self.selected.start;
        }
        self.reveal = true;
    }

    fn previous_boundary(&self, offset: usize) -> usize {
        self.text.grapheme_indices(true).rev().find_map(|(i, _)| (i < offset).then_some(i)).unwrap_or(0)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        self.text.grapheme_indices(true).find_map(|(i, _)| (i > offset).then_some(i)).unwrap_or(self.text.len())
    }

    fn previous_word(&self, offset: usize) -> usize {
        self.text
            .split_word_bound_indices()
            .rev()
            .find(|(i, w)| *i < offset && !w.trim().is_empty())
            .map_or(0, |(i, _)| i)
    }

    fn next_word(&self, offset: usize) -> usize {
        self.text
            .split_word_bound_indices()
            .find(|(i, w)| i + w.len() > offset && !w.trim().is_empty())
            .map_or(self.text.len(), |(i, w)| (i + w.len()).max(offset))
    }

    fn line_start(&self, offset: usize) -> usize {
        if !self.multiline {
            return 0;
        }
        self.text[..offset].rfind('\n').map_or(0, |i| i + 1)
    }

    fn line_end(&self, offset: usize) -> usize {
        if !self.multiline {
            return self.text.len();
        }
        self.text[offset..].find('\n').map_or(self.text.len(), |i| offset + i)
    }

    /// Replaces `range` with `new`, leaving the caret after it, and reports the edit.
    fn replace(&mut self, range: Range<usize>, new: &str, cx: &mut App) {
        if !self.editable {
            return;
        }
        let new = if self.multiline { new.to_owned() } else { new.replace(['\n', '\r'], " ") };
        let before = self.text.clone();
        self.text.replace_range(range.clone(), &new);
        let caret = range.start + new.len();
        self.move_to(caret);
        self.marked = None;
        if self.completes && caret == self.text.len() && !new.is_empty() {
            self.complete();
        }
        if self.text != before {
            self.edited(cx);
        }
    }

    /// Takes the first suggestion the typed text starts (ignoring case), selecting what it added.
    fn complete(&mut self) {
        let typed = self.text.to_lowercase();
        let Some(option) =
            self.options.iter().find(|o| o.to_lowercase().starts_with(&typed) && o.len() > self.text.len())
        else {
            return;
        };
        let typed_chars = self.text.chars().count();
        let start = option.char_indices().nth(typed_chars).map_or(option.len(), |(i, _)| i);
        self.text = option.to_string();
        self.selected = start..self.text.len();
        self.reversed = false;
    }

    fn edited(&mut self, cx: &mut App) {
        match self.commit {
            Commit::Text => {
                if let Some(sink) = &self.sink {
                    sink.send(TextChanged { text: self.text.clone().into() }, cx);
                }
            }
            Commit::Parse(_) => self.dirty = true,
        }
    }

    fn submit(&mut self, cx: &mut App) {
        match &self.commit {
            Commit::Text => {
                if let Some(sink) = &self.sink {
                    sink.send(TextSubmitted { text: self.text.clone().into() }, cx);
                }
            }
            Commit::Parse(_) => self.commit(cx),
        }
    }

    /// A parsed field's edit, as its value's event; text that does not parse goes back to the
    /// prop's.
    pub fn commit(&mut self, cx: &mut App) {
        let Commit::Parse(parse) = &self.commit else { return };
        if !std::mem::take(&mut self.dirty) {
            return;
        }
        match (parse(&self.text), &self.sink) {
            (Some(event), Some(sink)) => sink.send_boxed(event, cx),
            _ => {
                if let Some(prop) = self.synced.clone() {
                    self.text = prop.to_string();
                    let end = self.text.len();
                    self.move_to(end);
                }
            }
        }
    }

    fn display(&self) -> String {
        if self.secure { "\u{2022}".repeat(self.text.chars().count()) } else { self.text.clone() }
    }

    fn text_to_display(&self, offset: usize) -> usize {
        if self.secure { self.text[..offset].chars().count() * '\u{2022}'.len_utf8() } else { offset }
    }

    fn display_to_text(&self, offset: usize) -> usize {
        if !self.secure {
            return self.floor(offset.min(self.text.len()));
        }
        let n = offset / '\u{2022}'.len_utf8();
        self.text.char_indices().nth(n).map_or(self.text.len(), |(i, _)| i)
    }

    fn offset_to_utf16(&self, offset: usize) -> usize {
        self.text[..offset.min(self.text.len())].encode_utf16().count()
    }

    fn utf16_to_offset(&self, offset: usize) -> usize {
        let mut units = 0;
        for (i, ch) in self.text.char_indices() {
            if units >= offset {
                return i;
            }
            units += ch.len_utf16();
        }
        self.text.len()
    }

    fn utf16_to_range(&self, r: &Range<usize>) -> Range<usize> {
        self.utf16_to_offset(r.start)..self.utf16_to_offset(r.end)
    }

    fn range_to_utf16_units(&self, r: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(r.start)..self.offset_to_utf16(r.end)
    }

    /// Where `offset` sits relative to the text origin, top of its line.
    fn position(&self, offset: usize) -> Point<Pixels> {
        let d = self.text_to_display(offset);
        match &self.layout {
            Some(Layout::Line(line)) => point(line.x_for_index(d), px(0.)),
            Some(Layout::Lines(lines)) => {
                let mut start = 0;
                for (line, top) in lines {
                    if d <= start + line.len() {
                        let p = line.position_for_index(d - start, self.line_height).unwrap_or_default();
                        return point(p.x, *top + p.y);
                    }
                    start += line.len() + 1;
                }
                Point::default()
            }
            None => Point::default(),
        }
    }

    /// The text offset nearest `at`, in window coordinates.
    fn offset_at(&self, at: Point<Pixels>) -> usize {
        let Some((_, origin)) = self.painted else { return 0 };
        let local = at - origin;
        let d = match &self.layout {
            Some(Layout::Line(line)) => line.closest_index_for_x(local.x),
            Some(Layout::Lines(lines)) => {
                let mut start = 0;
                let mut found = self.display().len();
                for (i, (line, top)) in lines.iter().enumerate() {
                    let bottom = *top + line.size(self.line_height).height;
                    if local.y < bottom || i + 1 == lines.len() {
                        let p = point(local.x.max(px(0.)), (local.y - *top).max(px(0.)));
                        found = start + line.closest_index_for_position(p, self.line_height).unwrap_or_else(|i| i);
                        break;
                    }
                    start += line.len() + 1;
                }
                found
            }
            None => 0,
        };
        self.display_to_text(d)
    }

    /// One line up (`-1`) or down (`1`) from the caret, keeping its x.
    fn vertical(&self, direction: f32) -> usize {
        let p = self.position(self.cursor());
        let Some((_, origin)) = self.painted else { return self.cursor() };
        let target = origin + point(p.x, p.y + self.line_height * (0.5 + direction));
        if target.y < origin.y {
            return 0;
        }
        self.offset_at(target)
    }

    /// A key no binding took. Returns whether the field used it.
    fn key(&mut self, ks: &Keystroke, cx: &mut App) -> bool {
        if self.marked.is_some() {
            return false;
        }
        let m = ks.modifiers;
        let shortcut = if cfg!(target_os = "macos") { m.platform } else { m.control };
        let word = if cfg!(target_os = "macos") { m.alt } else { m.control };
        let cursor = self.cursor();
        let go = |this: &mut Self, to: usize| if m.shift { this.select_to(to) } else { this.move_to(to) };
        match ks.key.as_str() {
            "left" => {
                let to = if m.platform {
                    self.line_start(cursor)
                } else if word {
                    self.previous_word(cursor)
                } else if !m.shift && !self.selected.is_empty() {
                    self.selected.start
                } else {
                    self.previous_boundary(cursor)
                };
                go(self, to);
            }
            "right" => {
                let to = if m.platform {
                    self.line_end(cursor)
                } else if word {
                    self.next_word(cursor)
                } else if !m.shift && !self.selected.is_empty() {
                    self.selected.end
                } else {
                    self.next_boundary(cursor)
                };
                go(self, to);
            }
            "home" => go(self, if shortcut { 0 } else { self.line_start(cursor) }),
            "end" => go(self, if shortcut { self.text.len() } else { self.line_end(cursor) }),
            "up" if self.multiline => {
                let to = if m.platform { 0 } else { self.vertical(-1.) };
                go(self, to);
            }
            "down" if self.multiline => {
                let to = if m.platform { self.text.len() } else { self.vertical(1.) };
                go(self, to);
            }
            "backspace" => {
                let mut range = self.selected.clone();
                if range.is_empty() {
                    range.start = if word { self.previous_word(cursor) } else { self.previous_boundary(cursor) };
                }
                self.replace(range, "", cx);
            }
            "delete" => {
                let mut range = self.selected.clone();
                if range.is_empty() {
                    range.end = if word { self.next_word(cursor) } else { self.next_boundary(cursor) };
                }
                self.replace(range, "", cx);
            }
            "enter" if self.multiline && self.submit_on_return && !m.shift && !m.alt && !shortcut => self.submit(cx),
            "enter" if self.multiline && !shortcut => self.replace(self.selected.clone(), "\n", cx),
            "enter" => self.submit(cx),
            "a" if shortcut => {
                self.selected = 0..self.text.len();
                self.reversed = false;
            }
            "c" | "x" if shortcut => {
                if !self.secure && !self.selected.is_empty() {
                    cx.write_to_clipboard(ClipboardItem::new_string(self.text[self.selected.clone()].to_owned()));
                    if ks.key == "x" {
                        self.replace(self.selected.clone(), "", cx);
                    }
                }
            }
            "v" if shortcut => {
                let item = cx.read_from_clipboard();
                let images: Vec<PastedImage> = match &item {
                    Some(item) if self.accepts_media => item
                        .entries
                        .iter()
                        .filter_map(|entry| match entry {
                            gpui::ClipboardEntry::Image(image) => Some(PastedImage {
                                bytes: image.bytes.clone().into(),
                                mime: image.format.mime_type().into(),
                            }),
                            _ => None,
                        })
                        .collect(),
                    _ => Vec::new(),
                };
                if !images.is_empty() {
                    if let Some(sink) = &self.sink {
                        sink.send(ImagesPasted { images }, cx);
                    }
                } else if let Some(text) = item.and_then(|item| item.text()) {
                    self.replace(self.selected.clone(), &text, cx);
                }
            }
            _ => return false,
        }
        true
    }
}

/// The editor as GPUI's platform input handler: text, IME composition, candidate placement.
struct Input(Rc<std::cell::RefCell<Editor>>);

impl gpui::InputHandler for Input {
    fn selected_text_range(&mut self, _: bool, _: &mut Window, _: &mut App) -> Option<UTF16Selection> {
        let e = self.0.borrow();
        Some(UTF16Selection { range: e.range_to_utf16_units(&e.selected), reversed: e.reversed })
    }

    fn marked_text_range(&mut self, _: &mut Window, _: &mut App) -> Option<Range<usize>> {
        let e = self.0.borrow();
        e.marked.as_ref().map(|r| e.range_to_utf16_units(r))
    }

    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut App,
    ) -> Option<String> {
        let e = self.0.borrow();
        if e.secure {
            return None;
        }
        let range = e.utf16_to_range(&range);
        adjusted.replace(e.range_to_utf16_units(&range));
        Some(e.text[range].to_owned())
    }

    fn replace_text_in_range(&mut self, range: Option<Range<usize>>, text: &str, window: &mut Window, cx: &mut App) {
        let mut e = self.0.borrow_mut();
        let range = range.as_ref().map(|r| e.utf16_to_range(r)).or(e.marked.clone()).unwrap_or(e.selected.clone());
        e.replace(range, text, cx);
        drop(e);
        window.refresh();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
        window: &mut Window,
        _: &mut App,
    ) {
        let mut e = self.0.borrow_mut();
        if !e.editable {
            return;
        }
        let range = range.as_ref().map(|r| e.utf16_to_range(r)).or(e.marked.clone()).unwrap_or(e.selected.clone());
        e.text.replace_range(range.clone(), text);
        e.marked = (!text.is_empty()).then(|| range.start..range.start + text.len());
        // Composition is provisional: the app hears the text once it is committed.
        let caret = match selected {
            Some(s) => {
                let s = (s.start + range.start)..(s.end + range.start);
                let start = e.utf16_to_offset(e.offset_to_utf16(range.start) + s.start - range.start);
                let end = e.utf16_to_offset(e.offset_to_utf16(range.start) + s.end - range.start);
                start..end
            }
            None => range.start + text.len()..range.start + text.len(),
        };
        e.selected = caret;
        e.reversed = false;
        e.reveal = true;
        drop(e);
        window.refresh();
    }

    fn unmark_text(&mut self, window: &mut Window, cx: &mut App) {
        let mut e = self.0.borrow_mut();
        if e.marked.take().is_some() {
            e.edited(cx);
        }
        drop(e);
        window.refresh();
    }

    fn bounds_for_range(&mut self, range: Range<usize>, _: &mut Window, _: &mut App) -> Option<Bounds<Pixels>> {
        let e = self.0.borrow();
        let (_, origin) = e.painted?;
        let range = e.utf16_to_range(&range);
        let (a, b) = (e.position(range.start), e.position(range.end));
        Some(Bounds::from_corners(origin + a, origin + point(b.x.max(a.x), a.y + e.line_height)))
    }

    fn character_index_for_point(&mut self, at: Point<Pixels>, _: &mut Window, _: &mut App) -> Option<usize> {
        let e = self.0.borrow();
        Some(e.offset_to_utf16(e.offset_at(at)))
    }

    fn accepts_text_input(&mut self, _: &mut Window, _: &mut App) -> bool {
        self.0.borrow().editable
    }
}

/// Lays out and paints the editor's text, selection and caret, and registers the input handler
/// while the field has focus.
pub(crate) struct EditorElement {
    editor: Rc<std::cell::RefCell<Editor>>,
    focus: FocusHandle,
    look: FormLook,
    placeholder: Option<SharedString>,
    enabled: bool,
}

pub(crate) struct Prepainted {
    origin: Point<Pixels>,
    placeholder: Option<ShapedLine>,
    selection: Vec<Bounds<Pixels>>,
    caret: Option<Bounds<Pixels>>,
}

impl IntoElement for EditorElement {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl gpui::Element for EditorElement {
    type RequestLayoutState = ();
    type PrepaintState = Prepainted;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        if self.editor.borrow().multiline {
            style.size.width = relative(1.).into();
            style.flex_grow = 1.;
            style.min_size.height = self.look.line_height.into();
        } else {
            style.flex_grow = 1.;
            style.flex_basis = px(0.).into();
            style.min_size.width = px(0.).into();
            style.size.height = self.look.line_height.into();
        }
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        _: &mut App,
    ) -> Prepainted {
        let look = self.look;
        let mut e = self.editor.borrow_mut();
        e.line_height = look.line_height;
        let style = window.text_style();
        let font = style.font();
        let color = if self.enabled { look.text } else { look.secondary_text };
        let display = e.display();
        let run = |len: usize, underline: bool| TextRun {
            len,
            font: font.clone(),
            color,
            background_color: None,
            underline: underline.then_some(UnderlineStyle { color: Some(color), thickness: px(1.), wavy: false }),
            strikethrough: None,
        };
        let runs: Vec<TextRun> = match e.marked.clone() {
            Some(m) => {
                let (a, b) = (e.text_to_display(m.start), e.text_to_display(m.end));
                [run(a, false), run(b - a, true), run(display.len() - b, false)]
                    .into_iter()
                    .filter(|r| r.len > 0)
                    .collect()
            }
            None => vec![run(display.len(), false)],
        };
        let text_system = window.text_system().clone();
        let placeholder = match (&self.placeholder, display.is_empty()) {
            (Some(p), true) => {
                let run = TextRun { color: look.secondary_text, ..run(p.len(), false) };
                Some(text_system.shape_line(p.clone(), look.text_size, &[run], None))
            }
            _ => None,
        };
        let layout = if e.multiline {
            let lines = text_system
                .shape_text(display.clone().into(), look.text_size, &runs, Some(bounds.size.width), None)
                .unwrap_or_default();
            let mut top = px(0.);
            let mut placed = Vec::new();
            for line in lines {
                let height = line.size(look.line_height).height.max(look.line_height);
                placed.push((line, top));
                top += height;
            }
            Layout::Lines(placed)
        } else {
            Layout::Line(Box::new(text_system.shape_line(display.clone().into(), look.text_size, &runs, None)))
        };
        e.layout = Some(layout);

        let caret = e.position(e.cursor());
        if e.multiline {
            let content = match &e.layout {
                Some(Layout::Lines(lines)) => {
                    lines.last().map_or(px(0.), |(l, top)| *top + l.size(look.line_height).height.max(look.line_height))
                }
                _ => px(0.),
            };
            if e.reveal {
                if caret.y + look.line_height > e.scroll.y + bounds.size.height {
                    e.scroll.y = caret.y + look.line_height - bounds.size.height;
                }
                if caret.y < e.scroll.y {
                    e.scroll.y = caret.y;
                }
            }
            e.scroll.y = e.scroll.y.min((content - bounds.size.height).max(px(0.))).max(px(0.));
            e.scroll.x = px(0.);
            if (content - e.content).abs() > px(0.5) {
                // The area sizes itself from this on its next build.
                e.content = content;
                window.request_animation_frame();
            }
        } else {
            let width = match &e.layout {
                Some(Layout::Line(line)) => line.width,
                _ => px(0.),
            };
            let room = (bounds.size.width - px(2.)).max(px(0.));
            if e.reveal {
                if caret.x - e.scroll.x > room {
                    e.scroll.x = caret.x - room;
                }
                if caret.x < e.scroll.x {
                    e.scroll.x = caret.x;
                }
            }
            e.scroll.x = e.scroll.x.min((width - room).max(px(0.))).max(px(0.));
            e.scroll.y = px(0.);
        }
        e.reveal = false;
        let origin = bounds.origin - e.scroll;
        e.painted = Some((bounds, origin));

        let line_height = look.line_height;
        let mut selection = Vec::new();
        if !e.selected.is_empty() {
            let (a, b) = (e.position(e.selected.start), e.position(e.selected.end));
            if a.y == b.y {
                selection.push(Bounds::from_corners(origin + a, origin + point(b.x, b.y + line_height)));
            } else {
                let right = bounds.size.width;
                selection.push(Bounds::from_corners(origin + a, origin + point(right, a.y + line_height)));
                if b.y > a.y + line_height {
                    selection.push(Bounds::from_corners(
                        origin + point(px(0.), a.y + line_height),
                        origin + point(right, b.y),
                    ));
                }
                selection
                    .push(Bounds::from_corners(origin + point(px(0.), b.y), origin + point(b.x, b.y + line_height)));
            }
        }
        let caret = (self.enabled && e.selected.is_empty() && self.focus.is_focused(window))
            .then(|| Bounds::new(origin + caret, size(px(1.5), line_height)));
        Prepainted { origin, placeholder, selection, caret }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        prepainted: &mut Prepainted,
        window: &mut Window,
        cx: &mut App,
    ) {
        if self.enabled {
            window.handle_input(&self.focus, Input(self.editor.clone()), cx);
        }
        let look = self.look;
        let mask = gpui::ContentMask { bounds };
        window.with_content_mask(Some(mask), |window| {
            for s in prepainted.selection.drain(..) {
                window.paint_quad(fill(s, look.selection));
            }
            if let Some(p) = prepainted.placeholder.take() {
                let _ = p.paint(prepainted.origin, look.line_height, gpui::TextAlign::Left, None, window, cx);
            }
            let e = self.editor.borrow();
            match &e.layout {
                Some(Layout::Line(line)) => {
                    let _ = line.paint(prepainted.origin, look.line_height, gpui::TextAlign::Left, None, window, cx);
                }
                Some(Layout::Lines(lines)) => {
                    for (line, top) in lines {
                        let at = prepainted.origin + point(px(0.), *top);
                        let _ = line.paint(at, look.line_height, gpui::TextAlign::Left, None, window, cx);
                    }
                }
                None => {}
            }
            if let Some(caret) = prepainted.caret {
                window.paint_quad(fill(caret, look.text));
            }
        });
    }
}

/// What the builders below share: which editor, which prop text, how it behaves.
pub(crate) struct Field {
    pub multiline: bool,
    pub secure: bool,
    pub editable: bool,
    pub enabled: bool,
    pub text: SharedString,
    pub placeholder: Option<SharedString>,
    pub options: Vec<SharedString>,
    pub completes: bool,
}

fn editor(
    b: &Build,
    id: NodeId,
    f: &Field,
    commit: impl FnOnce() -> Commit,
    window: &mut Window,
    cx: &mut App,
) -> (Ctl, Rc<std::cell::RefCell<Editor>>) {
    let (ctl, editor) = b.ctl(id, f.enabled, window, cx, || Editor::new(f.multiline, commit()));
    prepare(&ctl, &editor, f, cx);
    (ctl, editor)
}

/// Brings the editor up to date with this build's props. Leaving the field commits a parsed
/// value and closes suggestions.
pub(crate) fn prepare(ctl: &Ctl, editor: &Rc<std::cell::RefCell<Editor>>, f: &Field, cx: &mut App) {
    let mut e = editor.borrow_mut();
    e.sink = Some(ctl.sink.clone());
    e.secure = f.secure;
    e.editable = f.editable && f.enabled;
    e.options = f.options.clone();
    e.completes = f.completes;
    if !ctl.focused {
        e.commit(cx);
        e.open = false;
        e.selecting = false;
    }
    e.sync(&f.text);
}

/// A single-line field: chrome, keys and text.
pub(crate) fn line(ctl: &Ctl, editor: &Rc<std::cell::RefCell<Editor>>, f: &Field) -> gpui::Stateful<gpui::Div> {
    let mut field = chrome(ctl, editor, f);
    if f.enabled {
        field = field.on_key_down(on_key(editor));
    }
    field.child(text_element(ctl, editor, f))
}

/// The field chrome: fill, focus ring, pointer and key handling around an [`EditorElement`].
pub(crate) fn chrome(ctl: &Ctl, editor: &Rc<std::cell::RefCell<Editor>>, f: &Field) -> gpui::Stateful<gpui::Div> {
    let look = ctl.look;
    let mut chrome = div()
        .id(ElementId::Integer(ctl.id.0))
        .flex()
        .gap(look.gap)
        .px(look.padding_x)
        .rounded(look.radius)
        .bg(look.fill)
        .border_2()
        .border_color(if ctl.focused { look.focus_ring } else { gpui::transparent_black() })
        .text_size(look.text_size)
        .line_height(look.line_height)
        .text_color(look.text);
    chrome = if f.multiline {
        chrome.flex_col().py(look.padding_y).overflow_hidden()
    } else {
        chrome.flex_row().items_center().h(look.control_height).min_w(look.field_min_width)
    };
    if !f.enabled {
        return chrome.opacity(look.disabled_opacity);
    }
    let down = {
        let (editor, sink) = (editor.clone(), ctl.sink.clone());
        move |event: &MouseDownEvent, window: &mut Window, cx: &mut App| {
            sink.take_focus(window, cx);
            let mut e = editor.borrow_mut();
            let at = e.offset_at(event.position);
            if event.click_count >= 3 {
                let (start, end) = (e.line_start(at), e.line_end(at));
                e.selected = start..end;
            } else if event.click_count == 2 {
                let (start, end) = (e.previous_word(e.next_boundary(at)), e.next_word(at));
                e.selected = start..end.max(start);
            } else if event.modifiers.shift {
                e.select_to(at);
            } else {
                e.move_to(at);
            }
            e.selecting = true;
            drop(e);
            window.refresh();
        }
    };
    let moved = {
        let editor = editor.clone();
        move |event: &MouseMoveEvent, window: &mut Window, _: &mut App| {
            let mut e = editor.borrow_mut();
            if e.selecting && event.dragging() {
                let at = e.offset_at(event.position);
                e.select_to(at);
                drop(e);
                window.refresh();
            }
        }
    };
    let up = {
        let editor = editor.clone();
        move |_: &gpui::MouseUpEvent, _: &mut Window, _: &mut App| editor.borrow_mut().selecting = false
    };
    let scrolled = {
        let editor = editor.clone();
        move |event: &ScrollWheelEvent, window: &mut Window, _: &mut App| {
            let delta = event.delta.pixel_delta(px(20.));
            editor.borrow_mut().scroll.y -= delta.y;
            window.refresh();
        }
    };
    chrome = chrome
        .track_focus(ctl.focus())
        .cursor(CursorStyle::IBeam)
        .on_mouse_down(MouseButton::Left, down)
        .on_mouse_move(moved)
        .on_mouse_up(MouseButton::Left, up.clone())
        .on_mouse_up_out(MouseButton::Left, up);
    if f.multiline {
        chrome = chrome.on_scroll_wheel(scrolled);
    }
    chrome
}

pub(crate) fn on_key(
    editor: &Rc<std::cell::RefCell<Editor>>,
) -> impl Fn(&KeyDownEvent, &mut Window, &mut App) + 'static {
    let editor = editor.clone();
    move |event: &KeyDownEvent, window: &mut Window, cx: &mut App| {
        if editor.borrow_mut().key(&event.keystroke, cx) {
            cx.stop_propagation();
            window.refresh();
        }
    }
}

pub(crate) fn text_element(ctl: &Ctl, editor: &Rc<std::cell::RefCell<Editor>>, f: &Field) -> EditorElement {
    EditorElement {
        editor: editor.clone(),
        focus: ctl.focus().clone(),
        look: ctl.look,
        placeholder: f.placeholder.clone(),
        enabled: f.enabled,
    }
}

/// A clickable edge icon reporting `event`.
fn edge_button<E: Clone + 'static>(
    ctl: &Ctl,
    key: &'static str,
    g: Glyph,
    label: SharedString,
    event: E,
    enabled: bool,
) -> impl IntoElement {
    let look = ctl.look;
    let sink = ctl.sink.clone();
    let button = div()
        .id(key)
        .role(gpui::Role::Button)
        .aria_label(label)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(look.icon_size + px(4.))
        .rounded(look.radius)
        .child(glyph(g, look.icon_size, look.secondary_text, look.glyph_stroke));
    if !enabled {
        return button;
    }
    button
        .occlude()
        .cursor(CursorStyle::PointingHand)
        .hover(move |s| s.bg(look.fill_hover))
        .on_click(move |_, _, cx| sink.send(event.clone(), cx))
}

pub(crate) fn text_field(
    b: &Build,
    id: NodeId,
    p: &TextFieldProps,
    window: &mut Window,
    cx: &mut App,
) -> gpui::AnyElement {
    let f = Field {
        multiline: false,
        secure: p.field == FieldStyle::Secure,
        editable: p.editable,
        enabled: p.enabled,
        text: p.text.clone(),
        placeholder: p.placeholder.clone(),
        options: Vec::new(),
        completes: false,
    };
    let (ctl, editor) = editor(b, id, &f, || Commit::Text, window, cx);
    let look = ctl.look;
    let role = match p.field {
        FieldStyle::Plain => gpui::Role::TextInput,
        FieldStyle::Secure => gpui::Role::PasswordInput,
        FieldStyle::Search => gpui::Role::SearchInput,
    };
    let name = p.label.clone().or_else(|| p.tooltip.clone()).or_else(|| p.placeholder.clone()).unwrap_or_default();
    let mut field = chrome(&ctl, &editor, &f).role(role).aria_label(name);
    if p.field != FieldStyle::Secure {
        field = field.aria_value(p.text.clone());
    }
    if let Some(placeholder) = &p.placeholder {
        field = field.aria_placeholder(placeholder.clone());
    }
    if p.enabled {
        field = field.on_key_down(on_key(&editor));
    }
    if p.field == FieldStyle::Search {
        field = field.child(glyph(Glyph::Search, look.icon_size, look.secondary_text, look.glyph_stroke));
    }
    if let Some(icon) = &p.leading {
        let event = FieldIconClicked { edge: IconEdge::Leading };
        field = field.child(edge_button(&ctl, "leading", Glyph::of(icon.icon), icon.label.clone(), event, p.enabled));
    }
    field = field.child(text_element(&ctl, &editor, &f));
    if p.field == FieldStyle::Search && !p.text.is_empty() && p.editable {
        let clear = TextChanged { text: SharedString::default() };
        field = field.child(edge_button(&ctl, "clear", Glyph::Clear, "Clear".into(), clear, p.enabled));
    }
    if let Some(icon) = &p.trailing {
        let event = FieldIconClicked { edge: IconEdge::Trailing };
        field = field.child(edge_button(&ctl, "trailing", Glyph::of(icon.icon), icon.label.clone(), event, p.enabled));
    }
    with_style(field, &p.style).into_any_element()
}

pub(crate) fn text_area(
    b: &Build,
    id: NodeId,
    p: &TextAreaProps,
    window: &mut Window,
    cx: &mut App,
) -> gpui::AnyElement {
    let f = Field {
        multiline: true,
        secure: false,
        editable: p.editable,
        enabled: p.enabled,
        text: p.text.clone(),
        placeholder: p.placeholder.clone(),
        options: Vec::new(),
        completes: false,
    };
    let (ctl, editor) = editor(b, id, &f, || Commit::Text, window, cx);
    let look = ctl.look;
    let name = p.label.clone().or_else(|| p.tooltip.clone()).or_else(|| p.placeholder.clone()).unwrap_or_default();
    let (content, picker) = {
        let mut e = editor.borrow_mut();
        e.submit_on_return = p.submit_on_return;
        e.accepts_media = p.accepts_media;
        // Each new request opens the picker; the first one seen is where the app started.
        if e.emoji.is_some_and(|seen| seen != p.emoji_picker) && p.enabled {
            e.emoji_open = true;
        }
        e.emoji = Some(p.emoji_picker);
        (e.content, e.emoji_open && p.enabled)
    };
    let min = look.line_height * f32::from(p.min_lines.max(1));
    let text_height = match p.max_lines {
        Some(max) => content.max(min).min(look.line_height * f32::from(max.max(p.min_lines).max(1))),
        None => min,
    };
    let mut area = chrome(&ctl, &editor, &f)
        .role(gpui::Role::MultilineTextInput)
        .aria_label(name)
        .aria_value(p.text.clone())
        .min_w(look.field_min_width)
        .h(text_height + look.padding_y * 2. + px(4.));
    // A rounded area (a chat composer) keeps its outline at rest.
    let rounded = p.style.corner_radii.top_left.is_some();
    if rounded && !ctl.focused {
        area = area.border_color(look.border);
    }
    if p.enabled {
        area = area.on_key_down(on_key(&editor));
        if p.accepts_media {
            let sink = ctl.sink.clone();
            area = area.on_drop(move |paths: &gpui::ExternalPaths, _: &mut Window, cx: &mut App| {
                sink.send(FilesDropped { paths: paths.paths().to_vec() }, cx);
            });
        }
    }
    let area = with_style(area.child(text_element(&ctl, &editor, &f)), &p.style);
    if !picker {
        return area.into_any_element();
    }
    let close = {
        let editor = editor.clone();
        move |_: &MouseDownEvent, window: &mut Window, _: &mut App| {
            editor.borrow_mut().emoji_open = false;
            window.refresh();
        }
    };
    let grid =
        EMOJI.iter().enumerate().fold(div().w(px(8. * 34.)).flex().flex_row().flex_wrap(), |grid, (i, emoji)| {
            let (editor, sink) = (editor.clone(), ctl.sink.clone());
            grid.child(
                div()
                    .id(ElementId::Integer(i as u64))
                    .role(gpui::Role::Button)
                    .aria_label(*emoji)
                    .size(px(34.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(look.radius)
                    .text_size(px(20.))
                    .cursor(CursorStyle::PointingHand)
                    .hover(move |s| s.bg(look.fill_hover))
                    .child(*emoji)
                    .on_click(move |_, window, cx| {
                        sink.take_focus(window, cx);
                        let mut e = editor.borrow_mut();
                        e.emoji_open = false;
                        let at = e.selected.clone();
                        e.replace(at, emoji, cx);
                        drop(e);
                        window.refresh();
                    }),
            )
        });
    div().flex().flex_col().child(area).child(popover(b, id, window, look, close, [grid])).into_any_element()
}

/// What the drawn emoji picker offers.
static EMOJI: [&str; 32] = [
    "\u{1f600}",
    "\u{1f602}",
    "\u{1f60a}",
    "\u{1f60d}",
    "\u{1f618}",
    "\u{1f60e}",
    "\u{1f914}",
    "\u{1f644}",
    "\u{1f62d}",
    "\u{1f621}",
    "\u{1f631}",
    "\u{1f634}",
    "\u{1f973}",
    "\u{1f607}",
    "\u{1f643}",
    "\u{1f609}",
    "\u{1f44d}",
    "\u{1f44e}",
    "\u{1f44f}",
    "\u{1f64f}",
    "\u{1f44b}",
    "\u{1f4aa}",
    "\u{1f440}",
    "\u{1f525}",
    "\u{2764}\u{fe0f}",
    "\u{1f494}",
    "\u{2728}",
    "\u{1f389}",
    "\u{1f4af}",
    "\u{2705}",
    "\u{274c}",
    "\u{1f680}",
];

pub(crate) fn combo_box(
    b: &Build,
    id: NodeId,
    p: &ComboBoxProps,
    window: &mut Window,
    cx: &mut App,
) -> gpui::AnyElement {
    let f = Field {
        multiline: false,
        secure: false,
        editable: p.editable,
        enabled: p.enabled,
        text: p.text.clone(),
        placeholder: p.placeholder.clone(),
        options: p.options.clone(),
        completes: p.completes,
    };
    let (ctl, editor) = editor(b, id, &f, || Commit::Text, window, cx);
    let look = ctl.look;
    let open = editor.borrow().open && p.enabled;
    let name = p.label.clone().or_else(|| p.tooltip.clone()).or_else(|| p.placeholder.clone()).unwrap_or_default();
    let pick = {
        let (editor, sink, options) = (editor.clone(), ctl.sink.clone(), p.options.clone());
        Rc::new(move |index: usize, window: &mut Window, cx: &mut App| {
            let Some(option) = options.get(index) else { return };
            let mut e = editor.borrow_mut();
            e.open = false;
            e.highlight = None;
            drop(e);
            sink.send(TextChanged { text: option.clone() }, cx);
            sink.send(SelectChanged { index }, cx);
            window.refresh();
        })
    };
    let mut field = chrome(&ctl, &editor, &f)
        .role(gpui::Role::ComboBox)
        .aria_label(name)
        .aria_value(p.text.clone())
        .aria_expanded(open)
        .pr(px(2.));
    if p.enabled {
        let (keys, editor, pick, count) = (on_key(&editor), editor.clone(), pick.clone(), p.options.len());
        field = field.on_key_down(move |event: &KeyDownEvent, window: &mut Window, cx: &mut App| {
            let mut e = editor.borrow_mut();
            let step = match event.keystroke.key.as_str() {
                "down" => Some(1isize),
                "up" => Some(-1),
                "escape" if e.open => {
                    e.open = false;
                    cx.stop_propagation();
                    window.refresh();
                    return;
                }
                "enter" if e.open && e.highlight.is_some() => {
                    let index = e.highlight.unwrap_or(0);
                    drop(e);
                    cx.stop_propagation();
                    pick(index, window, cx);
                    return;
                }
                _ => None,
            };
            if let Some(step) = step
                && count > 0
            {
                let next = match (e.open, e.highlight) {
                    (true, Some(h)) => (h as isize + step).clamp(0, count as isize - 1) as usize,
                    _ => {
                        if step > 0 {
                            0
                        } else {
                            count - 1
                        }
                    }
                };
                e.open = true;
                e.highlight = Some(next);
                cx.stop_propagation();
                window.refresh();
                return;
            }
            drop(e);
            keys(event, window, cx);
        });
    }
    field = field.child(text_element(&ctl, &editor, &f));
    let toggle = {
        let (editor, sink) = (editor.clone(), ctl.sink.clone());
        move |_: &gpui::ClickEvent, window: &mut Window, cx: &mut App| {
            sink.take_focus(window, cx);
            let mut e = editor.borrow_mut();
            e.open = !e.open;
            e.highlight = None;
            drop(e);
            window.refresh();
        }
    };
    let mut button = div()
        .id("toggle")
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(look.control_height - px(8.))
        .rounded(look.radius)
        .child(glyph(Glyph::ChevronDown, look.icon_size, look.text, look.glyph_stroke));
    if p.enabled {
        button = button.occlude().hover(move |s| s.bg(look.fill_hover)).on_click(toggle);
    }
    field = field.child(button);
    let mut root = div().flex().flex_col().child(with_style(field, &p.style));
    if open && !p.options.is_empty() {
        let highlight = editor.borrow().highlight;
        let close = {
            let editor = editor.clone();
            move |_: &MouseDownEvent, window: &mut Window, _: &mut App| {
                editor.borrow_mut().open = false;
                window.refresh();
            }
        };
        let items = p.options.iter().enumerate().map(|(index, option)| {
            let pick = pick.clone();
            menu_item(look, index, option.clone(), highlight == Some(index), false)
                .on_click(move |_, window, cx| pick(index, window, cx))
        });
        root = root.child(popover(b, id, window, look, close, items));
    }
    root.into_any_element()
}

/// A menu row: highlighted under the pointer or the keyboard, a check beside the current one.
pub(crate) fn menu_item(
    look: FormLook,
    index: usize,
    title: SharedString,
    highlighted: bool,
    current: bool,
) -> gpui::Stateful<gpui::Div> {
    let item = div()
        .id(ElementId::Integer(index as u64))
        .role(gpui::Role::MenuItem)
        .aria_label(title.clone())
        .aria_selected(current)
        .flex()
        .flex_row()
        .items_center()
        .gap(look.gap)
        .h(look.control_height)
        .px(look.padding_x)
        .rounded(look.radius)
        .whitespace_nowrap()
        .cursor(CursorStyle::PointingHand)
        .hover(move |s| s.bg(look.fill_hover))
        .child(div().flex_1().child(title));
    let item = if highlighted { item.bg(look.fill_active) } else { item };
    if current { item.child(glyph(Glyph::Check, look.icon_size, look.text, look.glyph_stroke)) } else { item }
}

/// A light-dismiss popover below its control, in the window's overlay layer: above every
/// overlay its control sits in, with a backdrop that consumes an outside press and closes it, as
/// `crate::overlay` does for nd overlays.
#[allow(clippy::too_many_arguments)]
pub(crate) fn popover(
    b: &Build,
    id: NodeId,
    window: &Window,
    look: FormLook,
    close: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    items: impl IntoIterator<Item = impl IntoElement>,
) -> impl IntoElement {
    let viewport = window.viewport_size();
    let (backend, window_id) = (b.backend.clone(), b.window_id);
    let backdrop = div()
        .id("nd-form-backdrop")
        .block_mouse_except_scroll()
        .absolute()
        .top_0()
        .left_0()
        .w(viewport.width)
        .h(viewport.height)
        .on_any_mouse_down(move |event, window, cx| {
            cx.stop_propagation();
            backend.swallow_release(window_id);
            close(event, window, cx);
        });
    let card = anchored().snap_to_window_with_margin(px(8.)).child(
        div()
            .id("popover")
            .occlude()
            .role(gpui::Role::MenuListPopup)
            .mt(px(4.))
            .min_w(px(160.))
            .p(look.popover_inset)
            .flex()
            .flex_col()
            .rounded(look.popover_radius)
            .bg(look.popover)
            .border_1()
            .border_color(look.border)
            .shadow(vec![gpui::BoxShadow {
                color: gpui::hsla(0., 0., 0., 0.18),
                offset: point(px(0.), px(4.)),
                blur_radius: px(12.),
                spread_radius: px(0.),
                inset: false,
            }])
            .text_size(look.text_size)
            .text_color(look.text)
            .children(items),
    );
    deferred(div().child(anchored().position(point(px(0.), px(0.))).child(backdrop)).child(card))
        .with_priority(crate::overlay::PANEL_PRIORITY + 1 + b.overlay_depth(id))
}

/// A single-line field whose text is a value's rendering, committing through `parse`.
pub(crate) fn value_field(
    b: &Build,
    id: NodeId,
    text: SharedString,
    enabled: bool,
    parse: impl Fn(&str) -> Option<Box<dyn Any>> + 'static,
    window: &mut Window,
    cx: &mut App,
) -> (Ctl, gpui::Stateful<gpui::Div>) {
    let f = Field {
        multiline: false,
        secure: false,
        editable: true,
        enabled,
        text,
        placeholder: None,
        options: Vec::new(),
        completes: false,
    };
    let (ctl, editor) = editor(b, id, &f, || Commit::Parse(Rc::new(parse)), window, cx);
    let field = line(&ctl, &editor, &f);
    (ctl, field)
}
