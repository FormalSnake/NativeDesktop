//! Tier 1: the session over libghostty-vt with no window: screen and scrollback state, colours,
//! wide characters, reflow on resize, selection and copy, links, effects, key and paste
//! encoding.

use nd_terminal::frame::{Attrs, CursorShape};
use nd_terminal::input::{Key, KeyInput, Mods, MouseButton};
use nd_terminal::{GridSize, Rgb, SessionEvent, SessionOptions, TerminalColors, TerminalSession, TerminalTheme};
use nd_theme::{Preset, Scheme, ThemeRegistry};

fn session(cols: u16, rows: u16) -> TerminalSession {
    let s = TerminalSession::detached(SessionOptions { cols, rows, ..SessionOptions::default() });
    // 8 by 16 pixel cells, so mouse positions below are easy to read.
    s.resize(GridSize { cols, rows, cell_width: 8, cell_height: 16 });
    s
}

fn theme(scheme: Scheme) -> TerminalTheme {
    let mut registry = ThemeRegistry::new(Preset::current());
    registry.set_os(nd_theme::OsTheme { scheme: Some(scheme), ..registry.os().clone() });
    TerminalTheme::from_theme(registry.app_theme(), &TerminalColors::default())
}

mod gate_m9_terminal {
    use super::*;

    #[test]
    fn prints_text_and_moves_the_cursor() {
        let s = session(20, 4);
        s.feed(b"hello\r\nworld");
        assert_eq!(s.screen_text(), "hello\nworld");
        s.frame(|f| {
            let cursor = f.cursor.expect("cursor visible");
            assert_eq!((cursor.col, cursor.row), (5, 1));
            assert_eq!(cursor.shape, CursorShape::BlockHollow, "unfocused terminals hollow the cursor");
        });
        s.set_focused(true);
        s.frame(|f| assert_eq!(f.cursor.unwrap().shape, CursorShape::Block));
        s.feed(b"\x1b[2J\x1b[H\x1b[?25l");
        s.frame(|f| assert!(f.cursor.is_none(), "DECTCEM off hides the cursor"));
    }

    #[test]
    fn resolves_colours_through_the_theme() {
        let s = session(30, 3);
        let t = theme(Scheme::Dark);
        s.set_theme(&t);
        s.feed(b"\x1b[31mred\x1b[0m \x1b[1;32mbold\x1b[0m \x1b[38;2;1;2;3mrgb\x1b[0m \x1b[7minv\x1b[0m plain");
        s.frame(|f| {
            let row = &f.rows[0];
            let at = |col: usize| row.cells[col];
            assert_eq!(at(0).fg, Some(t.ansi_rgb[1]), "SGR 31 is the theme's red");
            assert_eq!(t.ink(at(0).fg.unwrap()), t.ansi[1], "and draws with the token itself");
            assert!(at(4).attrs.contains(Attrs::BOLD));
            assert_eq!(at(4).fg, Some(t.ansi_rgb[2]));
            assert_eq!(at(9).fg, Some(Rgb::new(1, 2, 3)));
            assert_eq!(at(13).fg, Some(t.background_rgb), "inverse swaps the defaults");
            assert_eq!(at(13).bg, Some(t.foreground_rgb));
            assert_eq!(at(17).fg, None, "default colours stay unresolved for the token");
            assert_eq!(f.background, t.background_rgb);
        });
    }

    #[test]
    fn wide_characters_take_two_cells() {
        let s = session(20, 2);
        s.feed("a你好b\u{1f600}c".as_bytes());
        s.frame(|f| {
            let row = &f.rows[0];
            assert_eq!(row.grapheme(&row.cells[1]), "你");
            assert!(row.cells[1].attrs.contains(Attrs::WIDE));
            assert!(row.cells[2].attrs.contains(Attrs::SPACER));
            assert_eq!(row.grapheme(&row.cells[3]), "好");
            assert_eq!(row.grapheme(&row.cells[5]), "b");
            assert_eq!(row.grapheme(&row.cells[6]), "\u{1f600}");
            assert!(row.cells[6].attrs.contains(Attrs::WIDE));
            assert_eq!(row.grapheme(&row.cells[8]), "c");
            assert_eq!(row.plain_text(), "a你好b\u{1f600}c");
        });
    }

