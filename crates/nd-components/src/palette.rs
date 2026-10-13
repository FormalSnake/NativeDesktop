//! `command_palette()`: a Spotlight-style command bar over the window (ARCHITECTURE.md 3.11).
//!
//! Controlled: the app owns `query` and `items` and does all filtering and ordering; the palette
//! renders the rows in order and never filters them. Every edit fires [`PaletteQueryChanged`];
//! a present starts from the app's last `query` with the whole text selected. Return on a row
//! fires [`PaletteActivated`], Return with no highlight (or secondary-Return always) fires
//! [`PaletteSubmitted`] with the typed text, Esc or a click outside fires [`PaletteCancelled`];
//! an app-driven close does not echo a cancel.
//!
//! [`PaletteModel`] is the behaviour both backends share: highlight, inline completion and what
//! each key means. The AppKit mapping and the drawn one only draw it and feed it input.

use std::sync::Arc;

use nd_core::semantic::Role;
use nd_ui::backend::Props;
use nd_ui::prelude::*;
use nd_ui::{Key, NativeElement, StyleRefinement};

use crate::icon::Icon;

/// The palette's own geometry, the same on every backend. These are the palette's design, not
/// system metrics: the system draws the controls inside it (field, table, selection).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PaletteMetrics {
    pub width: f32,
    pub row_height: f32,
    /// A ceiling: the card's height follows its row count up to it.
    pub max_list_height: f32,
    /// The card's top edge sits at this fraction of the window height, so the field stays put
    /// while the list below it grows and shrinks.
    pub top_fraction: f32,
    /// Clearance the card keeps from every window edge.
    pub margin: f32,
    /// Black over the window behind the card.
    pub dim_alpha: f32,
    pub radius: f32,
    pub icon_side: f32,
    /// The box the search glyph sits in, in front of the field.
    pub search_icon: f32,
    /// The search glyph's point size inside that box.
    pub search_glyph: f32,
    pub search_icon_inset: f32,
    pub search_icon_gap: f32,
    pub field_font_size: f32,
    pub field_top: f32,
    /// The field's trailing inset: the icon inset, icon and gap again, so a long selected text
    /// stops as far from the right edge as it starts from the left.
    pub field_trailing: f32,
    pub separator_gap: f32,
    pub list_gap: f32,
    pub cell_inset: f32,
    pub cell_gap: f32,
    pub subtitle_gap: f32,
    pub hint_gap: f32,
    /// The most of a row the title may take, so a long title still leaves room for its subtitle.
    pub title_share: f32,
    pub shadow_radius: f32,
    pub shadow_opacity: f32,
}

pub const PALETTE: PaletteMetrics = PaletteMetrics {
    width: 640.,
    row_height: 40.,
    max_list_height: 400.,
    top_fraction: 0.18,
    margin: 20.,
    dim_alpha: 0.15,
    radius: 16.,
    icon_side: 16.,
    search_icon: 20.,
    search_glyph: 18.,
    search_icon_inset: 18.,
    search_icon_gap: 10.,
    field_font_size: 20.,
    field_top: 16.,
    field_trailing: 48.,
    separator_gap: 14.,
    list_gap: 6.,
    cell_inset: 10.,
    cell_gap: 10.,
    subtitle_gap: 8.,
    hint_gap: 16.,
    title_share: 0.62,
    shadow_radius: 28.,
    shadow_opacity: 0.32,
};

/// One result row, drawn on one line: icon, title, subtitle in secondary text after it, and a
/// right-aligned hint (the action Return takes, or a shortcut). `id` comes back in
/// [`PaletteActivated`].
#[derive(Clone, Debug, PartialEq)]
pub struct PaletteItem {
    pub id: SharedString,
    pub title: SharedString,
    pub subtitle: Option<SharedString>,
    pub icon: Option<Icon>,
    /// Encoded image bytes (a favicon); wins over `icon`.
    pub icon_bytes: Option<Arc<[u8]>>,
    pub hint: Option<SharedString>,
    /// On the first row only: the text the field completes to inline (see [`PaletteModel`]).
    pub completion: Option<SharedString>,
}

