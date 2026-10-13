//! The component gallery: one page per built-in component, picked in a source list sidebar
//! and from the command palette (secondary-K). On macOS every piece is AppKit's own: the
//! source list is an `NSOutlineView`, the palette a glass card over the window, the toasts
//! glass capsules. On Linux and Windows the same tree draws through GPUI.

use std::time::Duration;

use nd::prelude::*;

actions!(gallery, [OpenPalette, ToggleSidebar]);

pub const CONTEXT: &str = "Gallery";

/// The gallery's pages, in sidebar order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Palette,
    SourceList,
    Toasts,
    Banner,
}

impl Page {
    pub const ALL: [Page; 4] = [Page::Palette, Page::SourceList, Page::Toasts, Page::Banner];

    pub fn id(self) -> &'static str {
        match self {
            Page::Palette => "palette",
            Page::SourceList => "source-list",
            Page::Toasts => "toasts",
            Page::Banner => "banner",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Page::Palette => "Command palette",
            Page::SourceList => "Source list",
            Page::Toasts => "Toasts",
            Page::Banner => "Banner",
        }
    }

    fn icon(self) -> Icon {
        match self {
            Page::Palette => Icon::Search,
            Page::SourceList => Icon::PanelLeft,
            Page::Toasts => Icon::Info,
            Page::Banner => Icon::TriangleAlert,
        }
    }

    fn section(self) -> &'static str {
        match self {
            Page::Palette | Page::SourceList => "navigation",
            Page::Toasts | Page::Banner => "feedback",
        }
    }

    fn by_id(id: &str) -> Option<Page> {
        Page::ALL.into_iter().find(|p| p.id() == id)
    }
}

/// What the gallery shows when it opens; tests and goldens pick a state.
#[derive(Clone, Debug)]
pub struct Start {
    pub page: Page,
    pub palette: Option<SharedString>,
    pub toast: bool,
    pub appearance: Option<Appearance>,
}

impl Default for Start {
    fn default() -> Self {
        Self { page: Page::Palette, palette: None, toast: false, appearance: None }
    }
}

pub struct Gallery {
    page: Page,
    sidebar_collapsed: bool,
    palette_open: bool,
    query: SharedString,
    /// Folders the source list page adds; the sidebar shows them under "Folders".
    folders: Vec<SharedString>,
    folders_open: bool,
    toasts: Vec<Toast>,
    next_toast: u32,
    banner: bool,
    last_event: SharedString,
}

fn secondary_k() -> &'static str {
    if cfg!(target_os = "macos") { "\u{2318}K" } else { "Ctrl+K" }
}

impl Gallery {
    fn new(start: &Start) -> Self {
        let mut this = Self {
            page: start.page,
            sidebar_collapsed: false,
            palette_open: start.palette.is_some(),
            query: start.palette.clone().unwrap_or_default(),
            folders: vec!["Drafts".into(), "Ideas".into()],
            folders_open: true,
            toasts: Vec::new(),
            next_toast: 0,
            banner: true,
            last_event: "Nothing yet".into(),
        };
        if start.toast {
            this.toasts.push(toast("welcome", "Note deleted").action("Undo").persistent());
        }
        this
    }

    fn nodes(&self) -> Vec<SourceNode> {
        let mut nodes = vec![source_section("navigation", "Navigation"), source_section("feedback", "Feedback")];
        // Pages under their section, in page order: insert after the section header.
        for page in Page::ALL {
            let node = source_node(page.id(), page.title()).parent(page.section()).icon(page.icon());
            nodes.push(node);
        }
        nodes.push(source_section("folders", "Folders"));
        nodes.push(
            source_node("all-folders", "All folders")
                .parent("folders")
                .icon(Icon::Folder)
                .badge(self.folders.len().to_string())
                .expanded(self.folders_open),
        );
        for name in &self.folders {
            nodes.push(
                source_node(format!("folder:{name}"), name.clone())
                    .parent("all-folders")
                    .icon(Icon::Folder)
                    .actions(["remove"]),
            );
        }
        nodes
    }

    fn show_toast(&mut self, title: &str, build: impl FnOnce(Toast) -> Toast) {
        self.next_toast += 1;
        self.toasts.push(build(toast(format!("t{}", self.next_toast), title.to_owned())));
    }

