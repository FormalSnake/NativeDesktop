//! The layout and windows pages: in-window tabs, splits over a pane model, the status page and
//! dialogs. Each is a view of its own, so its state survives switching pages.

use nd::prelude::*;

/// Tabs: a view switcher over settings-style pages, and closable document tabs.
pub struct TabsDemo {
    selected: SharedString,
    documents: Vec<SharedString>,
    document: SharedString,
    next: u32,
    pub last: SharedString,
}

impl TabsDemo {
    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            selected: "general".into(),
            documents: vec!["Untitled".into(), "Notes".into()],
            document: "Untitled".into(),
            next: 1,
            last: "No tab picked yet".into(),
        }
    }
}

impl Render for TabsDemo {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let secondary = cx.theme().text.secondary;
        let documents = self.documents.iter().map(|d| {
            tab_page(d.clone(), d.clone())
                .closable()
                .child(v_flex().p_3().child(label(format!("The contents of {d}.")).text_style(TextStyle::Body)))
        });
        v_flex()
            .gap_4()
            .w_full()
            .child(
                tab_view("settings-tabs")
                    .label("Settings")
                    .selected(self.selected.clone())
                    .tab(
                        tab_page("general", "General").child(
                            v_flex()
                                .p_3()
                                .gap_2()
                                .child(
                                    h_flex()
                                        .gap_2()
                                        .child(switch("launch").label("Open at login"))
                                        .child(label("Open at login")),
                                )
                                .child(
                                    h_flex()
                                        .gap_2()
                                        .child(switch("updates").label("Check for updates"))
                                        .child(label("Check for updates")),
                                ),
                        ),
                    )
                    .tab(tab_page("appearance", "Appearance").child(
                        v_flex().p_3().gap_2().child(segmented("scheme", ["Light", "Dark", "Auto"]).selected(2)),
                    ))
                    .tab(
                        tab_page("advanced", "Advanced").child(
                            v_flex()
                                .p_3()
                                .gap_2()
                                .child(label("Every page stays mounted, and the tallest sets the height."))
                                .child(text_field("proxy").placeholder("Proxy address"))
                                .child(
                                    h_flex()
                                        .gap_2()
                                        .child(switch("logging").label("Verbose logging"))
                                        .child(label("Verbose logging")),
                                ),
                        ),
                    )
                    .on_select(cx.listener(|this, e: &TabChanged, _, cx| {
                        this.selected = e.id.clone();
                        this.last = format!("Selected {}", e.id).into();
                        cx.notify();
                    })),
            )
            .child(
                // docs:begin tab-view
                tab_view("documents")
                    .presentation(TabStyle::Strip)
                    .add_button(true)
                    .selected(self.document.clone())
                    .h(px(140.))
                    .tabs(documents)
                    .on_select(cx.listener(|this, e: &TabChanged, _, cx| {
                        this.document = e.id.clone();
                        cx.notify();
                    }))
                    .on_close(cx.listener(|this, e: &TabCloseRequested, _, cx| {
                        this.documents.retain(|d| *d != e.id);
                        if this.document == e.id {
                            this.document = this.documents.first().cloned().unwrap_or_default();
                        }
                        this.last = format!("Closed {}", e.id).into();
                        cx.notify();
                    }))
                    .on_add(cx.listener(|this, _: &TabAddRequested, _, cx| {
                        let name: SharedString = format!("Document {}", this.next).into();
                        this.next += 1;
                        this.documents.push(name.clone());
                        this.document = name;
                        cx.notify();
                    })), // docs:end tab-view
            )
            .child(text(self.last.clone()).text_color(secondary))
    }
}

/// Splits: a pane model rendered as nested splits; panes split and close from buttons.
pub struct SplitDemo {
    tree: panes::PaneTree<SharedString>,
    next: u32,
}

impl SplitDemo {
    // docs:begin pane-model
    pub fn new(_cx: &mut Context<Self>) -> Self {
        let tree = panes::seed_panes(vec![SharedString::from("Editor"), SharedString::from("Preview")]);
        Self { tree, next: 1 }
    }

    fn split(&mut self, axis: panes::Axis) {
        let focused = self.tree.focused_id().clone();
        let name: SharedString = format!("Pane {}", self.next).into();
        self.next += 1;
        self.tree = panes::split_pane(&self.tree, &focused, axis, name, None);
    }
    // docs:end pane-model
}

