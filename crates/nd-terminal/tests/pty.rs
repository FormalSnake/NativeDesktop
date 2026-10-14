//! Tier 1 with a real pseudo-terminal: input reaches the child and its echo comes back,
//! `printf` output with colours and wide characters lands in the grid, a resize reaches the
//! child, and its exit is reported.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use nd_terminal::frame::Attrs;
use nd_terminal::input::{Key, KeyInput, Mods};
use nd_terminal::{Command, GridSize, SessionEvent, SessionOptions, TerminalSession};

/// Pumps the session as a presenter would, on each wake, until `done` or the deadline.
fn pump_until(s: &TerminalSession, what: &str, mut done: impl FnMut(&TerminalSession) -> bool) {
    let (tx, rx) = mpsc::channel();
    let _guard = s.on_output(move || {
        let _ = tx.send(());
    });
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        while s.pump() {}
        if done(s) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {what} (exit {:?}, closed {}); screen:\n{}",
            s.exit_status(),
            s.is_closed(),
            s.screen_text()
        );
        let _ = rx.recv_timeout(Duration::from_millis(50));
    }
}

fn spawn(command: Command) -> TerminalSession {
    let s = TerminalSession::spawn(&command, SessionOptions { cols: 40, rows: 10, ..SessionOptions::default() })
        .expect("spawn");
    s.resize(GridSize { cols: 40, rows: 10, cell_width: 8, cell_height: 16 });
    s
}

mod gate_m9_terminal {
    use super::*;

    #[test]
    fn typed_keys_round_trip_through_cat() {
        let s = spawn(Command::new("cat"));
        for c in "hello".chars() {
            let key = nd_terminal::input::key_for_char(c).unwrap();
            assert!(s.key(&KeyInput::typed(key, Mods::empty(), c.to_string())));
        }
        assert!(s.key(&KeyInput::press(Key::Enter, Mods::empty())));
        // The line discipline echoes "hello", then cat writes it back.
        pump_until(&s, "the echo and cat's copy", |s| s.screen_text() == "hello\nhello");
        // Ctrl-D at the start of a line closes cat's input; it exits.
        s.key(&KeyInput::press(Key::D, Mods::CTRL));
        pump_until(&s, "cat to exit", |s| s.exit_status().is_some());
        assert!(s.exit_status().unwrap().success());
        assert!(s.take_events().iter().any(|e| matches!(e, SessionEvent::Exited(status) if status.success())));
    }

    #[test]
    fn printf_output_lands_in_the_grid() {
        let s = spawn(
            Command::new("printf")
                .arg("\\033[31mred\\033[0m \\033[1;34mbold\\033[0m 你好 \u{e0a0} \\033[4mu\\033[0m\\n"),
        );
        pump_until(&s, "printf to exit", |s| s.exit_status().is_some() && s.is_closed());
        s.frame(|f| {
            let row = &f.rows[0];
            assert_eq!(row.plain_text(), "red bold 你好 \u{e0a0} u");
            assert!(row.cells[0].fg.is_some(), "red is a palette colour");
            assert!(row.cells[4].attrs.contains(Attrs::BOLD));
            assert!(row.cells[9].attrs.contains(Attrs::WIDE));
            assert_eq!(row.grapheme(&row.cells[14]), "\u{e0a0}");
            assert_ne!(row.cells[16].underline, nd_terminal::frame::Underline::None);
        });
    }

    #[test]
    fn a_resize_reaches_the_child() {
        let s = spawn(Command::new("sh").arg("-c").arg("stty size; read x; stty size"));
        pump_until(&s, "the first size", |s| s.screen_text().contains("10 40"));
        s.resize(GridSize { cols: 50, rows: 12, cell_width: 8, cell_height: 16 });
        s.write(b"\r");
        pump_until(&s, "the second size", |s| s.screen_text().contains("12 50"));
    }

    #[test]
    fn reports_a_failing_exit() {
        let s = spawn(Command::new("sh").arg("-c").arg("exit 3"));
        pump_until(&s, "the exit", |s| s.exit_status().is_some());
        assert_eq!(s.exit_status().unwrap().code, 3);
    }
}
