use nd::prelude::*;

use crate::actions::*;
use crate::editor::Editor;
use crate::model::{Folder, Library, NoteId};
use crate::note_list::NoteList;
use crate::settings::{self, NewNoteFolder, SettingsSheet};
use crate::sidebar::FolderSidebar;

pub struct Notes {
    library: Entity<Library>,
    sidebar: Entity<FolderSidebar>,
    notes: Entity<NoteList>,
    editor: Entity<Editor>,
    search: Entity<TextFieldState>,
    sidebar_collapsed: bool,
    palette_open: bool,
    palette_query: SharedString,
}

impl Notes {
    pub fn new(library: Entity<Library>, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let editor = cx.new(|cx| Editor::new(library.clone(), cx));
        let notes = cx.new(|cx| NoteList::new(library.clone(), editor.clone(), cx));
        let sidebar = cx.new(|cx| FolderSidebar::new(library.clone(), notes.clone(), cx));
        let search = cx.new(|_| TextFieldState::default().placeholder("Search"));

        cx.observe(&search, |this, search, cx| {
            let query = search.read(cx).text().clone();
            this.notes.update(cx, |notes, cx| notes.set_query(query, cx));
        })
        .detach();

        Self {
            library,
            sidebar,
            notes,
            editor,
            search,
            sidebar_collapsed: false,
            palette_open: false,
            palette_query: SharedString::default(),
        }
    }

    pub fn notes(&self) -> &Entity<NoteList> {
        &self.notes
    }

    pub fn editor(&self) -> &Entity<Editor> {
        &self.editor
    }

    pub fn library(&self) -> &Entity<Library> {
        &self.library
    }

    pub fn palette_open(&self) -> bool {
        self.palette_open
    }

    fn new_note(&mut self, _: &NewNote, _window: &mut Window, cx: &mut Context<Self>) {
        let pin = settings::current(cx).new_note_folder == NewNoteFolder::Pinned;
        let id = self.library.update(cx, |library, cx| {
            let id = library.create(cx);
            if pin {
                library.toggle_pin(id, cx);
            }
            id
        });
        self.select(id, cx);
    }

    fn select(&mut self, id: NoteId, cx: &mut Context<Self>) {
        self.notes.update(cx, |notes, cx| notes.select(id, cx));
    }

    fn delete(&mut self, DeleteNote(id): &DeleteNote, _window: &mut Window, cx: &mut Context<Self>) {
        self.library.update(cx, |library, cx| library.delete(*id, cx));
    }

    fn pin(&mut self, PinNote(id): &PinNote, _window: &mut Window, cx: &mut Context<Self>) {
        self.library.update(cx, |library, cx| library.toggle_pin(*id, cx));
    }

    fn duplicate(&mut self, DuplicateNote(id): &DuplicateNote, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(copy) = self.library.update(cx, |library, cx| library.duplicate(*id, cx)) {
            self.select(copy, cx);
        }
    }

    // docs:begin sheet
    fn open_settings(&mut self, _: &OpenSettings, window: &mut Window, cx: &mut Context<Self>) {
        window.open_sheet(cx, |window, cx| cx.new(|cx| SettingsSheet::new(window, cx)));
    }
    // docs:end sheet

    fn toggle_sidebar(&mut self, _: &ToggleSidebar, _window: &mut Window, cx: &mut Context<Self>) {
        self.sidebar_collapsed = !self.sidebar_collapsed;
        cx.notify();
    }

    fn open_palette(&mut self, _: &OpenPalette, _window: &mut Window, cx: &mut Context<Self>) {
        self.palette_open = true;
        self.palette_query = SharedString::default();
        cx.notify();
    }

