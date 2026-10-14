use nd::prelude::*;

use crate::model::{Folder, Library};
use crate::note_list::NoteList;

pub struct FolderSidebar {
    library: Entity<Library>,
    notes: Entity<NoteList>,
    selected: Folder,
}

impl FolderSidebar {
    pub fn new(library: Entity<Library>, notes: Entity<NoteList>, cx: &mut Context<Self>) -> Self {
        // Counts change on create, delete and pin.
        cx.observe(&library, |_, _, cx| cx.notify()).detach();
        Self { library, notes, selected: Folder::All }
    }
}

impl Render for FolderSidebar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let library = self.library.read(cx);
        let items: Vec<SourceItem> = Folder::ALL
            .into_iter()
            .map(|folder| {
                let item = source_item(folder.id(), folder.label()).icon(folder.icon());
                match library.count(folder) {
                    0 => item,
                    count => item.badge(count.to_string()),
                }
            })
            .collect();
        source_list("folders").section("Library", items).selected(self.selected.id()).on_select(cx.listener(
            |this, e: &SourceSelected, _window, cx| {
                let Some(folder) = Folder::by_id(&e.id) else { return };
                this.selected = folder;
                this.notes.update(cx, |notes, cx| notes.set_folder(folder, cx));
                cx.notify();
            },
        ))
    }
}
