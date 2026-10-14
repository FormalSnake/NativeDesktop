//! GPUI input in Chromium's terms on Linux: Windows virtual key codes (which Chromium's
//! off-screen key events are keyed by), XKB keycodes for the DOM `code` of a US layout, modifier
//! flags, and wheel deltas.

use nd_gpui::gpui::{Keystroke, Modifiers, ScrollDelta};

use crate::keys::flag;

/// Pixels a wheel notch scrolls, as Chromium's GTK port counts it.
const PIXELS_PER_LINE: f32 = 40.;

pub(crate) fn modifiers(m: &Modifiers) -> u32 {
    let mut out = 0;
    for (on, bit) in
        [(m.shift, flag::SHIFT), (m.control, flag::CONTROL), (m.alt, flag::ALT), (m.platform, flag::COMMAND)]
    {
        if on {
            out |= bit;
        }
    }
    out
}

/// Wheel deltas in pixels: touchpads report pixels, wheels report lines.
pub(crate) fn wheel_delta(delta: &ScrollDelta) -> (i32, i32, bool) {
    match delta {
        ScrollDelta::Pixels(p) => (p.x.as_f32().round() as i32, p.y.as_f32().round() as i32, true),
        ScrollDelta::Lines(l) => ((l.x * PIXELS_PER_LINE).round() as i32, (l.y * PIXELS_PER_LINE).round() as i32, false),
    }
}

/// The Windows virtual key code of a GPUI key name, as `ui::KeyboardCode` numbers it. Shifted
/// symbols name the key they are on.
pub(crate) fn virtual_key(key: &str) -> i32 {
    let mut chars = key.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        return match c.to_ascii_lowercase() {
            c @ 'a'..='z' => c.to_ascii_uppercase() as i32,
            c @ '0'..='9' => c as i32,
            ')' => 0x30,
            '!' => 0x31,
            '@' => 0x32,
            '#' => 0x33,
            '$' => 0x34,
            '%' => 0x35,
            '^' => 0x36,
            '&' => 0x37,
            '*' => 0x38,
            '(' => 0x39,
            ';' | ':' => 0xBA,
            '=' | '+' => 0xBB,
            ',' | '<' => 0xBC,
            '-' | '_' => 0xBD,
            '.' | '>' => 0xBE,
            '/' | '?' => 0xBF,
            '`' | '~' => 0xC0,
            '[' | '{' => 0xDB,
            '\\' | '|' => 0xDC,
            ']' | '}' => 0xDD,
            '\'' | '"' => 0xDE,
            _ => 0,
        };
    }
    match key {
        "backspace" => 0x08,
        "tab" => 0x09,
        "enter" => 0x0D,
        "escape" => 0x1B,
        "space" => 0x20,
        "pageup" => 0x21,
        "pagedown" => 0x22,
        "end" => 0x23,
        "home" => 0x24,
        "left" => 0x25,
        "up" => 0x26,
        "right" => 0x27,
        "down" => 0x28,
        "insert" => 0x2D,
        "delete" => 0x2E,
        "back" => 0xA6,
        "forward" => 0xA7,
        "menu" => 0x5D,
        _ => match key.strip_prefix('f').and_then(|n| n.parse::<i32>().ok()) {
            Some(n @ 1..=24) => 0x6F + n,
            _ => 0,
        },
    }
}

