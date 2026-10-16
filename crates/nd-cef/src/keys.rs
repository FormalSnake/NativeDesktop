//! Input in Chromium's terms, shared by the platforms: modifier flag bits, the edit commands
//! shortcuts stand for, and zoom levels.

/// `cef_event_flags_t` bits.
#[allow(dead_code)]
pub(crate) mod flag {
    pub const CAPS_LOCK: u32 = 1;
    pub const SHIFT: u32 = 1 << 1;
    pub const CONTROL: u32 = 1 << 2;
    pub const ALT: u32 = 1 << 3;
    pub const LEFT_MOUSE: u32 = 1 << 4;
    pub const MIDDLE_MOUSE: u32 = 1 << 5;
    pub const RIGHT_MOUSE: u32 = 1 << 6;
    pub const COMMAND: u32 = 1 << 7;
    pub const NUM_LOCK: u32 = 1 << 8;
    pub const KEY_PAD: u32 = 1 << 9;
    pub const IS_REPEAT: u32 = 1 << 13;
    pub const PRECISION_SCROLLING_DELTA: u32 = 1 << 14;
}

#[cfg(target_os = "macos")]
/// An edit command a shortcut stands for (Cmd on macOS, Ctrl on Linux).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Edit {
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    PasteAndMatchStyle,
    SelectAll,
}

#[cfg(target_os = "macos")]
/// The edit command of a shortcut, by its unmodified key, shift and the platform's alternate
/// modifier (Option on macOS, Alt elsewhere).
pub(crate) fn edit_command(key: &str, shift: bool, alternate: bool) -> Option<Edit> {
    Some(match (key, shift, alternate) {
        ("z", false, false) => Edit::Undo,
        ("z", true, false) => Edit::Redo,
        ("x", false, false) => Edit::Cut,
        ("c", false, false) => Edit::Copy,
        ("v", false, false) => Edit::Paste,
        ("v", true, true) | ("v", true, false) => Edit::PasteAndMatchStyle,
        ("a", false, false) => Edit::SelectAll,
        _ => return None,
    })
}

/// Zoom levels are a log scale: each step multiplies by 1.2.
pub(crate) fn zoom_level(factor: f64) -> f64 {
    factor.max(0.01).ln() / 1.2f64.ln()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    fn shortcuts_are_edit_commands() {
        assert_eq!(edit_command("c", false, false), Some(Edit::Copy));
        assert_eq!(edit_command("z", true, false), Some(Edit::Redo));
        assert_eq!(edit_command("q", false, false), None);
    }

    #[test]
    fn zoom_round_trips() {
        let factor = |level: f64| 1.2f64.powf(level);
        assert!((factor(zoom_level(1.5)) - 1.5).abs() < 1e-9);
        assert_eq!(zoom_level(1.), 0.);
    }
}
