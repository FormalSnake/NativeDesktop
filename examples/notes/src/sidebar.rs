use nd::prelude::*;

use crate::model::{Folder, Library};

/// The folder list: a data-driven source list, so macOS draws its own sidebar and the GPUI
/// backends the matching drawn one. Counts are badges.
pub fn folder_sidebar(
    library: &Library,
    selected: Folder,
    on_select: impl Fn(Folder, &mut Window, &mut App) + 'static,
) -> SourceTree {
    let items = Folder::ALL.map(|folder| {
        let item = source_item(folder.id(), folder.label()).icon(folder.icon());
        match library.count(folder) {
            0 => item,
            count => item.badge(count.to_string()),
        }
    });
    source_list("folders").section("Library", items).selected(selected.id()).on_select(
        move |e: &SourceSelected, window, cx| {
            if let Some(folder) = Folder::from_id(&e.id) {
                on_select(folder, window, cx);
            }
        },
    )
}