pub fn palette_item(id: impl Into<SharedString>, title: impl Into<SharedString>) -> PaletteItem {
    PaletteItem {
        id: id.into(),
        title: title.into(),
        subtitle: None,
        icon: None,
        icon_bytes: None,
        hint: None,
        completion: None,
    }
}

impl PaletteItem {
    pub fn subtitle(mut self, subtitle: impl Into<SharedString>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }

    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn icon_bytes(mut self, bytes: impl Into<Arc<[u8]>>) -> Self {
        self.icon_bytes = Some(bytes.into());
        self
    }

    pub fn hint(mut self, hint: impl Into<SharedString>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub fn completion(mut self, completion: impl Into<SharedString>) -> Self {
        self.completion = Some(completion.into());
        self
    }

    /// What VoiceOver and AccessKit read for the row.
    pub fn accessible_label(&self) -> String {
        [Some(&self.title), self.subtitle.as_ref(), self.hint.as_ref()]
            .into_iter()
            .flatten()
            .filter(|s| !s.is_empty())
            .map(|s| s.as_ref())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

impl FluentBuilder for PaletteItem {}

#[derive(Clone, Debug, PartialEq)]
pub struct CommandPaletteProps {
    pub open: bool,
    pub placeholder: Option<SharedString>,
    pub query: SharedString,
    pub items: Vec<PaletteItem>,
    pub style: StyleRefinement,
}

/// The user changed the text: every keystroke, and an accepted completion. Never carries an
/// unaccepted completion.
#[derive(Clone, Debug, PartialEq)]
pub struct PaletteQueryChanged {
    pub text: SharedString,
}

/// Return or a click on a row.
#[derive(Clone, Debug, PartialEq)]
pub struct PaletteActivated {
    pub id: SharedString,
}

/// Return with no highlighted row, secondary-Return, or Return on a first row whose completion
/// the user took away: the typed text, never an unaccepted completion.
#[derive(Clone, Debug, PartialEq)]
pub struct PaletteSubmitted {
    pub text: SharedString,
}

/// Esc or a click outside the card.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PaletteCancelled;

/// Widget kind of `command_palette`.
pub struct CommandPaletteKind;

pub struct CommandPalette(NativeElement<CommandPaletteProps>);

fn describe(props: &Props) -> (Role, String) {
    let name = props
        .downcast_ref::<CommandPaletteProps>()
        .and_then(|p| p.placeholder.as_ref().map(|s| s.to_string()))
        .unwrap_or_else(|| "Command Palette".into());
    (Role::Dialog, name)
}

/// A palette over the active window. The element takes no space where it sits; place it anywhere
/// in the window's tree (`scaffold().overlay(..)` reads best).
pub fn command_palette(id: impl Into<SharedString>) -> CommandPalette {
    nd_ui::semantics::describe_widget::<CommandPaletteKind>(describe);
    let props = CommandPaletteProps {
        open: false,
        placeholder: None,
        query: SharedString::default(),
        items: Vec::new(),
        style: StyleRefinement::default(),
    };
    CommandPalette(NativeElement::new::<CommandPaletteKind>(Key::Id(id.into()), props).absolute().size_0())
}

impl CommandPalette {
    pub fn open(mut self, open: bool) -> Self {
        self.0.props_mut().open = open;
        self
    }

    pub fn placeholder(mut self, placeholder: impl Into<SharedString>) -> Self {
        self.0.props_mut().placeholder = Some(placeholder.into());
        self
    }

    /// The text a present starts from. Feeding back what [`PaletteQueryChanged`] reported changes
    /// nothing on screen.
    pub fn query(mut self, query: impl Into<SharedString>) -> Self {
        self.0.props_mut().query = query.into();
        self
    }

    /// The rows, already filtered and ordered.
    pub fn items(mut self, items: impl IntoIterator<Item = PaletteItem>) -> Self {
        self.0.props_mut().items = items.into_iter().collect();
        self
    }

    pub fn on_query_change(self, f: impl Fn(&PaletteQueryChanged, &mut Window, &mut App) + 'static) -> Self {
        CommandPalette(self.0.on(f))
    }

    pub fn on_activate(self, f: impl Fn(&PaletteActivated, &mut Window, &mut App) + 'static) -> Self {
        CommandPalette(self.0.on(f))
    }

    pub fn on_submit(self, f: impl Fn(&PaletteSubmitted, &mut Window, &mut App) + 'static) -> Self {
        CommandPalette(self.0.on(f))
    }

    pub fn on_cancel(self, f: impl Fn(&PaletteCancelled, &mut Window, &mut App) + 'static) -> Self {
        CommandPalette(self.0.on(f))
    }
}

impl FluentBuilder for CommandPalette {}

impl IntoElement for CommandPalette {
    fn into_any_element(mut self) -> AnyElement {
        let style = self.0.style().clone();
        self.0.props_mut().style = style;
        self.0.into_any_element()
    }
}

/// A key the palette's field handles itself. Everything else edits the text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaletteKey {
    Up,
    Down,
    Home,
    End,
    Tab,
    BackTab,
    /// Right arrow or end of line: accepts a shown completion, otherwise moves the caret.
    Right,
    Return,
    /// cmd-Return on macOS, ctrl-Return elsewhere: submits the typed text.
    SecondaryReturn,
    Escape,
}

/// What the palette reports to the app.
#[derive(Clone, Debug, PartialEq)]
pub enum PaletteEvent {
    QueryChanged(SharedString),
    Activated(SharedString),
    Submitted(SharedString),
    Cancelled,
}

/// The selection the field shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldSelection {
    /// The whole text, as a fresh present or a reseed leaves it.
    All,
    /// Caret at the end of the text.
    Caret,
    /// The completion after the typed text.
    Completion,
}

/// The palette's behaviour, backend-neutral. Feed it the app's props and the user's input; read
/// back what the field and the list show and the events to emit.
#[derive(Clone, Debug, Default)]
pub struct PaletteModel {
    rows: Vec<PaletteItem>,
    /// The app's last `query`; every present starts from it.
    controlled_query: SharedString,
    /// What the field holds as far as nd knows (3.13 rule 2): the last query applied or typed.
    /// An app feeding a reported query back finds it equal and nothing is reset.
    native_query: Option<SharedString>,
    /// What the user typed, without a completion shown after it.
    typed: String,
    /// The completion drawn selected after `typed`.
    suffix: String,
    /// Set by an edit that inserted text, cleared by one that removed text: Backspace over a
    /// completion must not bring it straight back.
    may_complete: bool,
    highlight: Option<usize>,
    presented: bool,
    select_all: bool,
}

impl PaletteModel {
    pub fn rows(&self) -> &[PaletteItem] {
        &self.rows
    }

