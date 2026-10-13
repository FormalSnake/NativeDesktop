//! The window structure a Mail-style app declares, and nothing else: a scaffold with a source
//! list sidebar, a toolbar, an inspector, a menu bar with key equivalents from the bindings, an
//! alert, a file picker, and a settings window with tabs and grouped rows. On macOS every piece
//! is the system's own component (ARCHITECTURE.md 3.10); the code names none of them.

use nd::prelude::*;

actions!(chrome, [NewMessage, DeleteMessage, OpenFile, OpenSettings, ToggleInspector]);

pub struct Mail {
    folder: SharedString,
    inspector: bool,
    status: SharedString,
}

impl Mail {
    fn delete(&mut self, _: &DeleteMessage, window: &mut Window, cx: &mut Context<Self>) {
        let this = cx.weak_entity();
        let alert =
            Alert::new("Delete this message?").message("It moves to the Trash.").destructive("Delete").cancel("Cancel");
        window.alert(cx, alert, move |answer, cx| {
            let _ = this.update(cx, |this, cx| {
                this.status = if answer.0 == 0 { "Deleted".into() } else { "Kept".into() };
                cx.notify();
            });
        });
    }

    fn open(&mut self, _: &OpenFile, window: &mut Window, cx: &mut Context<Self>) {
        let this = cx.weak_entity();
        window.open_dialog(cx, OpenDialog::file().prompt("Attach"), move |paths, cx| {
            let _ = this.update(cx, |this, cx| {
                this.status = match paths {
                    Some(paths) => format!("{} file(s) attached", paths.len()).into(),
                    None => "Nothing attached".into(),
                };
                cx.notify();
            });
        });
    }
}

impl Render for Mail {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        scaffold()
            .key_context("Mail")
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::open))
            .on_action(|_: &OpenSettings, _, cx| {
                open_settings_window(cx);
            })
            .on_action(cx.listener(|this, _: &ToggleInspector, _, cx| {
                this.inspector = !this.inspector;
                cx.notify();
            }))
            .toolbar(
                toolbar()
                    .title("Inbox")
                    .sidebar_toggle()
                    .item(icon_button("new", Icon::SquarePen).tooltip("New Message").dispatch(NewMessage))
                    .item(icon_button("delete", Icon::Trash).tooltip("Delete").dispatch(DeleteMessage))
                    .flexible_space()
                    .inspector_toggle(),
            )
            .sidebar(
                sidebar().min_w(px(180.)).child(
                    source_list("folders")
                        .section(
                            "Mailboxes",
                            [
                                source_item("inbox", "Inbox").icon(Icon::Inbox).badge("12"),
                                source_item("archive", "Archive").icon(Icon::Archive),
                                source_item("trash", "Trash").icon(Icon::Trash),
                            ],
                        )
                        .section("Smart", [source_item("starred", "Starred").icon(Icon::Star)])
                        .selected(self.folder.clone())
                        .on_select(cx.listener(|this, e: &SourceSelected, _, cx| {
                            this.folder = e.id.clone();
                            cx.notify();
                        })),
                ),
            )
            .content(
                v_flex()
                    .size_full()
                    .p(theme.spacing.content_margin)
                    .gap_3()
                    .child(text(self.folder.clone()).text_size(theme.type_scale.title))
                    .child(text(self.status.clone()).text_color(theme.text.secondary))
                    .child(glass().rounded_lg().p_3().child(text("Drafts sync when you are online."))),
            )
            .inspector(
                inspector()
                    .collapsed(!self.inspector)
                    .min_w(px(200.))
                    .child(v_flex().p_3().child(text("Details").text_color(theme.text.secondary))),
            )
    }
}

pub struct Settings {
    tab: SharedString,
    density: usize,
    size: f32,
}

impl Settings {
    pub fn tab(&self) -> &SharedString {
        &self.tab
    }
}

impl Render for Settings {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = if self.tab.as_ref() == "general" {
            form().child(
                form_section()
                    .title("Reading")
                    .child(form_row("Density").child(
                        select("density", ["Comfortable", "Compact"]).selected(self.density).on_change(cx.listener(
                            |this, e: &SelectChanged, _, cx| {
                                this.density = e.index;
                                cx.notify();
                            },
                        )),
                    ))
                    .child(form_row("Text size").help("Applies to the message list").child(
                        slider("size", self.size).range(11.0..=24.0).w(px(180.)).on_change(cx.listener(
                            |this, e: &SliderChanged, _, cx| {
                                this.size = e.value;
                                cx.notify();
                            },
                        )),
                    )),
            )
        } else {
            form().child(
                form_section()
                    .title("Accounts")
                    .footer("Accounts sync every five minutes.")
                    .child(form_row("Work").child(button("work", "Edit…"))),
            )
        };
        div()
            .size_full()
            .child(
                toolbar()
                    .tab("general", "General", Icon::Settings)
                    .tab("accounts", "Accounts", Icon::Inbox)
                    .selected_tab(self.tab.clone())
                    .on_tab(cx.listener(|this, e: &TabSelected, _, cx| {
                        this.tab = e.id.clone();
                        cx.notify();
                    })),
            )
            .child(body)
    }
}

pub fn app() -> Application {
    Application::new(nd::manifest!())
}

/// Opens the settings window, or brings it forward.
pub fn open_settings_window(cx: &mut App) -> Option<WindowHandle<Settings>> {
    open_settings(cx, |_, cx| cx.new(|_| Settings { tab: "general".into(), density: 0, size: 15. }))
}

pub fn init(cx: &mut App) -> WindowHandle<Mail> {
    cx.bind_keys([
        KeyBinding::new("secondary-n", NewMessage, Some("Mail")),
        KeyBinding::new("secondary-backspace", DeleteMessage, Some("Mail")),
        KeyBinding::new("secondary-o", OpenFile, Some("Mail")),
        KeyBinding::new("secondary-,", OpenSettings, None),
        KeyBinding::new("secondary-alt-i", ToggleInspector, Some("Mail")),
    ]);
    set_menus(
        cx,
        [
            Menu::app([MenuItem::action("Settings…", OpenSettings)]),
            Menu::new("File")
                .items([MenuItem::action("New Message", NewMessage), MenuItem::action("Attach File…", OpenFile)]),
            Menu::new("Message").items([MenuItem::action("Delete", DeleteMessage).destructive()]),
            Menu::new("View").items([MenuItem::action("Show Inspector", ToggleInspector)]),
        ],
    );
    cx.open_window(WindowOptions::main("mail").title("Mail").size(size(px(980.), px(620.))), |_, cx| {
        cx.new(|_| Mail { folder: "inbox".into(), inspector: true, status: "Ready".into() })
    })
}