impl Render for SplitDemo {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let raised = cx.theme().surface.raised;
        let accent = cx.theme().accent;
        let weak = cx.weak_entity();
        v_flex()
            .gap_3()
            .w_full()
            .child(
                h_flex()
                    .gap_2()
                    .child(button("split-right", "Split right").on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.split(panes::Axis::Horizontal);
                        cx.notify();
                    })))
                    .child(button("split-down", "Split down").on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.split(panes::Axis::Vertical);
                        cx.notify();
                    })))
                    .child(button("close-pane", "Close pane").on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        let focused = this.tree.focused_id().clone();
                        this.tree = panes::close_pane(&this.tree, &focused);
                        cx.notify();
                    }))),
            )
            .child(
                div().h(px(260.)).w_full().child(
                    // docs:begin pane-tree
                    pane_tree("panes", &self.tree, move |leaf, focused| {
                        let id = leaf.id.clone();
                        let weak = weak.clone();
                        div()
                            .size_full()
                            .p_3()
                            .bg(raised)
                            .when(focused, |d| d.border_2().border_color(accent))
                            .on_tap(move |_, _, cx| {
                                weak.update(cx, |this, cx| {
                                    this.tree = panes::focus_pane(&this.tree, &id);
                                    cx.notify();
                                });
                            })
                            .child(label(leaf.data.clone()))
                            .into_any_element()
                    })
                    .on_resize(cx.listener(|this, e: &PaneResized, _, cx| {
                        let next = panes::set_pane_ratio(&this.tree, &e.split, e.ratio);
                        if !next.same(&this.tree) {
                            this.tree = next;
                            cx.notify();
                        }
                    })), // docs:end pane-tree
                ),
            )
    }
}

/// The status page an empty view shows.
pub fn status_demo() -> AnyElement {
    div()
        .flex_1()
        .min_h_0()
        .w_full()
        .child(
            status_page("No Notes")
                .id("no-notes")
                .icon(Icon::Notebook)
                .description("Notes you create appear here.")
                .child(button("new-note", "New Note").prominent()),
        )
        .into_any_element()
}

/// Dialogs: an alert, a destructive confirmation, a modal alert and the about panel.
pub struct DialogsDemo {
    pub last: SharedString,
}

impl DialogsDemo {
    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self { last: "No dialog answered yet".into() }
    }
}

impl Render for DialogsDemo {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let secondary = cx.theme().text.secondary;
        let weak = cx.weak_entity();
        let report = move |what: &'static str| {
            let weak = weak.clone();
            move |answer: String, cx: &mut App| {
                weak.update(cx, |this, cx| {
                    this.last = format!("{what}: {answer}").into();
                    cx.notify();
                });
            }
        };
        let (alert_done, confirm_done, modal_done) = (report("Alert"), report("Confirm"), report("Modal"));
        v_flex()
            .gap_3()
            .child(
                h_flex()
                    .gap_2()
                    .child(button("show-alert", "Show alert").on_click(move |_, window, cx| {
                        let done = alert_done.clone();
                        let alert = Alert::new("Sync finished").message("12 notes were updated.");
                        window.alert(cx, alert, move |r, cx| done(format!("button {}", r.0), cx));
                    }))
                    .child(button("confirm-delete", "Delete note…").on_click(move |_, window, cx| {
                        let done = confirm_done.clone();
                        let message = Some(SharedString::from("This can't be undone."));
                        window.confirm(cx, "Delete “Groceries”?", message, "Delete", move |yes, cx| {
                            done(if yes { "deleted".into() } else { "kept".into() }, cx)
                        });
                    }))
                    .child(button("modal-alert", "Modal alert").on_click(move |_, window, cx| {
                        let done = modal_done.clone();
                        let alert = Alert::new("Quit Gallery?").button("Quit").cancel("Cancel").modal();
                        window.alert(cx, alert, move |r, cx| done(format!("button {}", r.0), cx));
                    }))
                    .child(button("about", "About").on_click(|_, window, cx| {
                        let about = AboutPanel::new()
                            .name("Gallery")
                            .version(env!("CARGO_PKG_VERSION"))
                            .credits("Every built-in component, on every platform.");
                        window.show_about(cx, about);
                    })),
            )
            .child(text(self.last.clone()).text_color(secondary))
    }
}
