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
        let mut folders = ListState::new(cx);
        folders.set_items(Folder::ALL.to_vec(), cx);
        // Counts change when notes do; the rows re-render, the list keeps its state.
        cx.observe(&library, |_, _, cx| cx.notify()).detach();
        Self { library, folders, selected: Folder::All }
    }

    fn row(&mut self, folder: &Folder, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let count = self.library.read(cx).count(*folder);
        source_row(folder.icon(), folder.label())
            .when(count > 0, |row| row.badge(count.to_string()))
            .test_id(format!("folder-{}", folder.label()))
    }
}

impl Render for FolderSidebar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        source_list(&self.folders, cx.processor(Self::row))
            .section("Library")
            .selected(self.selected)
            .on_select(cx.listener(|this, e: &SelectEvent<Folder>, _window, cx| {
                this.selected = e.key;
                cx.emit(FolderSelected(e.key));
                cx.notify();
            }))
    }
}
