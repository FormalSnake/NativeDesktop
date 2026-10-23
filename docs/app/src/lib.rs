//! The NativeDesktop docs as an nd app: the pages of docs/content in a source list sidebar, the
//! command palette over page titles (secondary-K), and each page's blocks in a scrolling column.
//! Example embeds run the web-capable examples live and show the per-OS captures.

mod blocks;
pub mod content;
mod embed;

use std::collections::HashMap;
use std::rc::Rc;

use nd::prelude::*;

pub use content::Content;

actions!(docs, [OpenPalette]);

pub const CONTEXT: &str = "Docs";

pub const REPO_URL: &str = "https://github.com/FormalStudio/NativeDesktop";

/// A file or directory in the repository on GitHub.
pub fn repo_url(path: &str) -> String {
    format!("{REPO_URL}/blob/main/{path}")
}

/// What the docs show when they open; tests pick a page.
#[derive(Clone, Debug, Default)]
pub struct Start {
    /// A page id (`concepts/theming`); the first page when `None`.
    pub page: Option<String>,
    pub appearance: Option<Appearance>,
}

pub struct Docs {
    content: Rc<Content>,
    load_error: Option<SharedString>,
    page: String,
    palette_open: bool,
    query: SharedString,
    /// The selected embed tab per example name.
    tabs: HashMap<String, embed::Tab>,
}

impl Docs {
    pub fn new(start: &Start, _cx: &mut Context<Self>) -> Self {
        let (content, load_error) = match Content::load() {
            Ok(content) => (content, None),
            Err(e) => (Content::default(), Some(e.into())),
        };
        let first = content.site.sections.iter().flat_map(|s| s.pages.first()).next().cloned().unwrap_or_default();
        let page = start.page.clone().filter(|p| content.site.page(p).is_some()).unwrap_or(first);
        Self {
            content: Rc::new(content),
            load_error,
            page,
            palette_open: false,
            query: SharedString::default(),
            tabs: HashMap::new(),
        }
    }

    pub fn content(&self) -> &Content {
        &self.content
    }

    pub fn page(&self) -> &str {
        &self.page
    }

    fn show(&mut self, id: &str) {
        if self.content.site.page(id).is_some() {
            self.page = id.to_owned();
        }
    }

    fn nodes(&self) -> Vec<SourceNode> {
        let site = &self.content.site;
        let mut nodes = Vec::new();
        for (i, section) in site.sections.iter().enumerate() {
            let section_id = format!("section:{i}");
            nodes.push(source_section(section_id.clone(), section.title.clone()));
            for page in section.pages.iter().filter_map(|id| site.page(id)) {
                nodes.push(
                    source_node(page.id.clone(), page.title.clone()).parent(section_id.clone()).icon(Icon::FileText),
                );
            }
        }
        nodes
    }

    fn palette_items(&self) -> Vec<PaletteItem> {
        let q = self.query.to_lowercase();
        fuzzy::rank(&self.query, &self.content.site.pages, |p| p.title.as_str())
            .map(|p| {
                let item = palette_item(format!("page:{}", p.id), p.title.clone()).icon(Icon::FileText);
                let item = item.when(!p.description.is_empty(), |i| i.subtitle(p.description.clone()));
                let item = item.when(p.id == self.page, |i| i.hint("Showing"));
                item.when(!q.is_empty() && p.title.to_lowercase().starts_with(&q), |i| i.completion(p.title.clone()))
            })
            .collect()
    }

