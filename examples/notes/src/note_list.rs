use nd::prelude::*;

use crate::actions::{DeleteNote, DuplicateNote, NewNote, PinNote};
use crate::model::{Folder, Library, NoteId};
use crate::settings::NotesSettings;

pub struct NoteSelected(pub NoteId);

pub struct NoteList {
    library: Entity<Library>,
    list: ListState<NoteId>,
    folder: Folder,
    query: SharedString,
}

impl EventEmitter<NoteSelected> for NoteList {}

impl NoteList {
    pub fn new(library: Entity<Library>, cx: &mut Context<Self>) -> Self {
        let list = ListState::new(cx).follow(Follow::None);
        cx.observe(&library, |this, _, cx| this.refresh(cx)).detach();
        let mut this = Self { library, list, folder: Folder::All, query: SharedString::default() };
        this.refresh(cx);
        this
    }

    pub fn set_filter(&mut self, folder: Folder, query: SharedString, cx: &mut Context<Self>) {
        self.folder = folder;
        self.query = query;
        self.list.scroll_to_top(cx);
        self.refresh(cx);
    }

    pub fn select(&mut self, id: NoteId, cx: &mut Context<Self>) {
        self.list.select(id, cx);
        self.list.scroll_to(id, Align::Nearest, px(0.), cx);
        cx.emit(NoteSelected(id));
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let rows = self.library.read(cx).rows(self.folder, &self.query);
        // Keyed diff: inserts and removals animate and the anchor row stays put. Versioned: only
        // rows whose note changed re-render.
        self.list.set_items_versioned(rows, cx);
    }

    fn row(&mut self, id: NoteId, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let compact = cx.settings::<NotesSettings>().compact_rows;
        let Some(note) = self.library.read(cx).note(&id) else {
            return div().into_any_element();
        };
        let tag = note.tag.map(|t| t.color(theme));
        let pinned = note.pinned;
        let accent = theme.accent;

        h_flex()
            .test_id(format!("note-{}", id.0))
            .gap_3()
            .px_3()
            .py(if compact { px(6.) } else { px(10.) })
            .items_start()
            // Custom-drawn gutter: tag dot, plus a pin notch for pinned notes.
            .child(
                canvas(move |bounds, paint, _cx| {
                    if let Some(color) = tag {
                        paint.fill_circle(point(bounds.center().x, bounds.top() + px(7.)), px(4.), color);
                    }
                    if pinned {
                        let notch = Bounds::new(
                            point(bounds.left(), bounds.bottom() - px(6.)),
                            size(bounds.size.width, px(2.)),
                        );
                        paint.fill_rounded_rect(notch, px(1.), accent);
                    }
                })
                .cache_key((tag, pinned, accent))
                .w(px(10.))
                .h_full(),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_0p5()
                    .child(
                        h_flex()
                            .gap_2()
                            .child(text(note.title.clone()).font_weight(FontWeight::SEMIBOLD).truncate().flex_1())
                            .child(
                                // A self-updating relative time: re-renders on its own minute tick,
                                // never through the row.
                                relative_time(note.updated)
                                    .text_size(theme.type_scale.caption)
                                    .text_color(theme.text.tertiary),
                            ),
                    )
                    .when(!compact, |col| {
                        col.child(text(note.preview()).text_color(theme.text.secondary).line_clamp(2))
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

impl Render for NoteList {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let compact = cx.settings::<NotesSettings>().compact_rows;
        let accent = cx.theme().accent;
        let pinned =
            |library: &Entity<Library>, id: NoteId, cx: &App| library.read(cx).note(&id).is_some_and(|n| n.pinned);
        let library = self.library.clone();

        list("notes", &self.list, cx.processor(Self::row))
            .test_id("notes")
            .estimated_row_height(if compact { px(36.) } else { px(64.) })
            .selection(Selection::single())
            .type_ahead(true)
            // Row actions belong to the list: on macOS they are NSTableView row actions.
            .row_actions(move |id: NoteId, cx: &App| {
                let pin_label = if pinned(&library, id, cx) { "Unpin" } else { "Pin" };
                RowActions::new()
                    .trailing(row_action("Delete", Icon::Trash).destructive().dispatch(DeleteNote(id)))
                    .leading(row_action(pin_label, Icon::Pin).tint(accent).dispatch(PinNote(id)))
                    .full_swipe(RowEdge::Trailing)
            })
            .on_select(cx.listener(|_, e: &SelectEvent<NoteId>, _window, cx| cx.emit(NoteSelected(e.key))))
            .empty(status_page(Icon::Notebook, "No Notes").description(match window.binding_label(&NewNote) {
                Some(keys) => format!("Press {keys} to write one."),
                None => "Use the New Note button to write one.".to_owned(),
            }))
    }
}
