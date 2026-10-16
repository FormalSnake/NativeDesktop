use std::rc::Rc;

use nd::prelude::*;

use crate::actions::{DeleteNote, DuplicateNote, NewNote, PinNote, shortcut};
use crate::demo;
use crate::model::{Folder, Library, NoteId};
use crate::settings::NotesSettings;

/// Told when the user picks a row.
type OnOpen = Rc<dyn Fn(NoteId, &mut Window, &mut App)>;

pub struct NoteList {
    library: Entity<Library>,
    list: ListState<NoteId>,
    folder: Folder,
    query: SharedString,
    on_open: OnOpen,
}

impl NoteList {
    pub fn new(
        library: Entity<Library>,
        on_open: impl Fn(NoteId, &mut Window, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) -> Self {
        let list = ListState::new(cx).follow(Follow::None);
        cx.observe(&library, |this, _, cx| this.refresh(cx)).detach();
        let mut this =
            Self { library, list, folder: Folder::All, query: SharedString::default(), on_open: Rc::new(on_open) };
        this.refresh(cx);
        this
    }

    pub fn list(&self) -> &ListState<NoteId> {
        &self.list
    }

    pub fn set_filter(&mut self, folder: Folder, query: SharedString, cx: &mut Context<Self>) {
        self.folder = folder;
        self.query = query;
        self.list.scroll_to_top(cx);
        self.refresh(cx);
    }

    // docs:begin select
    pub fn select(&mut self, id: NoteId, cx: &mut Context<Self>) {
        self.list.select(id, cx);
        self.list.scroll_to(id, Align::Nearest, px(0.), cx);
    }
    // docs:end select

    // docs:begin refresh
    fn refresh(&mut self, cx: &mut Context<Self>) {
        let rows = self.library.read(cx).rows(self.folder, &self.query);
        // Keyed diff: inserts and removals animate and the anchor row stays put. Versioned: only
        // rows whose note changed re-render.
        self.list.set_items_versioned(rows, cx);
    }
    // docs:end refresh

    fn row(&mut self, id: NoteId, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let compact = cx.settings::<NotesSettings>().compact_rows;
        let Some(note) = self.library.read(cx).note(&id).cloned() else {
            return div().into_any_element();
        };
        let pinned = note.pinned;
        let now = if demo::enabled() || cx.try_global::<crate::model::Seed>().is_some() {
            demo::now()
        } else {
            jiff::Timestamp::now()
        };

        h_flex()
            .id(format!("note-{}", id.0))
            .gap_3()
            .px_3()
            .py(if compact { px(6.) } else { px(10.) })
            .items_start()
            // Gutter: a tag dot, and a pin notch under it for pinned notes.
            .child(
                v_flex()
                    .w(px(10.))
                    .h_full()
                    .items_center()
                    .gap_1()
                    .pt(px(4.))
                    .child(match note.tag {
                        Some(tag) => div().size(px(8.)).rounded_full().bg(tag.color(theme)),
                        None => div().size(px(8.)),
                    })
                    .when(pinned, |gutter| gutter.child(div().w(px(10.)).h(px(2.)).rounded_full().bg(theme.accent))),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_0p5()
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(text(note.title.clone()).font_weight(FontWeight::SEMIBOLD).truncate()),
                            )
                            .child(
                                text(relative_time(note.updated, now))
                                    .text_size(theme.type_scale.caption)
                                    .text_color(theme.text.secondary),
                            ),
                    )
                    .when(!compact, |col| {
                        col.child(text(note.preview.clone()).text_color(theme.text.secondary).line_clamp(2))
                    }),
            )
            .context_menu(move |menu, _window, _cx| {
                menu.item(if pinned { "Unpin" } else { "Pin" }, PinNote(id))
                    .item("Duplicate", DuplicateNote(id))
                    .separator()
                    .item_destructive("Delete", DeleteNote(id))
            })
            .into_any_element()
    }
}

/// "now", "5m", "3h", "2d", else the date: short enough for the row's corner.
fn relative_time(then: jiff::Timestamp, now: jiff::Timestamp) -> String {
    let minutes = now.duration_since(then).as_secs() / 60;
    match minutes {
        ..1 => "now".to_owned(),
        1..60 => format!("{minutes}m"),
        60..1440 => format!("{}h", minutes / 60),
        1440..10_080 => format!("{}d", minutes / 1440),
        _ => then.to_zoned(jiff::tz::TimeZone::UTC).strftime("%b %-d").to_string(),
    }
}

impl Render for NoteList {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let compact = cx.settings::<NotesSettings>().compact_rows;
        let accent = cx.theme().accent;
        let library = self.library.clone();
        let on_open = self.on_open.clone();

        // docs:begin list
        list("notes", &self.list, cx.processor(Self::row))
            .estimated_row_height(if compact { px(36.) } else { px(64.) })
            .selection(Selection::single())
            // Row actions belong to the list: on macOS they are NSTableView row actions.
            .row_actions(move |id: NoteId, cx: &App| {
                let pinned = library.read(cx).note(&id).is_some_and(|n| n.pinned);
                RowActions::new()
                    .trailing(row_action("Delete").icon(Icon::Trash.lucide()).destructive().dispatch(DeleteNote(id)))
                    .leading(
                        row_action(if pinned { "Unpin" } else { "Pin" })
                            .icon(Icon::Pin.lucide())
                            .tint(accent)
                            .dispatch(PinNote(id)),
                    )
                    .full_swipe(RowEdge::Trailing)
            })
            .on_select(move |e: &SelectEvent<NoteId>, window, cx| on_open(e.key, window, cx))
            .empty(status_page("No Notes").icon(Icon::Notebook).description(match shortcut(&NewNote) {
                Some(keys) => format!("Press {keys} to write one."),
                None => "Use the New Note button to write one.".to_owned(),
            }))
        // docs:end list
    }
}
