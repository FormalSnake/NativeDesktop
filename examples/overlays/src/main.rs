//! Overlays over native views: a popover, a modal panel and a context menu, opened over a native
//! button and a web page. Whatever happens inside an overlay never reaches what is under it; the
//! status line counts what the button and the page received. The M4 overlay gate drives it.
//!
//! `OVERLAYS_RESIZE=1` shows only a web page filling the window, for the resize gate.

use nd::prelude::*;

actions!(overlays, [Choose]);

const PAGE: &str = "<!doctype html><html><body style=\"margin:0;height:100vh;background:#2f8f5b;\
font:15px -apple-system;color:white;display:flex;align-items:center;justify-content:center\">\
page</body></html>";

const EDGE_PAGE: &str = "<!doctype html><html><body style=\"margin:0;height:100vh;background:#2f8f5b\"></body></html>";

struct Scene {
    popover: bool,
    panel: bool,
    under: u32,
    chosen: u32,
    resize_only: bool,
}

impl Scene {
    fn status(&self) -> String {
        format!(
            "under={} chosen={} popover={} panel={}",
            self.under,
            self.chosen,
            if self.popover { "open" } else { "closed" },
            if self.panel { "open" } else { "closed" },
        )
    }
}

impl Scene {
    // docs:begin popover
    /// Anchored to the element it is a child of; `on_dismiss` is where the app closes it.
    fn popover_view(&self, cx: &mut Context<Self>) -> impl IntoElement {
        popover("pop").w(px(300.)).h(px(200.)).p(px(12.)).child(text("Clicks in here stay in here.")).on_dismiss(
            cx.listener(|this, _: &Dismissed, _, cx| {
                this.popover = false;
                cx.notify();
            }),
        )
    }
    // docs:end popover

    // docs:begin panel
    /// Modal: presses outside are consumed and the panel stays until Esc or the app closes it.
    fn panel_view(&self, cx: &mut Context<Self>) -> impl IntoElement {
        panel("panel").w(px(260.)).h(px(120.)).p(px(12.)).child(text("A modal panel. Esc closes it.")).on_dismiss(
            cx.listener(|this, _: &Dismissed, _, cx| {
                this.panel = false;
                cx.notify();
            }),
        )
    }
    // docs:end panel
}

// docs:begin menu
/// Built when the menu opens; each item dispatches an action.
fn menu_items(menu: ContextMenu, _window: &mut Window, _cx: &mut App) -> ContextMenu {
    menu.item("Choose", Choose).separator().item("Again", Choose)
}
// docs:end menu

impl Render for Scene {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.resize_only {
            // The page is the root's only child: with `ND_DEBUG_EXPOSE` any frame where it lags
            // the window shows the magenta window background.
            return native_surface("web", SurfaceSource::Html(EDGE_PAGE.into())).size_full().into_any_element();
        }
        v_flex()
            .size_full()
            .p(px(16.))
            .gap(px(12.))
            .on_action(cx.listener(|this, _: &Choose, _, cx| {
                this.chosen += 1;
                cx.notify();
            }))
            .child(
                h_flex()
                    .gap(px(8.))
                    .child(
                        div()
                            .id("anchor")
                            .w(px(320.))
                            .h(px(28.))
                            .child(button("open-popover", "Popover").on_click(cx.listener(
                                |this, _: &ClickEvent, _, cx| {
                                    this.popover = true;
                                    cx.notify();
                                },
                            )))
                            .when(self.popover, |d| d.child(self.popover_view(cx))),
                    )
                    .child(button("open-panel", "Panel").on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.panel = true;
                        cx.notify();
                    }))),
            )
            .child(
                h_flex()
                    .items_start()
                    .gap(px(12.))
                    .flex_1()
                    .child(button("under", "Under").w(px(150.)).h(px(32.)).on_click(cx.listener(
                        |this, _: &ClickEvent, _, cx| {
                            this.under += 1;
                            cx.notify();
                        },
                    )))
                    .child(div().id("menu-zone").w(px(40.)).h(px(260.)).bg(rgb(0x8899aa)).context_menu(menu_items))
                    .child(native_surface("web", SurfaceSource::Html(PAGE.into())).count_input().flex_1().h(px(320.))),
            )
            .child(text(self.status()).id("status"))
            .when(self.panel, |d| d.child(self.panel_view(cx)))
            .into_any_element()
    }
}

fn main() -> nd::Result<()> {
    let resize_only = std::env::var_os("OVERLAYS_RESIZE").is_some_and(|v| v == "1");
    Application::new(nd::manifest!()).run(move |cx| {
        let options = WindowOptions::main("main").title("Overlays").size(size(px(720.), px(520.)));
        cx.open_window(options, move |_window, cx| {
            cx.new(|_cx| Scene { popover: false, panel: false, under: 0, chosen: 0, resize_only })
        })
    })
}