    fn body(&self, theme: &Theme, cx: &Context<Self>) -> AnyElement {
        if let Some(error) = &self.load_error {
            return status_page("The docs did not load")
                .icon(Icon::TriangleAlert)
                .description(error.clone())
                .into_any_element();
        }
        let Some(page) = self.content.site.page(&self.page) else {
            return status_page("No pages").icon(Icon::FileText).into_any_element();
        };
        let blocks: Vec<AnyElement> = page
            .blocks
            .iter()
            .enumerate()
            .map(|(i, b)| self.block(b, &format!("{}/{i}", page.id), theme, cx))
            .collect();
        let column = v_flex()
            .w_full()
            .max_w(theme.spacing.content_width)
            .p(theme.spacing.content_margin)
            .gap(theme.spacing.md)
            .child(
                label(page.title.clone())
                    .id("page-title")
                    .wraps()
                    .text_size(theme.type_scale.large_title)
                    .font_weight(FontWeight::BOLD),
            )
            .when(!page.description.is_empty(), |c| {
                c.child(
                    label(page.description.clone())
                        .id("page-description")
                        .wraps()
                        .secondary()
                        .text_size(theme.type_scale.headline),
                )
            })
            .children(blocks);
        // Keyed by page, so a new page starts its own scroll view at the top.
        v_flex()
            .id(format!("page:{}", page.id))
            .size_full()
            .overflow_y_scroll()
            .child(h_flex().w_full().items_start().justify_center().child(column))
            .into_any_element()
    }
}

impl Render for Docs {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let title = self.content.site.page(&self.page).map_or_else(|| "Docs".to_owned(), |p| p.title.clone());
        let body = self.body(&theme, cx);
        scaffold()
            .key_context(CONTEXT)
            .on_action(cx.listener(|this, _: &OpenPalette, _, cx| {
                this.palette_open = !this.palette_open;
                this.query = SharedString::default();
                cx.notify();
            }))
            .toolbar(
                toolbar()
                    .title(title)
                    .sidebar_toggle()
                    .flexible_space()
                    .item(icon_button("search", Icon::Search).tooltip("Search pages").dispatch(OpenPalette)),
            )
            .sidebar(
                sidebar().min_w(px(220.)).child(
                    source_tree("pages")
                        .nodes(self.nodes())
                        .selected(self.page.clone())
                        .empty_state(Some(Icon::FileText), "No pages", None)
                        .on_select(cx.listener(|this, e: &SourceSelected, _, cx| {
                            this.show(&e.id);
                            cx.notify();
                        })),
                ),
            )
            .content(body)
            .overlay(
                command_palette("command-palette")
                    .open(self.palette_open)
                    .placeholder("Search pages")
                    .query(self.query.clone())
                    .items(self.palette_items())
                    .on_query_change(cx.listener(|this, e: &PaletteQueryChanged, _, cx| {
                        this.query = e.text.clone();
                        cx.notify();
                    }))
                    .on_activate(cx.listener(|this, e: &PaletteActivated, _, cx| {
                        this.palette_open = false;
                        if let Some(id) = e.id.strip_prefix("page:") {
                            this.show(id);
                        }
                        cx.notify();
                    }))
                    .on_submit(cx.listener(|this, e: &PaletteSubmitted, _, cx| {
                        this.palette_open = false;
                        let first = fuzzy::rank(&e.text, &this.content.site.pages, |p| p.title.as_str()).next();
                        if let Some(id) = first.map(|p| p.id.clone()) {
                            this.show(&id);
                        }
                        cx.notify();
                    }))
                    .on_cancel(cx.listener(|this, _: &PaletteCancelled, _, cx| {
                        this.palette_open = false;
                        cx.notify();
                    })),
            )
    }
}

pub fn app() -> Application {
    Application::new(nd::manifest!())
}

pub fn init(cx: &mut App) -> WindowHandle<Docs> {
    open(cx, Start::default())
}

/// Binds the docs' keys and opens the window on `start`'s page.
pub fn open(cx: &mut App, start: Start) -> WindowHandle<Docs> {
    cx.bind_keys([KeyBinding::new("secondary-k", OpenPalette, Some(CONTEXT))]);
    set_menus(
        cx,
        [
            Menu::app([]),
            Menu::new("View")
                .items([MenuItem::action("Search Pages", OpenPalette), MenuItem::role(MenuRole::ToggleSidebar)]),
        ],
    );
    cx.open_window(
        WindowOptions::main("docs").title("NativeDesktop Docs").size(size(px(1080.), px(740.))),
        move |window, cx| {
            if let Some(appearance) = start.appearance {
                window.set_theme(ThemeOverride::new().appearance(appearance), cx);
            }
            cx.new(|cx| Docs::new(&start, cx))
        },
    )
}
