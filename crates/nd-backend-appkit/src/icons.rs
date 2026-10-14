//! Lucide names to SF Symbols, so icons are the system's own on macOS.

use objc2::rc::Retained;
use objc2_app_kit::NSImage;
use objc2_foundation::NSString;

const SYMBOLS: &[(&str, &str)] = &[
    ("archive", "archivebox"),
    ("check", "checkmark"),
    ("chevron-left", "chevron.left"),
    ("chevron-right", "chevron.right"),
    ("circle-check", "checkmark.circle"),
    ("file-text", "doc.text"),
    ("folder", "folder"),
    ("inbox", "tray"),
    ("info", "info.circle"),
    ("map-pin", "mappin"),
    ("minus", "minus"),
    ("notebook", "book.closed"),
    ("panel-left", "sidebar.left"),
    ("panel-right", "sidebar.right"),
    ("pin", "pin"),
    ("plus", "plus"),
    ("search", "magnifyingglass"),
    ("settings", "gearshape"),
    ("share", "square.and.arrow.up"),
    ("square-pen", "square.and.pencil"),
    ("star", "star"),
    ("trash", "trash"),
    ("triangle-alert", "exclamationmark.triangle"),
    ("calendar", "calendar"),
    ("clock", "clock"),
    ("eye", "eye"),
    ("eye-off", "eye.slash"),
    ("globe", "globe"),
    ("lock", "lock"),
    ("mail", "envelope"),
    ("user", "person"),
    ("x", "xmark"),
    ("circle-x", "xmark.circle.fill"),
];

/// The SF Symbol for a Lucide name; `None` when nd has no mapping for it.
pub(crate) fn symbol(lucide: &str) -> Option<Retained<NSImage>> {
    NSImage::imageWithSystemSymbolName_accessibilityDescription(&NSString::from_str(symbol_name(lucide)?), None)
}

/// The SF Symbol's name for a Lucide name, for SwiftUI's `systemImage:`.
pub(crate) fn symbol_name(lucide: &str) -> Option<&'static str> {
    SYMBOLS.iter().find(|(l, _)| *l == lucide).map(|(_, name)| *name)
}