    pub fn is_presented(&self) -> bool {
        self.presented
    }

    pub fn highlight(&self) -> Option<usize> {
        self.highlight
    }

    /// What the user typed.
    pub fn typed(&self) -> &str {
        &self.typed
    }

    /// The completion shown after the typed text, empty when none.
    pub fn completion(&self) -> &str {
        &self.suffix
    }

    /// The field's whole text: typed text and completion.
    pub fn field_text(&self) -> String {
        format!("{}{}", self.typed, self.suffix)
    }

    pub fn selection(&self) -> FieldSelection {
        if self.select_all {
            FieldSelection::All
        } else if self.suffix.is_empty() {
            FieldSelection::Caret
        } else {
            FieldSelection::Completion
        }
    }

    /// The app's `query`. Returns true when the field must be reseeded with it (and its text
    /// selected): only while presented and only when it differs from what the field holds.
    pub fn set_query(&mut self, query: &SharedString) -> bool {
        self.controlled_query = query.clone();
        if self.native_query.as_ref() == Some(query) {
            return false;
        }
        self.native_query = Some(query.clone());
        if !self.presented {
            return false;
        }
        self.typed = query.to_string();
        self.suffix.clear();
        self.may_complete = false;
        self.select_all = true;
        true
    }

    /// The app's `items`. Returns true when the rows changed: the list reloads and, while
    /// presented, the top row is highlighted. An equal list keeps the highlight.
    pub fn set_items(&mut self, items: &[PaletteItem]) -> bool {
        let changed = self.rows != items;
        if changed {
            self.rows = items.to_vec();
            if self.presented {
                self.highlight = (!self.rows.is_empty()).then_some(0);
            } else {
                self.highlight = None;
            }
        }
        if self.presented {
            self.apply_completion();
        }
        changed
    }