    /// The palette's rows: pages, then commands, ranked by the query. A page whose title
    /// starts with the query offers itself as the inline completion.
    fn palette_items(&self) -> Vec<PaletteItem> {
        let q = self.query.to_lowercase();
        let mut items: Vec<PaletteItem> = fuzzy::rank(&self.query, Page::ALL, |p| p.title())
            .map(|p| {
                let item = palette_item(format!("page:{}", p.id()), p.title()).icon(p.icon()).subtitle("Page");
                let item = item.when(p == self.page, |i| i.hint("Showing"));
                item.when(!q.is_empty() && p.title().to_lowercase().starts_with(&q), |i| i.completion(p.title()))
            })
            .collect();
        let commands = [
            ("cmd:toast", "Show a toast", Icon::Info),
            ("cmd:banner", "Toggle the banner", Icon::TriangleAlert),
            ("cmd:sidebar", "Toggle the sidebar", Icon::PanelLeft),
        ];
        items.extend(
            fuzzy::rank(&self.query, commands, |c| c.1)
                .map(|(id, title, icon)| palette_item(id, title).icon(icon).subtitle("Command")),
        );
        items
    }

    fn activate(&mut self, id: &str) {
        self.palette_open = false;
        match id {
            "cmd:toast" => self.show_toast("Saved to Drafts", |t| t),
            "cmd:banner" => self.banner = !self.banner,
            "cmd:sidebar" => self.sidebar_collapsed = !self.sidebar_collapsed,
            other => {
                if let Some(page) = other.strip_prefix("page:").and_then(Page::by_id) {
                    self.page = page;
                }
            }
        }
    }

    fn page_body(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let secondary = theme.text.secondary;
        let column = v_flex().gap_3().max_w(px(560.));
        let note = |s: &str| text(s.to_owned()).text_color(secondary);
        match self.page {
            Page::Palette => column
                .child(note("Search pages and commands. Type the start of a page title and press Tab to take the completion."))
                .child(
                    button("open-palette", format!("Open command palette ({})", secondary_k())).on_click(cx.listener(
                        |this, _: &ClickEvent, _, cx| {
                            this.palette_open = true;
                            this.query = SharedString::default();
                            cx.notify();
                        },
                    )),
                )
                .into_any_element(),
            Page::SourceList => column
                .child(note("The sidebar is the source list. Hover a folder to show its remove action; open and close All folders."))
                .child(h_flex().gap_2().child(button("add-folder", "Add folder").on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        let n = this.folders.len() + 1;
                        this.folders.push(format!("Folder {n}").into());
                        cx.notify();
                    },
                ))))
                .into_any_element(),
            Page::Toasts => column
                .child(note("Toasts queue one at a time. An urgent toast interrupts the one showing."))
                .child(
                    h_flex()
                        .gap_2()
                        .child(button("toast", "Show toast").on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.show_toast("Saved to Drafts", |t| t);
                            cx.notify();
                        })))
                        .child(button("toast-action", "Show toast with action").on_click(cx.listener(
                            |this, _: &ClickEvent, _, cx| {
                                this.show_toast("Note deleted", |t| t.action("Undo"));
                                cx.notify();
                            },
                        )))
                        .child(button("toast-urgent", "Show urgent toast").on_click(cx.listener(
                            |this, _: &ClickEvent, _, cx| {
                                this.show_toast("You are offline", |t| t.high_priority().timeout(Duration::from_secs(8)));
                                cx.notify();
                            },
                        ))),
                )
                .into_any_element(),
            Page::Banner => column
                .child(note("A banner sits across the top of the content until it is dismissed."))
                .child(button("toggle-banner", if self.banner { "Hide banner" } else { "Show banner" }).on_click(
                    cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.banner = !this.banner;
                        cx.notify();
                    }),
                ))
                .into_any_element(),
        }
    }
}

