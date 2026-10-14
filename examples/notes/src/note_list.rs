use nd::prelude::*;
use nd::virtual_list::{RowActions, RowEdge, row_action};

use crate::actions::{DeleteNote, DuplicateNote, PinNote};
use crate::editor::Editor;
use crate::model::{Folder, Library, NoteId};
use crate::settings;

pub struct NoteList {
    library: Entity<Library>,
    editor: Entity<Editor>,
    list: ListState<NoteId>,
    folder: Folder,
    query: SharedString,
}

impl NoteList {
    pub fn new(library: Entity<Library>, editor: Entity<Editor>, cx: &mut Context<Self>) -> Self {
        // docs:begin state
        let list = ListState::new(cx).follow(Follow::None);
        // docs:end state
        cx.observe(&library, |this, _, cx| this.refresh(cx)).detach();
        let mut this = Self { library, editor, list, folder: Folder::All, query: SharedString::default() };
        this.refresh(cx);
        this
    }

    pub fn set_folder(&mut self, folder: Folder, cx: &mut Context<Self>) {
        self.folder = folder;
        self.list.scroll_to_top(cx);
        self.refresh(cx);
    }

    pub fn set_query(&mut self, query: SharedString, cx: &mut Context<Self>) {
        self.query = query;
        self.list.scroll_to_top(cx);
        self.refresh(cx);
    }

    pub fn list(&self) -> &ListState<NoteId> {
        &self.list
    }

    // docs:begin select
    pub fn select(&mut self, id: NoteId, cx: &mut Context<Self>) {
        // A note created this turn reaches the rows through the library observer only after it.
        self.refresh(cx);
        self.list.select(id, cx);
        self.list.scroll_to(id, Align::Nearest, px(0.), cx);
        self.editor.update(cx, |editor, cx| editor.open(id, cx));
    }
    // docs:end select

    // docs:begin refresh
    fn refresh(&mut self, cx: &mut Context<Self>) {
        let rows = self.library.read(cx).rows(self.folder, &self.query);
        // Keyed diff: inserts and removals animate and the anchor row stays put. Versioned: only
        // rows whose note changed re-render.
        self.list.set_items_versioned(rows, cx);
        cx.notify();
    }
    // docs:end refresh

    fn row(&mut self, id: NoteId, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let compact = settings::current(cx).compact_rows;
        let Some(note) = self.library.read(cx).note(&id) else {
            return div().into_any_element();
        };
        let pinned = note.pinned;
        let gutter =
            div().w(px(10.)).pt_1().flex().justify_center().when_some(note.tag, |gutter, tag| {
                gutter.child(div().size(px(8.)).rounded_full().bg(tag.color(theme)))
            });

        h_flex()
            .id(format!("note-{}", id.0))
            .gap_3()
            .px_3()
            .py(if compact { px(6.) } else { px(10.) })
            .items_start()
            .child(gutter)
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_0p5()
                    .child(
                        h_flex()
                            .gap_2()
                            .child(text(note.title.clone()).font_weight(FontWeight::SEMIBOLD).truncate().flex_1())
                            .when(pinned, |row| row.child(text("Pinned").text_color(theme.accent)))
                            .child(
                                text(note.updated.strftime("%b %-d").to_string())
                                    .text_size(theme.type_scale.caption)
                                    .text_color(theme.text.tertiary),
                            ),
                    )
                    .when(!compact, |col| col.child(text(note.preview()).text_color(theme.text.secondary).truncate())),
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
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let compact = settings::current(cx).compact_rows;
        let accent = cx.theme().accent;
        let library = self.library.clone();

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
            .on_select(cx.listener(|this, e: &SelectEvent<NoteId>, _window, cx| {
                this.editor.update(cx, |editor, cx| editor.open(e.key, cx));
            }))
            // docs:end list
            .empty(
                status_page("No Notes")
                    .icon(Icon::Notebook)
                    .description("Use the New Note button or the command palette to write one."),
            )
    }
}