    #[test]
    fn keeps_scrollback_and_scrolls_through_it() {
        let s = TerminalSession::detached(SessionOptions { cols: 20, rows: 5, scrollback: 1000 });
        for i in 0..100 {
            s.feed(format!("line {i}\r\n").as_bytes());
        }
        assert_eq!(s.scrollback_rows(), 96);
        assert_eq!(s.frame(|f| f.row_text(0)), "line 96");
        s.scroll_to_top();
        assert_eq!(s.frame(|f| f.row_text(0)), "line 0");
        s.scroll_rows(10);
        assert_eq!(s.frame(|f| f.row_text(0)), "line 10");
        s.frame(|f| assert_eq!((f.scroll_offset, f.scroll_total), (10, 101)));
        // Typing returns to the bottom.
        s.write(b"x");
        assert_eq!(s.frame(|f| f.row_text(0)), "line 96");
    }

    #[test]
    fn reflows_on_resize() {
        let s = session(10, 4);
        s.feed(b"abcdefghijklmnopqrstuvwxyz");
        assert_eq!(s.screen_text(), "abcdefghij\nklmnopqrst\nuvwxyz");
        s.resize(GridSize { cols: 30, rows: 4, cell_width: 8, cell_height: 16 });
        assert_eq!(s.screen_text(), "abcdefghijklmnopqrstuvwxyz", "soft wraps join when the grid widens");
        s.resize(GridSize { cols: 13, rows: 4, cell_width: 8, cell_height: 16 });
        assert_eq!(s.screen_text(), "abcdefghijklm\nnopqrstuvwxyz");
        // A hard line break never joins.
        let s = session(10, 4);
        s.feed(b"abc\r\ndef");
        s.resize(GridSize { cols: 30, rows: 4, cell_width: 8, cell_height: 16 });
        assert_eq!(s.screen_text(), "abc\ndef");
    }

    #[test]
    fn selects_with_the_mouse_and_copies() {
        let s = session(20, 3);
        s.feed(b"first line\r\nsecond line");
        // Cells are 8 by 16: press on "line" (col 6, row 0), drag to col 5 of row 1.
        s.mouse_down(MouseButton::Left, 6. * 8. + 1., 4., Mods::empty());
        s.mouse_moved(5. * 8. + 7., 20., Some(MouseButton::Left), Mods::empty());
        s.mouse_up(MouseButton::Left, 5. * 8. + 7., 20., Mods::empty());
        assert_eq!(s.selection_text().as_deref(), Some("line\nsecond"));
        assert!(s.frame(|f| f.rows[0].cells[6].attrs.contains(Attrs::SELECTED)));
        assert!(s.take_events().contains(&SessionEvent::SelectionChanged(true)));
        s.select_all();
        assert_eq!(s.selection_text().as_deref(), Some("first line\nsecond line"));
        s.clear_selection();
        assert!(!s.has_selection());
        assert!(s.take_events().contains(&SessionEvent::SelectionChanged(false)));
    }

    #[test]
    fn double_click_selects_a_word() {
        let s = session(30, 2);
        s.feed(b"echo hello-world done");
        let (x, y) = (8. * 7. + 2., 4.);
        s.mouse_down(MouseButton::Left, x, y, Mods::empty());
        s.mouse_up(MouseButton::Left, x, y, Mods::empty());
        s.mouse_down(MouseButton::Left, x, y, Mods::empty());
        s.mouse_up(MouseButton::Left, x, y, Mods::empty());
        let word = s.selection_text().unwrap_or_default();
        assert!(word == "hello-world" || word == "hello", "a word around the click, got {word:?}");
    }

    #[test]
    fn programs_tracking_the_mouse_get_reports_not_selections() {
        let s = session(20, 3);
        s.feed(b"\x1b[?1000h\x1b[?1006h");
        s.take_output();
        s.mouse_down(MouseButton::Left, 2. * 8. + 1., 1. * 16. + 1., Mods::empty());
        s.mouse_up(MouseButton::Left, 2. * 8. + 1., 1. * 16. + 1., Mods::empty());
        assert_eq!(s.take_output(), b"\x1b[<0;3;2M\x1b[<0;3;2m");
        assert!(!s.has_selection());
        // Shift selects anyway.
        s.feed(b"some text");
        s.mouse_down(MouseButton::Left, 1., 1., Mods::SHIFT);
        s.mouse_moved(3. * 8. + 7., 1., Some(MouseButton::Left), Mods::SHIFT);
        s.mouse_up(MouseButton::Left, 3. * 8. + 7., 1., Mods::SHIFT);
        assert!(s.has_selection());
    }

    #[test]
    fn finds_urls_and_osc8_links() {
        let s = session(60, 3);
        s.feed(b"open https://example.com/docs. now\r\n");
        s.feed(b"\x1b]8;;https://ghostty.org\x1b\\here\x1b]8;;\x1b\\ there");
        let link = s.link_at(10, 0).expect("plain URL");
        assert_eq!((link.url.as_str(), link.start, link.end), ("https://example.com/docs", 5, 29));
        assert!(s.link_at(2, 0).is_none());
        let link = s.link_at(1, 1).expect("OSC 8 link");
        assert_eq!(link.url, "https://ghostty.org");
        assert!(s.frame(|f| f.rows[1].cells[0].attrs.contains(Attrs::LINK)));
        assert!(s.link_at(6, 1).is_none());
    }

