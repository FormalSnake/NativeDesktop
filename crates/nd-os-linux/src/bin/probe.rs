//! `nd-linux-probe`: one GPUI window that negotiates decorations the way nd does, draws the
//! controls `chrome::plan` asks for, and makes its title row a drag region. The tier 3 tests on
//! headless Hyprland launch it and read its stderr:
//!
//! - `nd: chrome decorations=.. controls=..` whenever the plan changes (`chrome::describe`)
//! - `nd: viewport WxH` whenever the laid-out size changes
//! - `nd: key <keystroke>` for every keystroke
//! - `nd: first frame` once
//!
//! Colours are fixed so captures can be read back: content `#303030`, title row `#505050`, drawn
//! controls `#e01b24`. `ND_DEBUG_EXPOSE=1` paints the window background magenta under the
//! content. `ND_DEBUG_DEFER_LIVE_LAYOUT=1` lays the content out at a size 300 ms old, the control
//! that proves a capture can see late layout.

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("nd-linux-probe runs on Linux only");
    std::process::exit(2);
}

#[cfg(target_os = "linux")]
fn main() {
    probe::run();
}

#[cfg(target_os = "linux")]
mod probe {
    use std::time::Duration;

    use nd_gpui::gpui::{
        App, AppContext, Bounds, Context, InteractiveElement, IntoElement, MouseButton, ParentElement, Pixels, Render,
        Size, StatefulInteractiveElement, Styled, Window, WindowBounds, WindowOptions, div, point, px, rgb, size,
    };
    use nd_gpui::gpui_platform;
    use nd_os_linux::chrome::{self, Control};
    use nd_os_linux::window::{REQUESTED_DECORATIONS, chrome_plan, chrome_state, drag_region, perform};

    const CONTENT: u32 = 0x303030;
    const TITLE_ROW: u32 = 0x505050;
    const CONTROL: u32 = 0xe01b24;
    const EXPOSE: u32 = 0xff00ff;
    const TITLE_HEIGHT: f32 = 40.;

    struct Probe {
        expose: bool,
        defer: bool,
        /// The size the content is laid out at; the viewport itself unless deferring.
        laid_out: Option<Size<Pixels>>,
        pending: bool,
        reported_chrome: String,
        reported_size: Option<Size<Pixels>>,
        first_frame: bool,
    }

    impl Probe {
        fn content_size(&mut self, window: &Window, cx: &mut Context<Self>) -> Size<Pixels> {
            let viewport = window.viewport_size();
            if !self.defer {
                return viewport;
            }
            let laid_out = *self.laid_out.get_or_insert(viewport);
            if laid_out != viewport && !self.pending {
                self.pending = true;
                cx.spawn_in(window, async move |this, cx| {
                    cx.background_executor().timer(Duration::from_millis(300)).await;
                    let _ = this.update_in(cx, |this, window, cx| {
                        this.pending = false;
                        this.laid_out = Some(window.viewport_size());
                        cx.notify();
                    });
                })
                .detach();
            }
            laid_out
        }
    }

    impl Render for Probe {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let plan = chrome_plan(window, cx);
            let line = chrome::describe(&chrome_state(window, cx), &plan);
            if line != self.reported_chrome {
                eprintln!("nd: chrome {line}");
                self.reported_chrome = line;
            }
            let content = self.content_size(window, cx);
            if self.reported_size != Some(content) {
                eprintln!("nd: viewport {}x{}", f32::from(content.width), f32::from(content.height));
                self.reported_size = Some(content);
            }
            if !self.first_frame {
                self.first_frame = true;
                window.on_next_frame(|_, _| eprintln!("nd: first frame"));
            }

            let control = |c: Control| {
                div()
                    .id(c.name())
                    .size(px(24.))
                    .rounded_full()
                    .bg(rgb(CONTROL))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(move |_, window, _| perform(c, window))
            };
            let side = |controls: &[Control]| div().flex().gap(px(9.)).px(px(6.)).children(controls.iter().map(|c| control(*c)));
            let title_row = drag_region(
                div()
                    .id("title-row")
                    .flex()
                    .items_center()
                    .justify_between()
                    .h(px(TITLE_HEIGHT))
                    .w_full()
                    .bg(rgb(TITLE_ROW)),
            )
            .child(side(&plan.leading))
            .child(side(&plan.trailing));

            div()
                .size_full()
                .bg(rgb(if self.expose { EXPOSE } else { CONTENT }))
                .child(div().absolute().w(content.width).h(content.height).bg(rgb(CONTENT)).child(title_row))
        }
    }

    pub fn run() {
        let flag = |name: &str| std::env::var_os(name).is_some_and(|v| v == "1");
        let expose = flag("ND_DEBUG_EXPOSE");
        let defer = flag("ND_DEBUG_DEFER_LIVE_LAYOUT");
        gpui_platform::application().run(move |cx: &mut App| {
            let bounds = Bounds { origin: point(px(0.), px(0.)), size: size(px(800.), px(600.)) };
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                app_id: Some("nd-linux-probe".into()),
                window_decorations: Some(REQUESTED_DECORATIONS),
                ..Default::default()
            };
            cx.open_window(options, |_, cx| {
                cx.new(|_| Probe {
                    expose,
                    defer,
                    laid_out: None,
                    pending: false,
                    reported_chrome: String::new(),
                    reported_size: None,
                    first_frame: false,
                })
            })
            .expect("open window");
            cx.observe_keystrokes(|event, _, _| eprintln!("nd: key {}", event.keystroke.unparse())).detach();
            cx.activate(true);
        });
    }
}
