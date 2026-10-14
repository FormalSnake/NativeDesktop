//! Tier 2: the appearance audit over the terminal window, light, dark and matugen, on AppKit and
//! GPUI. The tab shows a fixed transcript (fed, not spawned, so every backend audits the same
//! cells): the default colours, the ANSI colours the theme's tokens supply, bold and the cursor.

use nd_terminal::{Command, SessionOptions, TerminalSession};

const TRANSCRIPT: &[u8] = b"\x1b[1;34m~/src/nd\x1b[0m \x1b[32m$\x1b[0m cargo test\r\n\
    \x1b[32m   Compiling\x1b[0m nd-terminal\r\n\
    test vt::wraps ... \x1b[32mok\x1b[0m\r\n\
    test pty::echo ... \x1b[31mFAILED\x1b[0m\r\n\
    \x1b[1;34m~/src/nd\x1b[0m \x1b[32m$\x1b[0m ";

fn main() {
    nd::audit::Audit::new("terminal", terminal::app)
        .window("main", |cx| {
            let session = TerminalSession::detached(SessionOptions::default());
            session.feed(TRANSCRIPT);
            terminal::open_with(cx, Command::shell(), vec![session]).id()
        })
        .run()
}
