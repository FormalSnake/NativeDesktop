//! Tier 2: every `nd-components` intent through the appearance audit (`nd::audit`), in light,
//! dark and matugen themes on each backend kind.

use nd::prelude::*;

struct Gallery;

impl Render for Gallery {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        scaffold()
            .toolbar(toolbar().title("Gallery").sidebar_toggle().item(button("new", "New")).flexible_space())
            .sidebar(
                sidebar().min_w(px(180.)).child(
                    source_list("places")
                        .section("Library", [source_item("all", "All").badge("12"), source_item("recent", "Recent")])
                        .section("Tags", [source_item("work", "Work")])
                        .selected("all"),
                ),
            )
            .content(
                v_flex()
                    .size_full()
                    .p(theme.spacing.content_margin)
                    .gap_3()
                    .child(text("Title").text_size(theme.type_scale.title))
                    .child(text("Body text"))
                    .child(text("Secondary text").text_color(theme.text.secondary))
                    .child(separator())
                    .child(h_flex().gap_2().child(button("ok", "Accept").primary()).child(button("no", "Decline")))
                    .child(select("sort", ["Name", "Date"]).selected(0))
                    .child(select("view", ["List", "Grid"]).selected(1).native(Mapping::Segmented))
                    .child(slider("volume", 0.4).w(px(160.)))
                    .child(slider("drawn", 0.6).drawn().w(px(160.)))
                    .child(glass().rounded_lg().p_3().child(text("On glass")))
                    .child(v_flex().p_3().theme(|t| t.appearance(Appearance::Dark)).child(text("Forced dark subtree")))
                    .child(
                        form().child(
                            form_section().title("Reading").footer("Applies to every window.").child(
                                form_row("Density")
                                    .help("Rows per screen")
                                    .child(select("density", ["Comfortable", "Compact"])),
                            ),
                        ),
                    ),
            )
            .inspector(inspector().min_w(px(200.)).child(v_flex().p_3().child(text("Details"))))
    }
}

/// `nd-virtual` presenters: a list with a selected row, a table under its header, a tree with an
/// expanded branch.
struct Lists {
    plain: ListState<u32>,
    files: ListState<u32>,
    outline: ListState<&'static str>,
}

impl Lists {
    fn new(cx: &mut Context<Self>) -> Self {
        let mut plain = ListState::new(cx);
        plain.set_items(0..20u32, cx);
        plain.select(2, cx);
        let mut files = ListState::new(cx);
        files.set_items(0..20u32, cx);
        files.select(1, cx);
        let mut outline = ListState::new(cx);
        let node = |key, parent| TreeNode { key, parent, version: 0 };
        outline.set_tree([node("Projects", None), node("nd", Some("Projects")), node("Notes", None)], cx);
        outline.set_expanded("Projects", true, cx);
        Self { plain, files, outline }
    }
}

impl Render for Lists {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let columns = vec![column("name", "Name").width(150.).sortable(), column("size", "Size").width(80.)];
        h_flex()
            .size_full()
            .gap_2()
            .child(
                list("plain", &self.plain, |row: u32, _, _| div().h(px(24.)).child(text(format!("Item {row}"))))
                    .selection(Selection::single())
                    .w(px(200.)),
            )
            .child(
                table("files", &self.files, columns, |row: u32, col: usize, _, _| {
                    div().h(px(24.)).child(text(if col == 0 {
                        format!("file-{row}")
                    } else {
                        format!("{} KB", row * 3)
                    }))
                })
                .selection(Selection::single())
                .w(px(240.)),
            )
            .child(
                tree("outline", &self.outline, |key: &'static str, _, _| div().h(px(22.)).child(text(key))).w(px(200.)),
            )
    }
}

/// Overlays over a native surface: an open popover and a modal panel, each with text.
struct Overlays;

impl Render for Overlays {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .p_3()
            .gap_2()
            .child(
                div()
                    .id("anchor")
                    .w(px(200.))
                    .h(px(28.))
                    .child(text("Anchor"))
                    .child(popover("pop").w(px(220.)).p_3().child(text("In a popover"))),
            )
            .child(native_surface("surface", SurfaceSource::TestPattern).w(px(300.)).h(px(160.)))
            .child(panel("panel").w(px(240.)).p_3().child(text("In a panel")))
    }
}

fn main() {
    nd::audit::Audit::new("intents", || Application::new(nd::manifest!()))
        .window("gallery", |cx| {
            cx.open_window(WindowOptions::main("gallery").title("Gallery").size(size(px(900.), px(640.))), |_, cx| {
                cx.new(|_| Gallery)
            })
            .id()
        })
        .window("lists", |cx| {
            cx.open_window(WindowOptions::main("lists").title("Lists").size(size(px(720.), px(400.))), |_, cx| {
                cx.new(Lists::new)
            })
            .id()
        })
        .window("overlays", |cx| {
            cx.open_window(WindowOptions::main("overlays").title("Overlays").size(size(px(640.), px(420.))), |_, cx| {
                cx.new(|_| Overlays)
            })
            .id()
        })
        .run()
}
