//! The terminal widget (ARCHITECTURE.md 2, M9): libghostty-vt parses the program's output and
//! keeps the screen and scrollback, a pseudo-terminal (or any byte stream) carries the bytes,
//! and a GPU presenter per backend kind draws the grid.
//!
//! The app owns a [`TerminalSession`] and shows it with [`terminal()`]:
//!
//! ```ignore
//! let session = TerminalSession::spawn(&Command::shell(), SessionOptions::default())?;
//! terminal("shell", &session).on_title_changed(|e, _, _| println!("{}", e.title))
//! ```

pub mod frame;
pub mod input;
mod io;
pub mod links;
mod session;
pub mod theme;

pub use io::{Command, ExitStatus};
pub use session::{DEFAULT_SCROLLBACK, GridSize, SessionEvent, SessionOptions, TerminalSession, WakerGuard};
pub use theme::{Rgb, TerminalColors, TerminalTheme};
