#![cfg(feature = "m6")]

pub mod actions;
pub mod app;
pub mod demo;
pub mod editor;
pub mod model;
pub mod note_list;
pub mod settings;
pub mod sidebar;

use nd::prelude::*;
use nd_settings::Store;

use crate::model::{Library, LibraryData};

pub fn run() -> nd::Result<()> {
    Application::new(nd::manifest!()).run(|cx| {
        actions::init(cx);
        settings::load(cx, nd_settings::Options::new("notes"));
        let dirs = nd::system::app_dirs();
        let dir = dirs.ensure_data().unwrap_or_else(|_| dirs.data());
        // Demo mode keeps its own file, so screenshots never touch the user's notes.
        let store = if demo::enabled() {
            let store = Store::open(dir.join("library-demo.json"), LibraryData::VERSION, None);
            store.update(|data| *data = demo::library());
            store
        } else {
            Store::open(dir.join("library.json"), LibraryData::VERSION, None)
        };
        let library = Library::open(store, cx);
        cx.open_window(WindowOptions::main("main").title("Notes").min_size(size(px(720.), px(440.))), |window, cx| {
            cx.new(|cx| app::Notes::new(library, window, cx))
        })
    })
}