    /// Opens from the app's last `query`, its whole text selected, the top row highlighted.
    /// Returns false when already presented.
    pub fn present(&mut self) -> bool {
        if self.presented {
            return false;
        }
        self.presented = true;
        self.typed = self.controlled_query.to_string();
        self.native_query = Some(self.controlled_query.clone());
        self.suffix.clear();
        self.may_complete = false;
        self.select_all = true;
        self.highlight = (!self.rows.is_empty()).then_some(0);
        true
    }

    /// Closes. A user dismissal reports a cancel; an app-driven one (`open` false, an unmount)
    /// reports nothing.
    pub fn dismiss(&mut self, programmatic: bool) -> Option<PaletteEvent> {
        if !self.presented {
            return None;
        }
        self.presented = false;
        self.suffix.clear();
        self.select_all = false;
        (!programmatic).then_some(PaletteEvent::Cancelled)
    }

    /// The field's text after a user edit (a keystroke, a paste, automation typing).
    pub fn edited(&mut self, text: &str) -> Vec<PaletteEvent> {
        self.may_complete = text.chars().count() > self.typed.chars().count();
        self.typed = text.to_owned();
        self.suffix.clear();
        self.select_all = false;
        self.native_query = Some(text.into());
        let events = vec![PaletteEvent::QueryChanged(text.to_owned().into())];
        self.apply_completion();
        events
    }

    /// Characters typed where the field's selection is. For backends whose field is drawn.
    pub fn insert(&mut self, text: &str) -> Vec<PaletteEvent> {
        let base = if self.select_all { String::new() } else { self.typed.clone() };
        self.edited(&format!("{base}{text}"))
    }

    /// Backspace in a drawn field: removes the selection, or the last character.
    pub fn backspace(&mut self) -> Vec<PaletteEvent> {
        let next = if self.select_all {
            String::new()
        } else if !self.suffix.is_empty() {
            self.typed.clone()
        } else {
            let mut t = self.typed.clone();
            t.pop();
            t
        };
        if next == self.typed && self.suffix.is_empty() && !self.select_all {
            return Vec::new();
        }
        self.edited(&next)
    }

    /// A key the field handles itself. Returns `None` when the key is not the palette's (the
    /// field's own editing takes it: Right with no completion moves the caret).
    pub fn key(&mut self, key: PaletteKey) -> Option<Vec<PaletteEvent>> {
        let none = Vec::new;
        match key {
            PaletteKey::Up => self.move_highlight(-1),
            PaletteKey::Down => self.move_highlight(1),
            PaletteKey::Home => {
                if !self.rows.is_empty() {
                    self.highlight = Some(0);
                }
            }
            PaletteKey::End => {
                if !self.rows.is_empty() {
                    self.highlight = Some(self.rows.len() - 1);
                }
            }
            // Tab never leaves the field: it accepts a completion or does nothing.
            PaletteKey::Tab | PaletteKey::BackTab => return Some(self.accept_completion().unwrap_or_default()),
            PaletteKey::Right => return self.accept_completion(),
            PaletteKey::Return => return Some(self.commit_return()),
            PaletteKey::SecondaryReturn => return Some(vec![self.submit()]),
            PaletteKey::Escape => return Some(self.dismiss(false).into_iter().collect()),
        }
        Some(none())
    }

