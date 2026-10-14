//! Tier 2 on the AppKit presenter's Metal renderer, offscreen: the `printf` golden in light and
//! dark, drawn by the same renderer the view presents with.

#![cfg(target_os = "macos")]

mod support;

use std::time::{Duration, Instant};

use nd_terminal::{SessionOptions, TerminalSession};
use nd_theme::{OsTheme, Preset, Scheme, ThemeRegistry};

fn wait_for_exit(session: &TerminalSession) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !(session.is_closed() && session.exit_status().is_some()) {
        assert!(Instant::now() < deadline, "printf did not finish:\n{}", session.screen_text());
        session.pump();
        std::thread::sleep(Duration::from_millis(10));
    }
    while session.pump() {}
}

mod gate_m9_terminal {
    use super::*;

    pub fn appkit_printf_golden() {
        let mut failed = Vec::new();
        for scheme in [Scheme::Light, Scheme::Dark] {
            let mut registry = ThemeRegistry::new(Preset::current());
            registry.set_os(OsTheme { scheme: Some(scheme), ..registry.os().clone() });
            let session = TerminalSession::spawn(&support::printf_command(), SessionOptions::default()).expect("spawn");
            wait_for_exit(&session);
            let mut surface =
                nd_terminal::appkit::Offscreen::new(registry.app_theme(), 13., 6., (720., 140.), 2.).expect("Metal");
            surface.fit(&session);
            let (w, h, rgba) = surface.image(&session);
            let image = image::RgbaImage::from_raw(w, h, rgba).expect("sized");
            let name = format!("printf-appkit-{}-macos", if scheme.is_dark() { "dark" } else { "light" });
            if let Err(e) = support::compare(&name, &image) {
                failed.push(e);
            }
        }
        assert!(failed.is_empty(), "{failed:#?}");
    }
}

fn main() {
    support::run(&[("gate_m9_terminal::appkit_printf_golden", gate_m9_terminal::appkit_printf_golden)]);
}
