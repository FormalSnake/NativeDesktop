//! Tier 2 on the GPUI presenter, in GPUI's headless app with the platform's text system and
//! renderer: the `printf` golden in light and dark, keys typed through the window's own key
//! path reaching `cat` and echoing back, and a click focusing the terminal.

mod support;

use std::sync::Arc;
use std::time::{Duration, Instant};

use nd_backend_gpui::GpuiHost;
use nd_gpui::gpui::HeadlessAppContext;
use nd_terminal::{Command, SessionOptions, TerminalSession, terminal};
use nd_theme::{OsTheme, Preset, Scheme, ThemeRegistry};
use nd_ui::prelude::*;
use nd_ui::semantics::{MouseButton, PointerPhase, SyntheticInput};
use nd_ui::{Keystroke, WindowId};

struct Shell {
    session: TerminalSession,
}

impl Render for Shell {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(terminal("term", &self.session).size_full().font_size(13.))
    }
}

struct Opened {
    cx: HeadlessAppContext,
    host: GpuiHost,
    window: WindowId,
}

fn open(session: &TerminalSession, scheme: Scheme, width: f32, height: f32) -> Opened {
    let text = nd_gpui::gpui_platform::current_platform(true).text_system();
    let mut cx =
        HeadlessAppContext::with_platform(text, Arc::new(()), nd_gpui::gpui_platform::current_headless_renderer);
    let session = session.clone();
    let slot: std::rc::Rc<std::cell::Cell<Option<WindowId>>> = Default::default();
    let s = slot.clone();
    let host = cx.update(|cx| {
        GpuiHost::builder().quit_when_empty(false).start(cx, move |app| {
            let handle = app.open_window(WindowOptions::main("term").size(size(px(width), px(height))), |_, cx| {
                cx.new(|_| Shell { session })
            });
            s.set(Some(handle.id()));
        })
    });
    let mut registry = ThemeRegistry::new(Preset::current());
    registry.set_os(OsTheme { scheme: Some(scheme), ..registry.os().clone() });
    host.backend().set_theme(registry);
    let window = slot.get().expect("window opened");
    let mut o = Opened { cx, host, window };
    settle(&mut o);
    o
}

fn settle(o: &mut Opened) {
    for _ in 0..4 {
        o.cx.run_until_parked();
        o.cx.update(|cx| {
            o.host.pump(cx);
            o.host.draw(cx);
        });
    }
    o.cx.run_until_parked();
}

/// Settles until `done`, giving the I/O threads real time between rounds.
fn settle_until(o: &mut Opened, what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        settle(o);
        if done() {
            return;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn spawn(command: &Command) -> TerminalSession {
    TerminalSession::spawn(command, SessionOptions::default()).expect("spawn")
}

#[cfg_attr(not(unix), allow(dead_code))]
mod gate_m9_terminal {
    use super::*;

    pub fn gpui_printf_golden() {
        let mut failed = Vec::new();
        for scheme in [Scheme::Light, Scheme::Dark] {
            let session = spawn(&support::printf_command());
            let mut o = open(&session, scheme, 720., 140.);
            settle_until(&mut o, "printf's output", || session.is_closed() && session.exit_status().is_some());
            let text = session.screen_text();
            assert!(text.contains("wide: 你好世界"), "{text}");
            let handle = o.host.window_handle(o.window).expect("GPUI window");
            let image = o.cx.capture_screenshot(handle).expect("capture");
            let name = format!("printf-gpui-{}", if scheme.is_dark() { "dark" } else { "light" });
            if let Err(e) = support::compare(&name, &image) {
                failed.push(e);
            }
        }
        assert!(failed.is_empty(), "{failed:#?}");
    }

    pub fn gpui_keys_round_trip_through_cat() {
        let session = spawn(&Command::new("cat"));
        let mut o = open(&session, Scheme::Light, 480., 160.);
        let window = o.window;
        // A click gives the terminal GPUI focus, where its keys and text input arrive.
        o.cx.update(|cx| {
            o.host.update(cx, |app| {
                for phase in [PointerPhase::Down, PointerPhase::Up] {
                    let at = nd_ui::Point { x: 40., y: 30. };
                    app.synthesize(
                        window,
                        &SyntheticInput::Pointer { phase, at, button: MouseButton::Left, click_count: 1 },
                    )
                    .expect("posted");
                }
            })
        });
        settle(&mut o);
        assert!(session.is_focused(), "the click focused the terminal");
        let handle = o.host.window_handle(window).expect("GPUI window");
        for key in ["h", "i", "enter"] {
            let keystroke = Keystroke::parse(key).expect("keystroke");
            // The platform's path: dispatch, then text to the focused input handler.
            let _ = o.cx.update(|cx| handle.update(cx, |_, window, cx| window.dispatch_keystroke(keystroke, cx)));
        }
        settle_until(&mut o, "cat's echo", || session.screen_text() == "hi\nhi");
    }
}

fn main() {
    // Both drive POSIX tools (`printf`, `cat`); the ConPTY path is covered by `pty.rs` on Windows.
    #[cfg(unix)]
    support::run(&[
        ("gate_m9_terminal::gpui_printf_golden", gate_m9_terminal::gpui_printf_golden),
        ("gate_m9_terminal::gpui_keys_round_trip_through_cat", gate_m9_terminal::gpui_keys_round_trip_through_cat),
    ]);
    #[cfg(not(unix))]
    support::run(&[]);
}
