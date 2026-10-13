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

fn main() {
    nd::audit::Audit::new("intents", || Application::new(nd::manifest!()))
        .window("gallery", |cx| {
            cx.open_window(WindowOptions::main("gallery").title("Gallery").size(size(px(900.), px(640.))), |_, cx| {
                cx.new(|_| Gallery)
            })
            .id()
        })
        .run()
}