/// The XKB keycode (evdev + 8) of the key on a US layout, from which Chromium derives the DOM
/// `code`; 0 when unknown.
pub(crate) fn native_key(key: &str) -> i32 {
    const ROWS: [(&str, i32); 4] = [("1234567890-=", 2), ("qwertyuiop[]", 16), ("asdfghjkl;'`", 30), ("zxcvbnm,./", 44)];
    const SHIFTED: [(&str, &str); 4] =
        [("!@#$%^&*()_+", "1234567890-="), ("{}", "[]"), (":\"~", ";'`"), ("<>?", ",./")];
    let mut chars = key.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        let c = c.to_ascii_lowercase();
        let c = SHIFTED.iter().find_map(|(from, to)| from.find(c).and_then(|i| to.chars().nth(i))).unwrap_or(c);
        if c == '\\' {
            return 43 + 8;
        }
        return ROWS.iter().find_map(|(row, first)| row.find(c).map(|i| first + i as i32 + 8)).unwrap_or(0);
    }
    let evdev = match key {
        "escape" => 1,
        "backspace" => 14,
        "tab" => 15,
        "enter" => 28,
        "space" => 57,
        "home" => 102,
        "up" => 103,
        "pageup" => 104,
        "left" => 105,
        "right" => 106,
        "end" => 107,
        "down" => 108,
        "pagedown" => 109,
        "insert" => 110,
        "delete" => 111,
        _ => match key.strip_prefix('f').and_then(|n| n.parse::<i32>().ok()) {
            Some(n @ 1..=10) => 58 + n,
            Some(11) => 87,
            Some(12) => 88,
            _ => return 0,
        },
    };
    evdev + 8
}

/// The character a key types with Control held, as Chromium's GTK port computes it: letters
/// become their control codes, so `Ctrl+A` is 0x01.
fn control_character(vk: i32, shift: bool) -> u16 {
    match vk {
        0x41..=0x5A => (vk - 0x40) as u16,
        0x32 if shift => 0x00,
        0x36 if shift => 0x1E,
        0xBD if shift => 0x1F,
        0xDB => 0x1B,
        0xDC => 0x1C,
        0xDD => 0x1D,
        0x0D => 0x0A,
        _ => 0,
    }
}

/// The key event a GPUI keystroke stands for: codes, modifiers and the characters it types.
pub(crate) fn key_event(keystroke: &Keystroke, kind: cef::KeyEventType, repeat: bool) -> cef::KeyEvent {
    let vk = virtual_key(&keystroke.key);
    let m = &keystroke.modifiers;
    let typed = keystroke.key_char.as_deref().and_then(|t| {
        let mut units = t.encode_utf16();
        let first = units.next()?;
        units.next().is_none().then_some(first)
    });
    let unmodified = match vk {
        0x0D => 0x0D,
        0x09 => 0x09,
        0x08 => 0x08,
        0x1B => 0x1B,
        _ => typed.unwrap_or(0),
    };
    let character = if m.control { control_character(vk, m.shift) } else { unmodified };
    let mut flags = modifiers(m);
    if repeat {
        flags |= flag::IS_REPEAT;
    }
    cef::KeyEvent {
        type_: kind,
        modifiers: flags,
        windows_key_code: vk,
        native_key_code: native_key(&keystroke.key),
        is_system_key: i32::from(m.alt),
        character,
        unmodified_character: unmodified,
        focus_on_editable_field: 0,
        ..cef::KeyEvent::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_map_to_windows_codes() {
        assert_eq!(virtual_key("a"), 0x41);
        assert_eq!(virtual_key("A"), 0x41);
        assert_eq!(virtual_key("7"), 0x37);
        assert_eq!(virtual_key("&"), 0x37);
        assert_eq!(virtual_key("enter"), 0x0D);
        assert_eq!(virtual_key("f5"), 0x74);
        assert_eq!(virtual_key("?"), 0xBF);
        assert_eq!(virtual_key("left"), 0x25);
    }

    #[test]
    fn keys_map_to_us_keycodes() {
        assert_eq!(native_key("a"), 38);
        assert_eq!(native_key("q"), 24);
        assert_eq!(native_key("1"), 10);
        assert_eq!(native_key("!"), 10);
        assert_eq!(native_key("escape"), 9);
        assert_eq!(native_key("f12"), 96);
        assert_eq!(native_key("\\"), 51);
    }

    #[test]
    fn control_letters_type_control_codes() {
        assert_eq!(control_character(0x41, false), 1);
        assert_eq!(control_character(0x5A, false), 26);
    }
}
