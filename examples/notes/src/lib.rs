pub mod actions;
pub mod app;
pub mod demo;
pub mod editor;
pub mod model;
pub mod note_list;
pub mod settings;
pub mod sidebar;

use nd::prelude::*;

pub fn app() -> Application {
    Application::new(nd::manifest!())
}

pub fn window(cx: &mut App) -> WindowHandle<app::Notes> {
    cx.open_window(WindowOptions::main("main").title("Notes").min_size(size(px(720.), px(440.))), |window, cx| {
        cx.new(|cx| app::Notes::new(window, cx))
    })
}

pub fn run() -> nd::Result<()> {
    app().settings::<settings::NotesSettings>().run(|cx| {
        actions::init(cx);
        if demo::enabled() {
            demo::seed(cx);
        }
        window(cx);
    })
}
