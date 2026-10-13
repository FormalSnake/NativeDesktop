use nd::prelude::*;

use crate::model::{Folder, Library};

pub struct FolderSelected(pub Folder);

pub struct FolderSidebar {
    library: Entity<Library>,
    folders: ListState<Folder>,
    selected: Folder,
}

impl EventEmitter<FolderSelected> for FolderSidebar {}

impl FolderSidebar {
    pub fn new(library: Entity<Library>, cx: &mut Context<Self>) -> Self {
        let folders = ListState::new(cx);
        // The count is the row version: a row re-renders only when its count changes.
        cx.observe(&library, |this, library, cx| {
            let rows = Folder::ALL.map(|f| (f, library.read(cx).count(f) as u64));
            this.folders.set_items_versioned(rows, cx);
        })
        .detach();
        let mut this = Self { library, folders, selected: Folder::All };
        let rows = Folder::ALL.map(|f| (f, this.library.read(cx).count(f) as u64));
        this.folders.set_items_versioned(rows, cx);
        this
    }

    fn row(&mut self, folder: Folder, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let count = self.library.read(cx).count(folder);
        source_row(folder.icon(), folder.label())
            .when(count > 0, |row| row.badge(count.to_string()))
            .test_id(format!("folder-{}", folder.label()))
    }
}

impl Render for FolderSidebar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        source_list("folders", &self.folders, cx.processor(Self::row))
            .section("Library")
            .selected(self.selected)
            .on_select(cx.listener(|this, e: &SelectEvent<Folder>, _window, cx| {
                this.selected = e.key;
                cx.emit(FolderSelected(e.key));
                cx.notify();
            }))
    }
}
