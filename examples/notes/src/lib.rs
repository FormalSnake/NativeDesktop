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

pub fn run() -> nd::Result<()> {
    Application::new(nd::manifest!())
        .settings::<settings::NotesSettings>()
        .single_instance(true)
        .run(|cx| {
            actions::init(cx);
            if demo::enabled() {
                demo::seed(cx);
            }
            cx.open_window(
                WindowOptions::main("main").title("Notes").min_size(size(px(720.), px(440.))),
                |window, cx| cx.new(|cx| app::Notes::new(window, cx)),
            )
        })
}