impl Render for Gallery {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let margin = theme.spacing.content_margin;
        let title_size = theme.type_scale.title;
        let secondary = theme.text.secondary;
        let body = self.page_body(cx);
        scaffold()
            .key_context(CONTEXT)
            .on_action(cx.listener(|this, _: &OpenPalette, _, cx| {
                this.palette_open = !this.palette_open;
                this.query = SharedString::default();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ToggleSidebar, _, cx| {
                this.sidebar_collapsed = !this.sidebar_collapsed;
                cx.notify();
            }))
            .toolbar(
                toolbar()
                    .title("Gallery")
                    .sidebar_toggle()
                    .flexible_space()
                    .item(icon_button("palette-button", Icon::Search).tooltip("Command palette").dispatch(OpenPalette)),
            )
            .sidebar(
                sidebar().collapsed(self.sidebar_collapsed).min_w(px(200.)).child(
                    source_tree("pages")
                        .nodes(self.nodes())
                        .actions([source_action("remove", Icon::Trash).tooltip("Remove folder").destructive()])
                        .selected(self.page.id())
                        .empty_state(Some(Icon::Folder), "No pages", None)
                        .on_select(cx.listener(|this, e: &SourceSelected, _, cx| {
                            if let Some(page) = Page::by_id(&e.id) {
                                this.page = page;
                            }
                            this.last_event = format!("Selected {}", e.id).into();
                            cx.notify();
                        }))
                        .on_expand(cx.listener(|this, e: &SourceExpanded, _, cx| {
                            if e.id == "all-folders" {
                                this.folders_open = true;
                            }
                            cx.notify();
                        }))
                        .on_collapse(cx.listener(|this, e: &SourceCollapsed, _, cx| {
                            if e.id == "all-folders" {
                                this.folders_open = false;
                            }
                            cx.notify();
                        }))
                        .on_action(cx.listener(|this, e: &SourceActionClicked, _, cx| {
                            let name = e.id.strip_prefix("folder:").unwrap_or(&e.id).to_owned();
                            this.folders.retain(|f| *f != name);
                            this.last_event = format!("Removed {name}").into();
                            this.show_toast(&format!("Removed {name}"), |t| t);
                            cx.notify();
                        })),
                ),
            )
            .content(
                v_flex()
                    .size_full()
                    .child(
                        banner("banner", "A new version of Gallery is ready")
                            .action("Restart")
                            .revealed(self.banner)
                            .on_action(cx.listener(|this, _: &BannerActionClicked, _, cx| {
                                this.banner = false;
                                this.last_event = "Restart clicked".into();
                                cx.notify();
                            })),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .p(margin)
                            .gap_3()
                            .child(text(self.page.title()).text_size(title_size))
                            .child(body)
                            .child(text(self.last_event.clone()).text_color(secondary)),
                    )
                    .child(
                        toast_overlay("toasts")
                            .toasts(self.toasts.clone())
                            .on_dismiss(cx.listener(|this, e: &ToastDismissed, _, cx| {
                                this.toasts.retain(|t| t.id != e.id);
                                cx.notify();
                            }))
                            .on_action(cx.listener(|this, _: &ToastActionClicked, _, cx| {
                                this.last_event = "Undo clicked".into();
                                cx.notify();
                            })),
                    ),
            )
            .overlay(
                command_palette("command-palette")
                    .open(self.palette_open)
                    .placeholder("Search pages and commands")
                    .query(self.query.clone())
                    .items(self.palette_items())
                    .on_query_change(cx.listener(|this, e: &PaletteQueryChanged, _, cx| {
                        this.query = e.text.clone();
                        cx.notify();
                    }))
                    .on_activate(cx.listener(|this, e: &PaletteActivated, _, cx| {
                        this.activate(&e.id);
                        cx.notify();
                    }))
                    .on_submit(cx.listener(|this, e: &PaletteSubmitted, _, cx| {
                        this.palette_open = false;
                        this.last_event = format!("Searched for \u{201c}{}\u{201d}", e.text).into();
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

pub fn init(cx: &mut App) -> WindowHandle<Gallery> {
    open(cx, Start::default())
}

/// Binds the gallery's keys and opens its window in `start`'s state.
pub fn open(cx: &mut App, start: Start) -> WindowHandle<Gallery> {
    cx.bind_keys([
        KeyBinding::new("secondary-k", OpenPalette, Some(CONTEXT)),
        KeyBinding::new("secondary-alt-s", ToggleSidebar, Some(CONTEXT)),
    ]);
    set_menus(
        cx,
        [
            Menu::app([]),
            Menu::new("View").items([
                MenuItem::action("Command Palette", OpenPalette),
                MenuItem::action("Toggle Sidebar", ToggleSidebar),
            ]),
        ],
    );
    cx.open_window(WindowOptions::main("gallery").title("Gallery").size(size(px(960.), px(620.))), move |window, cx| {
        if let Some(appearance) = start.appearance {
            window.set_theme(ThemeOverride::new().appearance(appearance), cx);
        }
        cx.new(|_| Gallery::new(&start))
    })
}