    /// The palette is controlled: it shows exactly these rows, in this order. Notes come first, so a
    /// title that starts with the query offers itself as the inline completion.
    pub fn palette_items(&self, cx: &App) -> Vec<PaletteItem> {
        let query = self.palette_query.to_lowercase();
        let library = self.library.read(cx);
        let notes: Vec<_> = library.rows(Folder::All, "").into_iter().filter_map(|(id, _)| library.note(&id)).collect();

        let mut items: Vec<PaletteItem> = fuzzy::rank(&self.palette_query, &notes, |n| n.title.as_ref())
            .take(20)
            .map(|note| {
                let completes = !query.is_empty() && note.title.to_lowercase().starts_with(&query);
                palette_item(format!("note:{}", note.id.0), note.title.clone())
                    .subtitle(note.preview())
                    .icon(Icon::FileText)
                    .when(completes, |item| item.completion(note.title.clone()))
            })
            .collect();

        let commands = [
            ("cmd:new", "New Note", Icon::SquarePen),
            ("cmd:settings", "Settings", Icon::Settings),
            ("cmd:sidebar", "Toggle Sidebar", Icon::PanelLeft),
        ];
        items.extend(
            fuzzy::rank(&self.palette_query, commands, |c| c.1)
                .map(|(id, title, icon)| palette_item(id, title).icon(icon)),
        );
        items
    }

    pub fn palette_activate(&mut self, event: &PaletteActivated, window: &mut Window, cx: &mut Context<Self>) {
        self.palette_open = false;
        match event.id.as_ref() {
            "cmd:new" => self.new_note(&NewNote, window, cx),
            "cmd:settings" => self.open_settings(&OpenSettings, window, cx),
            "cmd:sidebar" => self.toggle_sidebar(&ToggleSidebar, window, cx),
            other => {
                if let Some(id) = other.strip_prefix("note:").and_then(|n| n.parse().ok()) {
                    self.select(NoteId(id), cx);
                }
            }
        }
        cx.notify();
    }
}

impl Render for Notes {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette_items = self.palette_items(cx);

        scaffold()
            .key_context(CONTEXT)
            .on_action(cx.listener(Self::new_note))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::pin))
            .on_action(cx.listener(Self::duplicate))
            .on_action(cx.listener(Self::open_settings))
            .on_action(cx.listener(Self::toggle_sidebar))
            .on_action(cx.listener(Self::open_palette))
            .on_action(|_: &ReportIssue, _, _| report_issue())
            .toolbar(
                toolbar()
                    .sidebar_toggle()
                    .item(icon_button("new-note", Icon::SquarePen).tooltip("New Note").dispatch(NewNote))
                    .flexible_space()
                    .item(search_field("search", &self.search).label("Search").w(px(220.))),
            )
            .sidebar(sidebar().collapsed(self.sidebar_collapsed).min_w(px(200.)).child(self.sidebar.clone()))
            .content(
                h_flex()
                    .size_full()
                    .child(div().w(px(300.)).h_full().child(self.notes.clone()))
                    .child(separator().vertical())
                    .child(div().flex_1().h_full().child(self.editor.clone())),
            )
            // docs:begin palette
            .overlay(
                command_palette("palette")
                    .open(self.palette_open)
                    .placeholder("Search notes and commands")
                    .query(self.palette_query.clone())
                    .items(palette_items)
                    .on_query_change(cx.listener(|this, e: &PaletteQueryChanged, _window, cx| {
                        this.palette_query = e.text.clone();
                        cx.notify();
                    }))
                    .on_activate(cx.listener(Self::palette_activate))
                    .on_submit(cx.listener(|this, e: &PaletteSubmitted, _window, cx| {
                        // Nothing matched: typed text becomes a new note.
                        this.palette_open = false;
                        cx.notify();
                        let id = this.library.update(cx, |library, cx| {
                            let id = library.create(cx);
                            library.set_body(id, e.text.to_string(), cx);
                            id
                        });
                        this.select(id, cx);
                    }))
                    .on_cancel(cx.listener(|this, _: &PaletteCancelled, _window, cx| {
                        this.palette_open = false;
                        cx.notify();
                    })),
            )
        // docs:end palette
    }
}