    /// A click on row `index`.
    pub fn click(&mut self, index: usize) -> Vec<PaletteEvent> {
        if index >= self.rows.len() {
            return Vec::new();
        }
        self.highlight = Some(index);
        vec![PaletteEvent::Activated(self.rows[index].id.clone())]
    }

    /// Sets the highlight directly (a native table moved it), clamped to the rows.
    pub fn set_highlight(&mut self, index: Option<usize>) {
        self.highlight = index.filter(|i| *i < self.rows.len());
    }

    /// The typed text, never an unaccepted completion.
    pub fn submit(&self) -> PaletteEvent {
        let text = if self.presented { self.typed.clone() } else { self.controlled_query.to_string() };
        PaletteEvent::Submitted(text.into())
    }

    fn move_highlight(&mut self, delta: isize) {
        if self.rows.is_empty() {
            return;
        }
        let last = self.rows.len() as isize - 1;
        self.highlight = Some(match self.highlight {
            None => 0,
            Some(cur) => (cur as isize + delta).clamp(0, last) as usize,
        });
    }

    /// Draws the first row's completion after the typed text, or takes a stale one away.
    fn apply_completion(&mut self) {
        let mut next = String::new();
        if self.may_complete
            && let Some(full) = self.rows.first().and_then(|r| r.completion.as_ref())
        {
            let typed_len = self.typed.chars().count();
            let full_len = full.chars().count();
            if typed_len > 0 && full_len > typed_len {
                let head: String = full.chars().take(typed_len).collect();
                if head.to_lowercase() == self.typed.to_lowercase() {
                    next = full.chars().skip(typed_len).collect();
                }
            }
        }
        self.suffix = next;
    }

    /// Tab or Right over a completion: it becomes typed text and reaches the app.
    fn accept_completion(&mut self) -> Option<Vec<PaletteEvent>> {
        if self.suffix.is_empty() {
            return None;
        }
        self.typed.push_str(&self.suffix);
        self.suffix.clear();
        self.may_complete = false;
        self.select_all = false;
        self.native_query = Some(self.typed.clone().into());
        Some(vec![PaletteEvent::QueryChanged(self.typed.clone().into())])
    }

