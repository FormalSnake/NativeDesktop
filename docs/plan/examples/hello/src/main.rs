use nd::prelude::*;

struct Counter {
    count: u32,
}

impl Render for Counter {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        scaffold()
            .toolbar(toolbar().item(
                button("add", "Add one").on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                    this.count += 1;
                    cx.notify();
                })),
            ))
            .content(
                v_flex()
                    .size_full()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .child(text(self.count.to_string()).text_size(px(48.)).tabular_nums())
                    .child(text("clicks").text_color(theme.text.secondary)),
            )
    }
}

fn main() -> nd::Result<()> {
    Application::new(nd::manifest!()).run(|cx| {
        cx.open_window(WindowOptions::main("main").title("Hello"), |_window, cx| {
            cx.new(|_cx| Counter { count: 0 })
        })
    })
}
