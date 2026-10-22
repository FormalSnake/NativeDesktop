//! WinUI access keys on the Fluent look. A lone Alt shows a key tip on each control with a key;
//! typing a key presses its control, Esc or Alt again hides the tips, and Alt with a one-letter
//! key presses the control without them. Keys are matched case-insensitively.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use nd_gpui::gpui::{self, prelude::*};
use gpui::{AnyElement, Keystroke, div, px, rgb};
use nd_ui::backend::{Backend, NodeKind};
use nd_ui::semantics::{ActionError, SemanticAction};
use nd_ui::{ButtonProps, DivProps, NodeId, WindowId};

use crate::backend::{GpuiBackend, is_button};

/// Windows showing key tips, with the keys typed so far.
pub(crate) type Showing = Rc<RefCell<HashMap<WindowId, String>>>;

/// KeyTip from WinUI's generic.xaml: SystemChromeGrayColor fill, white 12 px text, padding 4
/// and a transparent 1 px border, square, the same in light and dark.
const FILL: u32 = 0x767676;
const TEXT: f32 = 12.;
const LINE: f32 = 16.;
const PAD: f32 = 4.;
const HEIGHT: f32 = LINE + 2. * (PAD + 1.);

pub(crate) struct AccessKey {
    /// Uppercase.
    pub key: String,
    /// The node that carries the key, where its tip sits.
    pub owner: NodeId,
    /// The control the key presses: the owner, or the first control inside a `div`'s.
    pub target: NodeId,
}

fn normalized(key: &str) -> Option<String> {
    let key = key.trim().to_uppercase();
    (!key.is_empty() && key.chars().all(char::is_alphanumeric)).then_some(key)
}

/// The letter or digit a keystroke types, uppercase.
fn typed(keystroke: &Keystroke) -> Option<char> {
    let mut chars = keystroke.key.chars();
    let c = chars.next()?;
    (chars.next().is_none() && c.is_alphanumeric()).then(|| c.to_ascii_uppercase())
}

impl GpuiBackend {
    fn fluent(&self, window: WindowId) -> bool {
        self.state.borrow().theme.window_theme(window).preset == nd_theme::Preset::Fluent
    }