    /// A first row carrying a completion stands for the completed text. Once Backspace has taken
    /// the completion away, Return means what was typed.
    fn commit_return(&mut self) -> Vec<PaletteEvent> {
        if self.highlight == Some(0)
            && self.suffix.is_empty()
            && let Some(full) = self.rows.first().and_then(|r| r.completion.as_ref())
            && !full.is_empty()
            && full.to_lowercase() != self.typed.to_lowercase()
        {
            return vec![self.submit()];
        }
        match self.highlight.and_then(|i| self.rows.get(i)) {
            Some(row) => vec![PaletteEvent::Activated(row.id.clone())],
            None => vec![self.submit()],
        }
    }
}

/// Sends a model event to the app as the matching typed event.
pub fn emit_palette_event(emit: &nd_ui::events::Emit, event: PaletteEvent) {
    match event {
        PaletteEvent::QueryChanged(text) => emit.emit(PaletteQueryChanged { text }),
        PaletteEvent::Activated(id) => emit.emit(PaletteActivated { id }),
        PaletteEvent::Submitted(text) => emit.emit(PaletteSubmitted { text }),
        PaletteEvent::Cancelled => emit.emit(PaletteCancelled),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(titles: &[&str]) -> Vec<PaletteItem> {
        titles.iter().map(|t| palette_item(*t, *t)).collect()
    }

    fn open(items: Vec<PaletteItem>, query: &str) -> PaletteModel {
        let mut m = PaletteModel::default();
        m.set_query(&query.into());
        m.set_items(&items);
        m.present();
        m
    }

    #[test]
    fn present_starts_from_the_last_query_with_everything_selected() {
        let mut m = open(rows(&["a", "b"]), "hello");
        assert_eq!(m.field_text(), "hello");
        assert_eq!(m.selection(), FieldSelection::All);
        assert_eq!(m.highlight(), Some(0));
        m.edited("help");
        m.dismiss(true);
        m.present();
        assert_eq!(m.field_text(), "hello", "a present never starts from an earlier present's text");
    }

    #[test]
    fn arrows_home_and_end_clamp_the_highlight() {
        let mut m = open(rows(&["a", "b", "c"]), "");
        m.key(PaletteKey::Up);
        assert_eq!(m.highlight(), Some(0));
        m.key(PaletteKey::Down);
        m.key(PaletteKey::Down);
        m.key(PaletteKey::Down);
        assert_eq!(m.highlight(), Some(2));
        m.key(PaletteKey::Home);
        assert_eq!(m.highlight(), Some(0));
        m.key(PaletteKey::End);
        assert_eq!(m.highlight(), Some(2));
    }

    #[test]
    fn fresh_results_highlight_the_top_row_and_equal_results_keep_it() {
        let mut m = open(rows(&["a", "b", "c"]), "");
        m.key(PaletteKey::Down);
        assert!(!m.set_items(&rows(&["a", "b", "c"])), "an equal vector is not a change");
        assert_eq!(m.highlight(), Some(1));
        assert!(m.set_items(&rows(&["x", "y"])));
        assert_eq!(m.highlight(), Some(0));
        m.set_items(&[]);
        assert_eq!(m.highlight(), None);
    }

    #[test]
    fn return_activates_the_highlighted_row_or_submits_the_text() {
        let mut m = open(rows(&["a", "b"]), "");
        m.key(PaletteKey::Down);
        assert_eq!(m.key(PaletteKey::Return), Some(vec![PaletteEvent::Activated("b".into())]));
        let mut m = open(Vec::new(), "");
        m.edited("/tmp/new");
        assert_eq!(m.key(PaletteKey::Return), Some(vec![PaletteEvent::Submitted("/tmp/new".into())]));
        let mut m = open(rows(&["a"]), "");
        m.edited("zz");
        assert_eq!(
            m.key(PaletteKey::SecondaryReturn),
            Some(vec![PaletteEvent::Submitted("zz".into())]),
            "secondary-Return submits even with a row highlighted"
        );
    }

    #[test]
    fn escape_cancels_and_an_app_close_does_not_echo() {
        let mut m = open(rows(&["a"]), "");
        assert_eq!(m.key(PaletteKey::Escape), Some(vec![PaletteEvent::Cancelled]));
        assert!(!m.is_presented());
        let mut m = open(rows(&["a"]), "");
        assert_eq!(m.dismiss(true), None);
        assert_eq!(m.dismiss(false), None, "closing a closed palette reports nothing");
    }

    fn completing(full: &str) -> Vec<PaletteItem> {
        vec![palette_item("n1", full).completion(full)]
    }

    #[test]
    fn typing_shows_the_first_rows_completion_selected() {
        let mut m = open(Vec::new(), "");
        assert_eq!(m.edited("tri"), vec![PaletteEvent::QueryChanged("tri".into())]);
        m.set_items(&completing("Trip to Lisbon"));
        assert_eq!(m.field_text(), "trip to Lisbon");
        assert_eq!(m.completion(), "p to Lisbon");
        assert_eq!(m.selection(), FieldSelection::Completion);
    }

    #[test]
    fn tab_and_right_accept_and_only_then_the_app_sees_the_completion() {
        let mut m = open(completing("Trip to Lisbon"), "");
        let typed = m.edited("tri");
        assert_eq!(
            typed,
            vec![PaletteEvent::QueryChanged("tri".into())],
            "an unaccepted completion never reaches the app"
        );
        assert_eq!(m.key(PaletteKey::Tab), Some(vec![PaletteEvent::QueryChanged("trip to Lisbon".into())]));
        assert_eq!(m.completion(), "");
        assert_eq!(m.key(PaletteKey::Tab), Some(Vec::new()), "Tab with nothing to accept stays in the field");
        assert_eq!(m.key(PaletteKey::Right), None, "Right with no completion moves the caret");
        let mut m = open(completing("Trip to Lisbon"), "");
        m.edited("t");
        assert_eq!(m.key(PaletteKey::Right), Some(vec![PaletteEvent::QueryChanged("trip to Lisbon".into())]));
    }

    #[test]
    fn backspace_over_a_completion_does_not_bring_it_back() {
        let mut m = open(completing("Trip to Lisbon"), "");
        m.edited("tri");
        assert_eq!(m.completion(), "p to Lisbon");
        // The field removed the selected completion: the text is what was typed.
        m.edited("tri");
        assert_eq!(m.completion(), "", "a removing edit does not complete");
        m.set_items(&completing("Trip to Lisbon"));
        assert_eq!(m.completion(), "", "fresh rows do not bring it back either");
        assert_eq!(
            m.key(PaletteKey::Return),
            Some(vec![PaletteEvent::Submitted("tri".into())]),
            "Return on a completing first row whose completion was removed submits the typed text"
        );
        m.edited("trip");
        assert_eq!(m.completion(), " to Lisbon", "typing again completes again");
    }

    #[test]
    fn return_on_a_completing_first_row_activates_it() {
        let mut m = open(completing("Trip to Lisbon"), "");
        m.edited("tri");
        assert_eq!(m.key(PaletteKey::Return), Some(vec![PaletteEvent::Activated("n1".into())]));
    }

    #[test]
    fn completion_matches_case_insensitively_and_only_extensions() {
        let mut m = open(completing("Trip"), "");
        m.edited("TRI");
        assert_eq!(m.field_text(), "TRIp");
        m.edited("TRIP");
        assert_eq!(m.completion(), "", "a completion that adds nothing is not shown");
        m.edited("TRIPS");
        assert_eq!(m.completion(), "");
        let mut m = open(completing("Lisbon"), "");
        m.edited("tri");
        assert_eq!(m.completion(), "", "a completion must extend what was typed");
    }

    #[test]
    fn a_reported_query_fed_back_resets_nothing() {
        let mut m = open(completing("Trip to Lisbon"), "");
        m.edited("tri");
        assert!(!m.set_query(&"tri".into()), "the app echoing the typed text is no reseed");
        assert_eq!(m.completion(), "p to Lisbon");
        assert!(m.set_query(&"other".into()), "a different query reseeds the open field");
        assert_eq!(m.field_text(), "other");
        assert_eq!(m.selection(), FieldSelection::All);
    }

    #[test]
    fn drawn_editing_replaces_the_selection() {
        let mut m = open(Vec::new(), "seed");
        m.insert("x");
        assert_eq!(m.typed(), "x", "typing over a selected seed replaces it");
        m.insert("y");
        assert_eq!(m.typed(), "xy");
        m.backspace();
        assert_eq!(m.typed(), "x");
        m.set_items(&completing("xylophone"));
        m.insert("y");
        assert_eq!(m.completion(), "lophone");
        m.backspace();
        assert_eq!((m.typed(), m.completion()), ("xy", ""), "Backspace takes the completion first");
    }

    #[test]
    fn clicks_activate_and_out_of_range_rows_do_nothing() {
        let mut m = open(rows(&["a", "b"]), "");
        assert_eq!(m.click(1), vec![PaletteEvent::Activated("b".into())]);
        assert_eq!(m.highlight(), Some(1));
        assert!(m.click(9).is_empty());
    }
}
