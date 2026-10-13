//! Framework actions and the standard bindings every app gets.

use nd_gpui::gpui::{self, KeyBinding};

gpui::actions!(
    nd,
    [
        /// Move focus to the next node in nd's focus order (tree order, then `tab_index`).
        FocusNext,
        /// Move focus to the previous node in nd's focus order.
        FocusPrevious
    ]
);

/// OS conventions shared by every app. Apps add their own bindings after these, so theirs win.
pub fn standard_bindings() -> Vec<KeyBinding> {
    vec![KeyBinding::new("tab", FocusNext, None), KeyBinding::new("shift-tab", FocusPrevious, None)]
}