    #[test]
    fn reports_title_cwd_bell_and_notifications() {
        let s = session(20, 3);
        s.feed(b"\x1b]0;my title\x07\x1b]7;file://host/Users/me/My%20Dir\x07\x07\x1b]9;done\x07");
        let events = s.take_events();
        assert!(events.contains(&SessionEvent::TitleChanged("my title".into())), "{events:?}");
        assert!(events.contains(&SessionEvent::CwdChanged("/Users/me/My Dir".into())), "{events:?}");
        assert!(events.contains(&SessionEvent::Bell), "{events:?}");
        assert!(
            events.iter().any(|e| matches!(e, SessionEvent::Notification { body, .. } if body == "done")),
            "{events:?}"
        );
        assert_eq!(s.title(), "my title");
        assert_eq!(s.cwd(), "/Users/me/My Dir");
    }

    #[test]
    fn answers_queries() {
        let s = session(20, 3);
        s.feed(b"\x1b[c");
        let da = String::from_utf8(s.take_output()).unwrap();
        assert!(da.starts_with("\x1b[?62;"), "DA1 reports VT220, got {da:?}");
        s.feed(b"\x1b[6n");
        assert_eq!(s.take_output(), b"\x1b[1;1R", "cursor position report");
        s.set_theme(&theme(Scheme::Dark));
        s.feed(b"\x1b[?996n");
        assert_eq!(s.take_output(), b"\x1b[?997;1n", "dark scheme report");
        s.feed(b"\x1b[18t");
        assert_eq!(s.take_output(), b"\x1b[8;3;20t", "text area size in cells");
    }

    #[test]
    fn encodes_keys_for_the_mode_in_effect() {
        let s = session(20, 3);
        assert!(s.key(&KeyInput::press(Key::ArrowUp, Mods::empty())));
        assert_eq!(s.take_output(), b"\x1b[A");
        s.feed(b"\x1b[?1h");
        s.key(&KeyInput::press(Key::ArrowUp, Mods::empty()));
        assert_eq!(s.take_output(), b"\x1bOA", "application cursor keys");
        s.key(&KeyInput::typed(Key::A, Mods::SHIFT, "A"));
        assert_eq!(s.take_output(), b"A");
        s.key(&KeyInput::press(Key::C, Mods::CTRL));
        assert_eq!(s.take_output(), b"\x03");
        s.key(&KeyInput::press(Key::Escape, Mods::empty()));
        assert_eq!(s.take_output(), b"\x1b");
        // Kitty keyboard protocol: the program pushes "disambiguate escape codes".
        s.feed(b"\x1b[>1u");
        assert_eq!(s.kitty_flags().bits(), 1);
        s.key(&KeyInput::press(Key::Escape, Mods::empty()));
        assert_eq!(s.take_output(), b"\x1b[27u");
        s.key(&KeyInput::press(Key::C, Mods::CTRL));
        assert_eq!(s.take_output(), b"\x1b[99;5u");
    }

    #[test]
    fn pastes_bracketed_when_asked() {
        let s = session(20, 3);
        s.paste("one\ntwo");
        assert_eq!(s.take_output(), b"one\rtwo");
        s.feed(b"\x1b[?2004h");
        s.paste("one\ntwo\x1b[201~");
        let out = s.take_output();
        assert!(out.starts_with(b"\x1b[200~one"), "{out:?}");
        assert!(out.ends_with(b"\x1b[201~"));
        assert_eq!(out.windows(6).filter(|w| w == b"\x1b[201~").count(), 1, "the terminator inside is defused");
    }

    #[test]
    fn reports_focus_when_asked() {
        let s = session(20, 3);
        s.set_focused(true);
        assert!(s.take_output().is_empty());
        s.feed(b"\x1b[?1004h");
        s.set_focused(false);
        assert_eq!(s.take_output(), b"\x1b[O");
        s.set_focused(true);
        assert_eq!(s.take_output(), b"\x1b[I");
    }

    #[test]
    fn only_dirty_rows_are_rebuilt() {
        let s = session(20, 4);
        s.feed(b"a\r\nb\r\nc");
        let first = s.frame(|f| f.generation);
        assert_eq!(s.frame(|f| f.generation), first, "a clean terminal keeps its frame");
        s.feed(b"\r\x1b[Kd");
        s.frame(|f| {
            assert_eq!(f.generation, first + 1);
            assert_eq!(f.text(), "a\nb\nd");
        });
    }
}
