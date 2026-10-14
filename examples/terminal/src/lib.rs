//! A terminal with browser-style tabs: each tab is its own shell session, listed in the
//! sidebar by the title the shell reports. Cmd-T (Ctrl-Shift-T elsewhere) opens a tab, Cmd-W
//! closes the selected one, and a tab closes itself when its shell exits. Sessions belong to the
//! app, not the view, so switching tabs keeps each one's scrollback and running program.

use nd::prelude::*;
use nd_terminal::{Command, Exited, LinkActivated, SessionOptions, TerminalSession, TitleChanged, terminal};

actions!(terminal_app, [NewTab, CloseTab]);

struct Tab {
    id: u64,
    title: SharedString,
    session: TerminalSession,
}

pub struct Terminals {
    tabs: Vec<Tab>,
    selected: u64,
    next: u64,
    command: Command,
}

impl Terminals {
    fn new(command: Command, sessions: Vec<TerminalSession>) -> Self {
        let mut this = Self { tabs: Vec::new(), selected: 0, next: 1, command };
        for session in sessions {
            this.push(session);
        }
        this
    }

    fn push(&mut self, session: TerminalSession) {
        let id = self.next;
        self.next += 1;
        self.tabs.push(Tab { id, title: "Shell".into(), session });
        self.selected = id;
    }

    fn new_tab(&mut self, _: &NewTab, _: &mut Window, cx: &mut Context<Self>) {
        match TerminalSession::spawn(&self.command, SessionOptions::default()) {
            Ok(session) => self.push(session),
            Err(e) => eprintln!("terminal: could not start a shell: {e}"),
        }
        cx.notify();
    }

    fn close_tab(&mut self, _: &CloseTab, _: &mut Window, cx: &mut Context<Self>) {
        let selected = self.selected;
        self.close(selected);
        cx.notify();
    }

    fn close(&mut self, id: u64) {
        let Some(at) = self.tabs.iter().position(|t| t.id == id) else { return };
        self.tabs.remove(at);
        if self.selected == id {
            self.selected = self.tabs.get(at.min(self.tabs.len().saturating_sub(1))).map_or(0, |t| t.id);
        }
    }

    pub fn tab_titles(&self) -> Vec<SharedString> {
        self.tabs.iter().map(|t| t.title.clone()).collect()
    }
}

impl Render for Terminals {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = self.tabs.iter().find(|t| t.id == self.selected);
        let title = selected.map_or_else(|| SharedString::from("Terminal"), |t| t.title.clone());
        let content = match selected {
            Some(tab) => {
                let id = tab.id;
                terminal(SharedString::from(format!("term-{id}")), &tab.session)
                    .size_full()
                    .on_title_changed(cx.listener(move |this, e: &TitleChanged, _, cx| {
                        if let Some(tab) = this.tabs.iter_mut().find(|t| t.id == id) {
                            tab.title = e.title.clone().into();
                            cx.notify();
                        }
                    }))
                    .on_exit(cx.listener(move |this, _: &Exited, _, cx| {
                        this.close(id);
                        cx.notify();
                    }))
                    .on_link(|e: &LinkActivated, _, _| open_url(&e.link.url))
                    .into_any_element()
            }
            None => v_flex().size_full().items_center().justify_center().child(text("No open tabs")).into_any_element(),
        };
        let items: Vec<SourceItem> =
            self.tabs.iter().map(|t| source_item(t.id.to_string(), t.title.clone()).icon(Icon::FileText)).collect();
        scaffold()
            .key_context("Terminals")
            .on_action(cx.listener(Self::new_tab))
            .on_action(cx.listener(Self::close_tab))
            .toolbar(
                toolbar()
                    .title(title)
                    .sidebar_toggle()
                    .flexible_space()
                    .item(icon_button("new-tab", Icon::Plus).tooltip("New Tab").dispatch(NewTab)),
            )
            .sidebar(sidebar().min_w(px(180.)).child(
                source_list("tabs").section("Tabs", items).selected(self.selected.to_string()).on_select(cx.listener(
                    |this, e: &SourceSelected, _, cx| {
                        if let Ok(id) = e.id.parse() {
                            this.selected = id;
                            cx.notify();
                        }
                    },
                )),
            ))
            .content(content)
    }
}

fn open_url(url: &str) {
    let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    if let Err(e) = std::process::Command::new(opener).arg(url).spawn() {
        eprintln!("terminal: could not open {url}: {e}");
    }
}

pub fn app() -> Application {
    Application::new(nd::manifest!())
}

/// The bindings: Cmd-T and Cmd-W, Ctrl-Shift-T and Ctrl-Shift-W off macOS (plain Ctrl-T and
/// Ctrl-W belong to the shell).
pub fn bind_keys(cx: &mut App) {
    let (new, close) = if cfg!(target_os = "macos") { ("cmd-t", "cmd-w") } else { ("ctrl-shift-t", "ctrl-shift-w") };
    cx.bind_keys([
        KeyBinding::new(new, NewTab, Some("Terminals")),
        KeyBinding::new(close, CloseTab, Some("Terminals")),
    ]);
}

/// The window with one tab running the user's shell.
pub fn open(cx: &mut App) -> WindowHandle<Terminals> {
    bind_keys(cx);
    let command = Command::shell();
    let sessions = TerminalSession::spawn(&command, SessionOptions::default()).into_iter().collect();
    open_with(cx, command, sessions)
}

/// The window with `sessions` as its tabs, new tabs running `command`.
pub fn open_with(cx: &mut App, command: Command, sessions: Vec<TerminalSession>) -> WindowHandle<Terminals> {
    cx.open_window(WindowOptions::main("main").title("Terminal").size(size(px(900.), px(560.))), move |_window, cx| {
        cx.new(move |_cx| Terminals::new(command, sessions))
    })
}
