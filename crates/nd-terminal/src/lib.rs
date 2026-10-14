//! The terminal widget (ARCHITECTURE.md 2, M9): libghostty-vt parses the program's output and
//! keeps the screen and scrollback, a pseudo-terminal (or any byte stream) carries the bytes,
//! and a GPU presenter per backend kind draws the grid: GPUI's renderer on the GPUI backend,
//! a Metal layer view on AppKit.
//!
//! The app owns a [`TerminalSession`] and shows it with [`terminal()`]:
//!
//! ```ignore
//! let session = TerminalSession::spawn(&Command::shell(), SessionOptions::default())?;
//! terminal("shell", &session).size_full().on_title_changed(|e, _, _| println!("{}", e.title))
//! ```

#[cfg(target_os = "macos")]
pub mod appkit;
mod element;
pub mod frame;
pub mod glyph;
#[cfg(any(not(target_os = "macos"), feature = "gpui"))]
mod gpui;
pub mod icons;
pub mod input;
mod io;
pub mod links;
mod paint;
mod session;
pub mod sprite;
pub mod theme;

pub use element::{
    Bell, CwdChanged, Exited, FocusChanged, LinkActivated, Notification, SelectionChanged, Terminal, TerminalProps,
    TerminalWidget, TitleChanged, terminal,
};
pub use io::{Command, ExitStatus};
pub use links::Link;
pub use session::{DEFAULT_SCROLLBACK, GridSize, SessionEvent, SessionOptions, TerminalSession, WakerGuard};
pub use theme::{Rgb, TerminalColors, TerminalTheme};

/// Registers the presenters with the backends this build links. `terminal()` calls it; tests
/// that build elements another way call it themselves.
pub fn register_presenters() {
    thread_local!(static DONE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) });
    if DONE.with(|d| d.replace(true)) {
        return;
    }
    #[cfg(any(not(target_os = "macos"), feature = "gpui"))]
    gpui::register();
    #[cfg(target_os = "macos")]
    appkit::register();
}
