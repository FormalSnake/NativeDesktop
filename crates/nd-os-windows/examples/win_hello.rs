//! `examples/hello` on Windows until nd-backend-gpui hosts it (the M2 GPUI lane): the same counter
//! drawn through GPUI, with this crate's chrome on the HWND. Mica behind the window, a 32 px
//! Fluent title row that drags, app-drawn caption buttons answering the hit test as the system's
//! (so the maximize button opens Snap Layouts), and pen input logged from `WM_POINTER`.
//!
//! Debug switches, as on the AppKit host:
//! - `ND_DEBUG_EXPOSE=1` paints the content opaque with a 3 px green edge along its right and
//!   bottom, which the resize gate looks for in every captured frame.
//! - `ND_DEBUG_DEFER_LIVE_LAYOUT=1` sizes the content from the previous frame's viewport, which
//!   the gate's control run must catch.
//!
//! stderr: `nd: window hwnd=<n> scale=<s>`, `nd: count <n>`, `nd: pen <phase> pressure=<p>`.

#[cfg(windows)]
fn main() {
    app::main();
}

#[cfg(not(windows))]
fn main() {
    eprintln!("win_hello runs on Windows only");
}

#[cfg(windows)]
mod app {
    use nd_gpui::gpui::{
        App, AppContext, Bounds, Context, FontWeight, InteractiveElement, IntoElement, ParentElement, Pixels, Render,
        Size, StatefulInteractiveElement, Styled, TitlebarOptions, Window, WindowAppearance,
        WindowBackgroundAppearance, WindowBounds, WindowOptions, div, px, rgb, rgba, size,
    };
    use nd_gpui::gpui_platform;
    use nd_os_windows::backdrop::{self, Backdrop};
    use nd_os_windows::caption::{CaptionButton, CaptionLayout, FLUENT_BUTTON_WIDTH, FLUENT_TITLE_HEIGHT};
    use nd_os_windows::chrome::{Chrome, ChromeEvent};
    use nd_os_windows::pointer::PointerPhase;

    const EDGE: f32 = 3.;

    struct Hello {
        count: u32,
        chrome: Option<Chrome>,
        expose: bool,
        defer: bool,
        last_viewport: Option<Size<Pixels>>,
    }

    fn env_on(name: &str) -> bool {
        std::env::var_os(name).is_some_and(|v| v == "1")
    }

    impl Hello {
        fn caption_button(&self, which: CaptionButton, dark: bool, maximized: bool) -> impl IntoElement {
            let state = self.chrome.as_ref().map(Chrome::caption_state).unwrap_or_default();
            let hovered = state.hovered == Some(which);
            let pressed = state.pressed == Some(which);
            // Fluent: subtle fill on hover and press; close turns red.
            let (bg, fg) = match (which, pressed, hovered) {
                (CaptionButton::Close, true, _) => (rgba(0xc42b1ce6), rgb(0xffffff)),
                (CaptionButton::Close, false, true) => (rgba(0xc42b1cff), rgb(0xffffff)),
                (_, true, _) if dark => (rgba(0xffffff0b), rgb(0xffffff)),
                (_, true, _) => (rgba(0x00000006), rgb(0x000000)),
                (_, false, true) if dark => (rgba(0xffffff0f), rgb(0xffffff)),
                (_, false, true) => (rgba(0x00000009), rgb(0x000000)),
                _ if dark => (rgba(0x00000000), rgb(0xffffff)),
                _ => (rgba(0x00000000), rgb(0x000000)),
            };
            let glyph = match which {
                CaptionButton::Minimize => "\u{E921}",
                CaptionButton::Maximize if maximized => "\u{E923}",
                CaptionButton::Maximize => "\u{E922}",
                CaptionButton::Close => "\u{E8BB}",
            };
            div()
                .w(px(FLUENT_BUTTON_WIDTH))
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(bg)
                .text_color(fg)
                .font_family("Segoe Fluent Icons")
                .text_size(px(10.))
                .child(glyph)
        }
    }

    impl Render for Hello {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let viewport = window.viewport_size();
            let content = if self.defer { self.last_viewport.replace(viewport).unwrap_or(viewport) } else { viewport };
            let maximized = window.is_maximized();
            if let Some(chrome) = &self.chrome {
                chrome.set_layout(CaptionLayout::fluent(f32::from(viewport.width), FLUENT_TITLE_HEIGHT));
            }
            let dark = matches!(window.appearance(), WindowAppearance::Dark | WindowAppearance::VibrantDark);
            let fg = if dark { rgb(0xffffff) } else { rgb(0x1a1a1a) };
            let secondary = if dark { rgba(0xffffffc8) } else { rgba(0x000000a0) };

