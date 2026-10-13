use nd_ui::{Keystroke, Modifiers};
use objc2_app_kit::{NSEvent, NSEventModifierFlags};

/// Named keys by virtual key code (`HIToolbox/Events.h`), so they match whatever character the
/// layout produces for them.
fn named_key(code: u16) -> Option<&'static str> {
    Some(match code {
        0x24 | 0x4c => "enter",
        0x30 => "tab",
        0x31 => "space",
        0x33 => "backspace",
        0x35 => "escape",
        0x75 => "delete",
        0x73 => "home",
        0x77 => "end",
        0x74 => "pageup",
        0x79 => "pagedown",
        0x7b => "left",
        0x7c => "right",
        0x7d => "down",
        0x7e => "up",
        _ => return None,
    })
}

/// The nd keystroke for a key-down. `None` for events that carry no key (dead keys mid
/// composition).
pub fn keystroke(event: &NSEvent) -> Option<Keystroke> {
    let flags = event.modifierFlags();
    let code = event.keyCode();
    let named = named_key(code);
    let modifiers = Modifiers {
        control: flags.contains(NSEventModifierFlags::Control),
        alt: flags.contains(NSEventModifierFlags::Option),
        shift: flags.contains(NSEventModifierFlags::Shift),
        platform: flags.contains(NSEventModifierFlags::Command),
        // AppKit sets the function flag on arrows and navigation keys too; only a bare fn
        // press counts as the fn modifier.
        function: flags.contains(NSEventModifierFlags::Function) && named.is_none(),
    };
    let key = match named {
        Some(name) => name.to_owned(),
        None => {
            let chars = event.charactersIgnoringModifiers()?.to_string();
            if chars.is_empty() {
                return None;
            }
            chars.to_lowercase()
        }
    };
    let key_char = if modifiers.platform || modifiers.control || named.is_some_and(|n| n != "space") {
        None
    } else {
        event.characters().map(|c| c.to_string()).filter(|c| !c.is_empty())
    };
    Some(Keystroke { modifiers, key, key_char })
}