    /// The window's access keys in tree order. A key taken earlier in the tree wins, as WinUI
    /// keeps the first element added.
    pub(crate) fn access_keys(&self, window: WindowId) -> Vec<AccessKey> {
        let state = self.state.borrow();
        let Some(root) = state.windows.get(&window).and_then(|w| w.root) else { return Vec::new() };
        let control = |id: NodeId| {
            state.nodes.get(&id).is_some_and(|n| {
                matches!(n.kind, NodeKind::Widget(_))
                    && n.props.downcast_ref::<ButtonProps>().is_none_or(|p| p.enabled)
            })
        };
        let first_control = |from: NodeId| {
            let mut stack = vec![from];
            while let Some(id) = stack.pop() {
                if control(id) {
                    return Some(id);
                }
                if let Some(n) = state.nodes.get(&id) {
                    stack.extend(n.children.iter().rev());
                }
            }
            None
        };
        let mut keys: Vec<AccessKey> = Vec::new();
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            let Some(node) = state.nodes.get(&id) else { continue };
            stack.extend(node.children.iter().rev());
            let (key, target) = if let Some(p) = node.props.downcast_ref::<ButtonProps>() {
                (p.access_key.as_deref(), (is_button(node.kind) && p.enabled).then_some(id))
            } else if let Some(p) = node.props.downcast_ref::<DivProps>() {
                (p.access_key.as_deref(), p.access_key.as_ref().and_then(|_| first_control(id)))
            } else {
                continue;
            };
            let (Some(key), Some(target)) = (key.and_then(normalized), target) else { continue };
            let shown = self.bounds(id).is_some_and(|b| b.size.width > 0. && b.size.height > 0.);
            if shown && !keys.iter().any(|k| k.key == key) {
                keys.push(AccessKey { key, owner: id, target });
            }
        }
        keys
    }

    pub(crate) fn hide_key_tips(&self, window: WindowId) -> bool {
        self.key_tips.borrow_mut().remove(&window).is_some()
    }

    /// Handles a keystroke for access keys before anything else sees it. `true` when it was
    /// taken: tips shown or hidden, a control pressed, or a key no tip starts with typed while
    /// they show (WinUI rejects it). Enter, Tab, arrows and shortcuts hide the tips and go on.
    pub(crate) fn access_key_input(&self, window: WindowId, keystroke: &Keystroke) -> bool {
        if !self.fluent(window) {
            return false;
        }
        let m = keystroke.modifiers;
        let typing = typed(keystroke);
        let Some(so_far) = self.key_tips.borrow().get(&window).cloned() else {
            // GPUI reports a lone Alt press and release as the keystroke `alt`.
            if keystroke.key == "alt" && !m.modified() {
                if self.access_keys(window).is_empty() {
                    return false;
                }
                self.key_tips.borrow_mut().insert(window, String::new());
                return true;
            }
            if !(m.alt && !m.control && !m.platform && !m.function) {
                return false;
            }
            let Some(c) = typing else { return false };
            let pressed = self.access_keys(window).into_iter().find(|k| k.key.len() == 1 && k.key.starts_with(c));
            return pressed.is_some_and(|k| {
                self.press_access_key(k.target);
                true
            });
        };
        if keystroke.key == "escape" || keystroke.key == "alt" {
            self.hide_key_tips(window);
            return true;
        }
        let Some(c) = typing.filter(|_| !(m.control || m.alt || m.platform || m.function)) else {
            self.hide_key_tips(window);
            return false;
        };
        let so_far = format!("{so_far}{c}");
        let keys = self.access_keys(window);
        if let Some(k) = keys.iter().find(|k| k.key == so_far) {
            self.hide_key_tips(window);
            self.press_access_key(k.target);
        } else if keys.iter().any(|k| k.key.starts_with(&so_far)) {
            self.key_tips.borrow_mut().insert(window, so_far);
        }
        true
    }

    /// A button or toggle is pressed as automation presses it; a control a press does not
    /// act on (a text field) takes focus instead.
    fn press_access_key(&self, target: NodeId) {
        let mut backend = self.clone();
        if let Err(ActionError::Unsupported(_)) = backend.perform(target, &SemanticAction::Click) {
            let _ = backend.perform(target, &SemanticAction::Focus);
        }
    }

    /// The key tips over the window's content: each centred under its control, or above it
    /// where the window ends first (WinUI's automatic placement, without its cost search).
    pub(crate) fn key_tip_layer(&self, window: WindowId, height: f32) -> Option<AnyElement> {
        let so_far = self.key_tips.borrow().get(&window).cloned()?;
        let tips = self.access_keys(window).into_iter().filter(|k| k.key.starts_with(&so_far)).filter_map(|k| {
            let b = self.bounds(k.owner)?;
            let below = b.origin.y + b.size.height;
            let top = if below + HEIGHT <= height { below } else { b.origin.y - HEIGHT };
            Some(
                div()
                    .absolute()
                    .left(px(b.origin.x))
                    .top(px(top))
                    .w(px(b.size.width))
                    .flex()
                    .justify_center()
                    .child(
                        div()
                            .flex_none()
                            .p(px(PAD))
                            .border_1()
                            .border_color(gpui::transparent_black())
                            .bg(rgb(FILL))
                            .text_color(gpui::white())
                            .text_size(px(TEXT))
                            .line_height(px(LINE))
                            .whitespace_nowrap()
                            .child(k.key),
                    ),
            )
        });
        Some(div().absolute().inset_0().children(tips).into_any_element())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_uppercase_letters_and_digits() {
        assert_eq!(normalized(" s"), Some("S".into()));
        assert_eq!(normalized("a1"), Some("A1".into()));
        assert_eq!(normalized(""), None);
        assert_eq!(normalized("+"), None);
    }

    #[test]
    fn a_keystroke_types_one_letter_or_digit() {
        assert_eq!(typed(&Keystroke::parse("alt-s").unwrap()), Some('S'));
        assert_eq!(typed(&Keystroke::parse("2").unwrap()), Some('2'));
        assert_eq!(typed(&Keystroke::parse("enter").unwrap()), None);
    }
}
