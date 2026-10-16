//! AppKit input in Chromium's terms: modifier flags, which key-downs also produce a character,
//! and wheel deltas.

use objc2_app_kit::NSEventModifierFlags;

use crate::keys::flag;

pub(crate) fn modifiers(flags: NSEventModifierFlags) -> u32 {
    let mut out = 0;
    for (ns, cef) in [
        (NSEventModifierFlags::CapsLock, flag::CAPS_LOCK),
        (NSEventModifierFlags::Shift, flag::SHIFT),
        (NSEventModifierFlags::Control, flag::CONTROL),
        (NSEventModifierFlags::Option, flag::ALT),
        (NSEventModifierFlags::Command, flag::COMMAND),
        (NSEventModifierFlags::NumericPad, flag::KEY_PAD),
    ] {
        if flags.contains(ns) {
            out |= cef;
        }
    }
    out
}

/// Mouse buttons held, from `NSEvent.pressedMouseButtons` (bit 0 left, 1 right, 2 middle).
pub(crate) fn buttons(pressed: usize) -> u32 {
    let mut out = 0;
    if pressed & 1 != 0 {
        out |= flag::LEFT_MOUSE;
    }
    if pressed & 2 != 0 {
        out |= flag::RIGHT_MOUSE;
    }
    if pressed & 4 != 0 {
        out |= flag::MIDDLE_MOUSE;
    }
    out
}

/// AppKit's function-key range (arrows, F-keys, Home): no character for Chromium.
fn is_function_key(c: u16) -> bool {
    (0xF700..=0xF8FF).contains(&c)
}

/// The character a key-down that produced no `insertText:` still types: Return, keypad Enter
/// and Tab, which reach the text system as commands but a page hears as `keypress`.
pub(crate) fn command_character(characters: &str) -> Option<u16> {
    let mut units = characters.encode_utf16();
    let c = units.next()?;
    if units.next().is_some() || is_function_key(c) {
        return None;
    }
    match c {
        0x0d | 0x03 => Some(0x0d),
        0x09 => Some(0x09),
        _ => None,
    }
}

/// The first UTF-16 unit of `s`, Chromium's `character` for a key event.
pub(crate) fn first_unit(s: &str) -> u16 {
    s.encode_utf16().next().filter(|c| !is_function_key(*c)).unwrap_or(0)
}

/// Wheel deltas in pixels: trackpads report precise point deltas, wheels report lines.
pub(crate) fn wheel_delta(precise: bool, dx: f64, dy: f64) -> (i32, i32) {
    let scale = if precise { 1. } else { 40. };
    ((dx * scale).round() as i32, (dy * scale).round() as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modifiers_map_to_cef_flags() {
        let f = NSEventModifierFlags::Shift | NSEventModifierFlags::Command;
        assert_eq!(modifiers(f), flag::SHIFT | flag::COMMAND);
        assert_eq!(buttons(0b101), flag::LEFT_MOUSE | flag::MIDDLE_MOUSE);
    }

    #[test]
    fn return_and_tab_type_a_character_arrows_do_not() {
        assert_eq!(command_character("\r"), Some(0x0d));
        assert_eq!(command_character("\u{3}"), Some(0x0d));
        assert_eq!(command_character("\t"), Some(0x09));
        assert_eq!(command_character("\u{F700}"), None);
        assert_eq!(first_unit("\u{F701}"), 0);
        assert_eq!(first_unit("a"), u16::from(b'a'));
    }

    #[test]
    fn wheels_scroll_in_lines_trackpads_in_points() {
        assert_eq!(wheel_delta(true, 1.4, -3.6), (1, -4));
        assert_eq!(wheel_delta(false, 0., -1.), (0, -40));
    }
}
