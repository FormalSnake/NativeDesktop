//! Framework actions and the standard bindings every app gets.

use nd_gpui::gpui::{self, KeyBinding};

gpui::actions!(
    nd,
    [
        /// Move focus to the next node in nd's focus order (tree order, then `tab_index`).
        FocusNext,
        /// Move focus to the previous node in nd's focus order.
        FocusPrevious,
        /// Return: listeners on the focus path see it first; unhandled, it activates the default
        /// button of the innermost focus scope (or the window), unless the focused node takes
        /// Return itself (a multi-line text view).
        Confirm,
        /// Escape: listeners on the focus path see it first; unhandled, it dismisses the innermost
        /// focus scope that asked for it (the innermost overlay).
        Cancel,
        /// F12: cycles the frame HUD hidden, minimal and full (`nd::hud`).
        ToggleFrameHud,
        /// Shows or hides the window's sidebar. Unhandled, nd toggles the window's scaffold the
        /// way its toolbar toggle does (`WindowCommand::ToggleSidebar`).
        ToggleSidebar
    ]
);

/// OS conventions shared by every app. Apps add their own bindings after these, so theirs win.
pub fn standard_bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("tab", FocusNext, None),
        KeyBinding::new("shift-tab", FocusPrevious, None),
        KeyBinding::new("enter", Confirm, None),
        KeyBinding::new("escape", Cancel, None),
        KeyBinding::new("f12", ToggleFrameHud, None),
        KeyBinding::new(SIDEBAR_KEY, ToggleSidebar, None),
    ]
}

/// The platform's sidebar shortcut: Control-Command-S on macOS (the HIG's View menu), F9 in
/// GNOME (the HIG's standard keyboard shortcuts) and on Windows, which has no convention of its
/// own and shares GNOME's key for the same app.
const SIDEBAR_KEY: &str = if cfg!(target_os = "macos") { "ctrl-cmd-s" } else { "f9" };
