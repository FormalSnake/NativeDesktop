//! Every override layer in one window: element style, per platform, native mapping, theme
//! (app, window, subtree) and patterns. Defaults are native; each line below opts out of one.

use nd::prelude::*;

actions!(overrides, [Rename, Share]);

struct Showcase {
    volume: f32,
    sort: usize,
}

impl Render for Showcase {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();

        v_flex()
            .size_full()
            .bg(theme.surface.base)
            // Pattern override: a custom titlebar even on macOS. The traffic lights move to fixed
            // coordinates inside it; on Windows the system caption buttons stay, on Hyprland nothing.
            .child(
                drag_region()
                    .h(px(52.))
                    .px_4()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(window_controls().at(point(px(18.), px(20.))))
                    .child(text("Overrides").font_weight(FontWeight::SEMIBOLD).ml(px(72.)))
                    .child(div().flex_1())
                    .child(
                        // Per platform: glass bezel on macOS, accent style on Windows, pill on Linux.
                        button("share", "Share")
                            .icon(Icon::Share)
                            .dispatch(Share)
                            .on_macos(|b| b.bezel(Bezel::Glass))
                            .on_windows(|b| b.accent())
                            .on_linux(|b| b.rounded_full().px_4())
                            .on_web(|b| b.outline()),
                    ),
            )
            .child(
                v_flex()
                    .p(theme.spacing.content_margin)
                    .gap_4()
                    .max_w(px(560.))
                    // Element style on a native control: tint and font map to bezelColor and NSFont
                    // on macOS; a radius AppKit cannot draw is ignored with a dev warning.
                    .child(
                        button("rename", "Rename…")
                            .dispatch(Rename)
                            .bg(rgb(0x2f6f4f))
                            .text_color(white())
                            .font(theme.fonts.rounded.clone()),
                    )
                    // Mapping override: force the composed (drawn) slider on every OS, for a custom
                    // look the native control cannot express.
                    .child(
                        slider("volume", self.volume)
                            .range(0.0..=1.0)
                            .drawn()
                            .h(px(28.))
                            .rounded_full()
                            .on_change(cx.listener(|this, e: &SliderChanged, _window, cx| {
                                this.volume = e.value;
                                cx.notify();
                            })),
                    )
                    // Mapping override: a picker shown as a segmented control instead of a pop-up.
                    .child(
                        select("sort", ["Name", "Date", "Size"])
                            .selected(self.sort)
                            .native(Mapping::Segmented)
                            .on_change(cx.listener(|this, e: &SelectChanged, _window, cx| {
                                this.sort = e.index;
                                cx.notify();
                            })),
                    )
                    // Subtree theme override: everything inside uses a warm accent and larger radii.
                    .child(
                        glass()
                            .variant(GlassVariant::Clear)
                            .tint(rgba(0xff8a3d33))
                            .rounded_xl()
                            .p_4()
                            .gap_2()
                            .theme(|t| t.accent(rgb(0xff8a3d)).radius_scale(1.5))
                            .child(text("Tinted glass with its own accent"))
                            .child(h_flex().gap_2().child(button("ok", "Accept").primary()).child(button("no", "Decline")))
                            // Pattern override on one element: this context menu is drawn, not an NSMenu.
                            .context_menu(|menu, _window, _cx| menu.item("Rename…", Rename).item("Share", Share))
                            .pattern(Pattern::Drawn),
                    ),
            )
    }
}

fn main() -> nd::Result<()> {
    Application::new(nd::manifest!())
        // App-level theme override, applied on top of the platform preset.
        .theme(ThemeOverride::new().accent(rgb(0x3d6bff)).radius_scale(1.25))
        // App-level pattern override; nativedesktop.toml [patterns] sits below this.
        .patterns(Patterns { tooltips: Pattern::Drawn, ..Patterns::default() })
        .run(|cx| {
            cx.open_window(
                WindowOptions::main("main").titlebar(Titlebar::Custom).size(size(px(640.), px(480.))),
                |window, cx| {
                    // Window-level theme override: this window is always dark.
                    window.set_theme(ThemeOverride::new().appearance(Appearance::Dark), cx);
                    cx.new(|_cx| Showcase { volume: 0.6, sort: 0 })
                },
            )
        })
}