            let title_row = div()
                .h(px(FLUENT_TITLE_HEIGHT))
                .w_full()
                .flex()
                .items_center()
                .child(div().pl(px(16.)).text_size(px(12.)).text_color(fg).child("Hello"))
                .child(div().flex_1())
                .child(
                    div()
                        .flex()
                        .h_full()
                        .child(self.caption_button(CaptionButton::Minimize, dark, maximized))
                        .child(self.caption_button(CaptionButton::Maximize, dark, maximized))
                        .child(self.caption_button(CaptionButton::Close, dark, maximized)),
                );

            let add = div()
                .id("add")
                .px(px(12.))
                .py(px(6.))
                .rounded(px(4.))
                .bg(if dark { rgba(0xffffff0f) } else { rgba(0xffffffb3) })
                .border_1()
                .border_color(if dark { rgba(0xffffff17) } else { rgba(0x0000000f) })
                .text_size(px(14.))
                .text_color(fg)
                .child("Add one")
                .on_click(cx.listener(|this, _, _, cx| {
                    this.count += 1;
                    eprintln!("nd: count {}", this.count);
                    cx.notify();
                }));

            let body = div()
                .flex_1()
                .w_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(8.))
                .child(add)
                .child(
                    div()
                        .text_size(px(48.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(fg)
                        .child(self.count.to_string()),
                )
                .child(div().text_size(px(14.)).text_color(secondary).child("clicks"));

            let mut root = div().size_full().relative().flex().flex_col().child(title_row).child(body);
            if self.expose {
                let (w, h) = (f32::from(content.width), f32::from(content.height));
                root = root
                    .bg(rgb(0x202020))
                    .child(
                        div()
                            .absolute()
                            .left(px(w - EDGE))
                            .top(px(FLUENT_TITLE_HEIGHT))
                            .w(px(EDGE))
                            .h(px(h))
                            .bg(rgb(0x00ff00)),
                    )
                    .child(div().absolute().left(px(0.)).top(px(h - EDGE)).w(px(w)).h(px(EDGE)).bg(rgb(0x00ff00)));
            }
            root
        }
    }

    pub fn main() {
        let expose = env_on("ND_DEBUG_EXPOSE");
        let defer = env_on("ND_DEBUG_DEFER_LIVE_LAYOUT");
        gpui_platform::application().run(move |cx: &mut App| {
            let bounds = Bounds::centered(None, size(px(640.), px(400.)), cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("Hello".into()),
                    appears_transparent: true,
                    ..Default::default()
                }),
                // Any non-opaque appearance makes GPUI clear to transparent; the backdrop itself is set below.
                window_background: WindowBackgroundAppearance::MicaBackdrop,
                ..Default::default()
            };
            cx.open_window(options, |window, cx| {
                let chrome = Chrome::install(window, CaptionLayout::fluent(640., FLUENT_TITLE_HEIGHT));
                let entity = cx.new(|_| Hello { count: 0, chrome: None, expose, defer, last_viewport: None });
                match chrome {
                    Ok(chrome) => {
                        let hwnd = chrome.hwnd();
                        let dark = backdrop::apps_use_dark();
                        if let Err(e) = backdrop::apply(hwnd, Backdrop::Mica, dark) {
                            eprintln!("nd: backdrop unavailable: {e}");
                        }
                        eprintln!("nd: window hwnd={} scale={}", hwnd.0 as isize, chrome.scale());
                        let weak = entity.downgrade();
                        let async_cx = cx.to_async();
                        chrome.on_event(move |event| {
                            if let ChromeEvent::Pointer(p) = event
                                && p.pen.is_some()
                                && p.phase != PointerPhase::Update
                            {
                                let pressure = p.pen.and_then(|pen| pen.pressure).unwrap_or(-1.);
                                eprintln!("nd: pen {:?} pressure={pressure:.2} at {:.0},{:.0}", p.phase, p.x, p.y);
                            }
                            let weak = weak.clone();
                            // Runs on the next turn of GPUI's loop: the window procedure may be inside one of GPUI's own calls.
                            async_cx
                                .spawn(async move |cx| {
                                    let _ = weak.update(cx, |_, cx| cx.notify());
                                })
                                .detach();
                        });
                        entity.update(cx, |hello, _| hello.chrome = Some(chrome));
                    }
                    Err(e) => eprintln!("nd: chrome unavailable: {e}"),
                }
                entity
            })
            .expect("open window");
            cx.activate(true);
        });
    }
}
