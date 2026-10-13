//! Every built-in component on one page each, with live state, in the layout System Settings
//! uses. Pages live in their own modules and register in [`pages::all`].

pub mod forms;
pub mod pages;

use nd::prelude::*;
use pages::{Mount, Page};

pub struct Gallery {
    pages: Vec<(Page, Mount)>,
    current: SharedString,
}

impl Gallery {
    pub fn new(cx: &mut App) -> Self {
        let pages: Vec<(Page, Mount)> = pages::all()
            .into_iter()
            .map(|page| {
                let mount = (page.mount)(cx);
                (page, mount)
            })
            .collect();
        let current = pages.first().map(|(page, _)| page.id.into()).unwrap_or_default();
        Self { pages, current }
    }

    pub fn current(&self) -> &SharedString {
        &self.current
    }
}

impl Render for Gallery {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let shown = self.pages.iter().find(|(page, _)| page.id == self.current.as_ref());
        let title = shown.map_or("", |(page, _)| page.title);
        scaffold()
            .toolbar(toolbar().title(title).sidebar_toggle())
            .sidebar(
                sidebar().min_w(px(200.)).child(
                    source_list("pages")
                        .items(self.pages.iter().map(|(page, _)| source_item(page.id, page.title).icon(page.icon)))
                        .selected(self.current.clone())
                        .on_select(cx.listener(|this, e: &SourceSelected, _, cx| {
                            this.current = e.id.clone();
                            cx.notify();
                        })),
                ),
            )
            .content(div().id("page").size_full().children(shown.map(|(_, mount)| mount())))
    }
}

pub fn app() -> Application {
    Application::new(nd::manifest!())
}

pub fn open(cx: &mut App) -> WindowHandle<Gallery> {
    open_in(cx, None)
}

/// Opens the gallery, forced to `appearance` when given (goldens), else following the system.
pub fn open_in(cx: &mut App, appearance: Option<Appearance>) -> WindowHandle<Gallery> {
    cx.open_window(WindowOptions::main("main").title("Gallery").size(size(px(960.), px(720.))), move |window, cx| {
        if let Some(appearance) = appearance {
            window.set_theme(ThemeOverride::new().appearance(appearance), cx);
        }
        cx.new(|cx| Gallery::new(cx))
    })
}
