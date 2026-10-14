//! Browser-style window tabs: every tab is a window of one tab group, so on macOS they are the
//! system's own window tabs (tab bar, `+`, drag out to a new window, Show All Tabs) and
//! elsewhere tabs of a drawn tab bar that tears off into a new window. Cmd-T (Ctrl-T elsewhere)
//! and the tab bar's `+` open a tab, Cmd-W and a tab's close button close one. The app owns the
//! tabs: a close is held until the app has dropped the tab's page, then the window goes.

use nd::core::WindowId;
use nd::prelude::*;

actions!(tabs_app, [NewTab, CloseTab, ShowAllTabs]);

pub const CONTEXT: &str = "Tabs";
pub const GROUP: &str = "browser";

/// What one tab shows: an address field over the page, or a status page while it has none.
pub struct Page {
    pub number: u32,
    address: Entity<TextFieldState>,
    pub url: Option<SharedString>,
}

impl Render for Page {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = match &self.url {
            Some(url) => webview(format!("page-{}", self.number), url.clone()).size_full().into_any_element(),
            None => status_page("New Tab")
                .id("new-tab")
                .icon(Icon::Globe)
                .description("Type an address to open a page.")
                .into_any_element(),
        };
        v_flex()
            .size_full()
            .child(h_flex().p_2().child(
                text_field("address").bind(&self.address).placeholder("Search or enter address").flex_1().on_submit(
                    cx.listener(|this: &mut Page, e: &TextSubmitted, window, cx| {
                        let typed = e.text.trim();
                        if !typed.is_empty() {
                            let url = if typed.contains("://") { typed.to_owned() } else { format!("https://{typed}") };
                            // The window's title is the tab's title.
                            cx.set_window_title(window.id(), typed.to_owned());
                            this.url = Some(url.into());
                            cx.notify();
                        }
                    }),
                ),
            ))
            .child(div().flex_1().min_h_0().child(content))
    }
}

/// One open tab: its window and its page.
pub struct Tab {
    pub window: WindowId,
    pub page: Entity<Page>,
}

/// Every open tab, oldest first.
#[derive(Default)]
pub struct Browser {
    pub tabs: Vec<Tab>,
    next: u32,
    /// Tabs whose close the app received, in order, for tests.
    pub closed: Vec<u32>,
}

/// A tab window's root: dispatches the tab actions and shows the page.
pub struct TabWindow {
    browser: Entity<Browser>,
    page: Entity<Page>,
}

impl Render for TabWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .key_context(CONTEXT)
            .on_action(cx.listener(|this: &mut TabWindow, _: &NewTab, _, cx| {
                open_tab(cx, &this.browser);
            }))
            .on_action(cx.listener(|this: &mut TabWindow, _: &CloseTab, window, cx| {
                close_tab(cx, &this.browser, window.id());
            }))
            .on_action(cx.listener(|_: &mut TabWindow, _: &ShowAllTabs, window, cx| {
                cx.window_command(window.id(), nd::backend::WindowCommand::ToggleTabOverview);
            }))
            .child(self.page.clone())
    }
}

/// Opens a tab next to the group's current one and selects it.
pub fn open_tab(cx: &mut App, browser: &Entity<Browser>) -> WindowHandle<TabWindow> {
    let number = browser.update(cx, |b, _| {
        b.next += 1;
        b.next
    });
    let options = WindowOptions::main("tab").title("New Tab").size(size(px(900.), px(600.))).tab_group(GROUP);
    let browser = browser.clone();
    let handle = cx.open_window(options, move |window, cx| {
        let new_tab = browser.clone();
        window.on_new_tab(move |_, cx| {
            open_tab(cx, &new_tab);
        });
        let closing = browser.clone();
        window.on_should_close(move |window, cx| {
            // The app drops the tab's page and closes the window itself; returning false keeps
            // the platform from closing it a second time.
            close_tab(cx, &closing, window.id());
            false
        });
        let address = cx.new(|_| TextFieldState::default());
        let page = cx.new(|_| Page { number, address, url: None });
        cx.new(|_| TabWindow { browser, page })
    });
    let page = handle.root().read(cx).page.clone();
    handle.root().read(cx).browser.clone().update(cx, |b, _| b.tabs.push(Tab { window: handle.id(), page }));
    handle
}

/// Drops the tab's page, then closes its window.
pub fn close_tab(cx: &mut App, browser: &Entity<Browser>, window: WindowId) {
    let removed = browser.update(cx, |b, _| {
        let at = b.tabs.iter().position(|t| t.window == window)?;
        let tab = b.tabs.remove(at);
        Some(tab.page)
    });
    if let Some(page) = removed {
        let number = page.read(cx).number;
        browser.update(cx, |b, _| b.closed.push(number));
    }
    cx.close_window(window);
}

pub fn app() -> Application {
    Application::new(nd::manifest!())
}

/// The bindings and menus: New Tab and Close Tab in File, Show All Tabs in Window, so the menu
/// bar and the shortcuts never disagree.
pub fn bind(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("secondary-t", NewTab, Some(CONTEXT)),
        KeyBinding::new("secondary-w", CloseTab, Some(CONTEXT)),
        KeyBinding::new("secondary-shift-\\", ShowAllTabs, Some(CONTEXT)),
    ]);
    set_menus(
        cx,
        [
            Menu::app([]),
            Menu::new("File").items([MenuItem::action("New Tab", NewTab), MenuItem::action("Close Tab", CloseTab)]),
            Menu::window([MenuItem::action("Show All Tabs", ShowAllTabs)]),
        ],
    );
}

/// Binds the keys and opens the first tab.
pub fn init(cx: &mut App) -> Entity<Browser> {
    bind(cx);
    let browser = cx.new(|_| Browser::default());
    open_tab(cx, &browser);
    browser
}
