//! Input in the form the session encodes: physical keys with modifiers and the text the
//! platform produced for them. libghostty's encoder turns these into the bytes the program
//! asked for (legacy, application cursor keys, `modifyOtherKeys` or the Kitty protocol).

pub use libghostty_vt::key::{Action as KeyAction, Key, KittyKeyFlags, Mods};

/// One key event.
#[derive(Clone, Debug, PartialEq)]
pub struct KeyInput {
    pub key: Key,
    pub action: KeyAction,
    pub mods: Mods,
    /// Modifiers the platform already used to produce `text` (Shift for `A`).
    pub consumed: Mods,
    /// The text the key produced on the current layout, if any.
    pub text: Option<String>,
    /// The character the key produces with no modifiers, which the Kitty protocol reports.
    pub unshifted: char,
    /// An input method is composing; the key belongs to it.
    pub composing: bool,
}

impl KeyInput {
    pub fn press(key: Key, mods: Mods) -> Self {
        Self {
            key,
            action: KeyAction::Press,
            mods,
            consumed: Mods::empty(),
            text: None,
            unshifted: unshifted_us(key),
            composing: false,
        }
    }

    /// A printable key: `text` is what it typed, with Shift consumed when it was held.
    pub fn typed(key: Key, mods: Mods, text: impl Into<String>) -> Self {
        let consumed = if mods.contains(Mods::SHIFT) { Mods::SHIFT } else { Mods::empty() };
        Self { text: Some(text.into()), consumed, ..Self::press(key, mods) }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
}

impl From<MouseButton> for libghostty_vt::mouse::Button {
    fn from(b: MouseButton) -> Self {
        match b {
            MouseButton::Left => Self::Left,
            MouseButton::Middle => Self::Middle,
            MouseButton::Right => Self::Right,
        }
    }
}

/// The character `key` types on a US layout with no modifiers, `'\0'` for keys that type
/// none. Platforms that can ask the active layout use that instead.
pub fn unshifted_us(key: Key) -> char {
    use Key::*;
    match key {
        A => 'a',
        B => 'b',
        C => 'c',
        D => 'd',
        E => 'e',
        F => 'f',
        G => 'g',
        H => 'h',
        I => 'i',
        J => 'j',
        K => 'k',
        L => 'l',
        M => 'm',
        N => 'n',
        O => 'o',
        P => 'p',
        Q => 'q',
        R => 'r',
        S => 's',
        T => 't',
        U => 'u',
        V => 'v',
        W => 'w',
        X => 'x',
        Y => 'y',
        Z => 'z',
        Digit0 => '0',
        Digit1 => '1',
        Digit2 => '2',
        Digit3 => '3',
        Digit4 => '4',
        Digit5 => '5',
        Digit6 => '6',
        Digit7 => '7',
        Digit8 => '8',
        Digit9 => '9',
        Space => ' ',
        Minus => '-',
        Equal => '=',
        BracketLeft => '[',
        BracketRight => ']',
        Backslash => '\\',
        Semicolon => ';',
        Quote => '\'',
        Comma => ',',
        Period => '.',
        Slash => '/',
        Backquote => '`',
        _ => '\0',
    }
}

/// The physical key with this US-layout character, for platforms that name keys by what
/// they type.
pub fn key_for_char(c: char) -> Option<Key> {
    use Key::*;
    Some(match c.to_ascii_lowercase() {
        'a' => A,
        'b' => B,
        'c' => C,
        'd' => D,
        'e' => E,
        'f' => F,
        'g' => G,
        'h' => H,
        'i' => I,
        'j' => J,
        'k' => K,
        'l' => L,
        'm' => M,
        'n' => N,
        'o' => O,
        'p' => P,
        'q' => Q,
        'r' => R,
        's' => S,
        't' => T,
        'u' => U,
        'v' => V,
        'w' => W,
        'x' => X,
        'y' => Y,
        'z' => Z,
        '0' => Digit0,
        '1' => Digit1,
        '2' => Digit2,
        '3' => Digit3,
        '4' => Digit4,
        '5' => Digit5,
        '6' => Digit6,
        '7' => Digit7,
        '8' => Digit8,
        '9' => Digit9,
        ' ' => Space,
        '-' => Minus,
        '=' => Equal,
        '[' => BracketLeft,
        ']' => BracketRight,
        '\\' => Backslash,
        ';' => Semicolon,
        '\'' => Quote,
        ',' => Comma,
        '.' => Period,
        '/' => Slash,
        '`' => Backquote,
        _ => return None,
    })
}

/// macOS virtual key codes (`kVK_*` in `HIToolbox/Events.h`) to physical keys.
pub fn key_for_mac_keycode(code: u16) -> Key {
    use Key::*;
    match code {
        0x00 => A,
        0x01 => S,
        0x02 => D,
        0x03 => F,
        0x04 => H,
        0x05 => G,
        0x06 => Z,
        0x07 => X,
        0x08 => C,
        0x09 => V,
        0x0A => IntlBackslash,
        0x0B => B,
        0x0C => Q,
        0x0D => W,
        0x0E => E,
        0x0F => R,
        0x10 => Y,
        0x11 => T,
        0x12 => Digit1,
        0x13 => Digit2,
        0x14 => Digit3,
        0x15 => Digit4,
        0x16 => Digit6,
        0x17 => Digit5,
        0x18 => Equal,
        0x19 => Digit9,
        0x1A => Digit7,
        0x1B => Minus,
        0x1C => Digit8,
        0x1D => Digit0,
        0x1E => BracketRight,
        0x1F => O,
        0x20 => U,
        0x21 => BracketLeft,
        0x22 => I,
        0x23 => P,
        0x24 => Enter,
        0x25 => L,
        0x26 => J,
        0x27 => Quote,
        0x28 => K,
        0x29 => Semicolon,
        0x2A => Backslash,
        0x2B => Comma,
        0x2C => Slash,
        0x2D => N,
        0x2E => M,
        0x2F => Period,
        0x30 => Tab,
        0x31 => Space,
        0x32 => Backquote,
        0x33 => Backspace,
        0x35 => Escape,
        0x36 => MetaRight,
        0x37 => MetaLeft,
        0x38 => ShiftLeft,
        0x39 => CapsLock,
        0x3A => AltLeft,
        0x3B => ControlLeft,
        0x3C => ShiftRight,
        0x3D => AltRight,
        0x3E => ControlRight,
        0x3F => Fn,
        0x40 => F17,
        0x41 => NumpadDecimal,
        0x43 => NumpadMultiply,
        0x45 => NumpadAdd,
        0x47 => NumLock,
        0x4B => NumpadDivide,
        0x4C => NumpadEnter,
        0x4E => NumpadSubtract,
        0x4F => F18,
        0x50 => F19,
        0x51 => NumpadEqual,
        0x52 => Numpad0,
        0x53 => Numpad1,
        0x54 => Numpad2,
        0x55 => Numpad3,
        0x56 => Numpad4,
        0x57 => Numpad5,
        0x58 => Numpad6,
        0x59 => Numpad7,
        0x5A => F20,
        0x5B => Numpad8,
        0x5C => Numpad9,
        0x5D => IntlYen,
        0x5E => IntlRo,
        0x5F => NumpadComma,
        0x60 => F5,
        0x61 => F6,
        0x62 => F7,
        0x63 => F3,
        0x64 => F8,
        0x65 => F9,
        0x66 => NonConvert,
        0x67 => F11,
        0x68 => KanaMode,
        0x69 => F13,
        0x6A => F16,
        0x6B => F14,
        0x6D => F10,
        0x6F => F12,
        0x71 => F15,
        0x72 => Insert,
        0x73 => Home,
        0x74 => PageUp,
        0x75 => Delete,
        0x76 => F4,
        0x77 => End,
        0x78 => F2,
        0x79 => PageDown,
        0x7A => F1,
        0x7B => ArrowLeft,
        0x7C => ArrowRight,
        0x7D => ArrowDown,
        0x7E => ArrowUp,
        _ => Unidentified,
    }
}

/// GPUI's key names (`Keystroke::key`) to physical keys.
pub fn key_for_name(name: &str) -> Key {
    use Key::*;
    match name {
        "enter" => Enter,
        "tab" => Tab,
        "backspace" => Backspace,
        "delete" => Delete,
        "escape" => Escape,
        "space" => Space,
        "up" => ArrowUp,
        "down" => ArrowDown,
        "left" => ArrowLeft,
        "right" => ArrowRight,
        "home" => Home,
        "end" => End,
        "pageup" => PageUp,
        "pagedown" => PageDown,
        "insert" => Insert,
        "f1" => F1,
        "f2" => F2,
        "f3" => F3,
        "f4" => F4,
        "f5" => F5,
        "f6" => F6,
        "f7" => F7,
        "f8" => F8,
        "f9" => F9,
        "f10" => F10,
        "f11" => F11,
        "f12" => F12,
        other => {
            let mut chars = other.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => key_for_char(c).unwrap_or(Unidentified),
                _ => Unidentified,
            }
        }
    }
}
