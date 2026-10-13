//! The semantic side of the navigation and feedback widgets: probes, semantic actions, and the
//! inspection calls tier 2 tests and `paletteLayout` read AppKit's state through.

use nd_components::PaletteKey;
use nd_core::{NodeId, Rect};
use nd_ui::semantics::{ActionError, Probe, SemanticAction};
use objc2::rc::Retained;
use objc2_app_kit::{NSOutlineView, NSView};
use objc2_foundation::{NSRect, NSString};

pub use crate::palette::{PaletteLayout, PaletteRowLayout};

use super::{AppKitBackend, Native, State};

fn rect(r: NSRect) -> Rect {
    Rect::new(r.origin.x as f32, r.origin.y as f32, r.size.width as f32, r.size.height as f32)
}

impl State {
    /// What the nav widgets add to a probe: a presented palette's card and text, the toast on
    /// screen. `content` is the window content the bounds are relative to.
    pub(super) fn probe_nav(&self, native: &Native, content: Option<&NSView>, probe: &mut Probe) {
        match native {
            Native::Palette(p) => {
                let model = p.model();
                probe.expanded = Some(model.is_presented());
                probe.value = Some(model.field_text());
                if let Some(content) = content {
                    probe.bounds = Some(p.card_frame_in(content).map_or(Rect::default(), rect));
                }
            }
            Native::Toasts(t) => match t.shown() {
                Some((title, ..)) => {
                    probe.name = Some(title);
                    if let (Some(content), Some(view)) = (content, t.toast_view()) {
                        probe.bounds = Some(rect(content.convertRect_fromView(view.bounds(), Some(&view))));
                    }
                }
                None => probe.bounds = Some(Rect::default()),
            },
            Native::SourceTree(s) => {
                let outline = s.outline();
                let row = outline.selectedRow();
                if row >= 0 {
                    probe.value =
                        outline.itemAtRow(row).and_then(|i| i.downcast::<NSString>().ok()).map(|s| s.to_string());
                }
            }
            _ => {}
        }
    }
}

impl AppKitBackend {
    /// Semantic actions on the nav widgets; `None` when `node` is none of them.
    pub(super) fn perform_nav(&self, node: NodeId, action: &SemanticAction) -> Option<Result<(), ActionError>> {
        let state = self.0.borrow();
        match (state.nodes.get(&node)?, action) {
            (Native::Palette(p), SemanticAction::SetValue(text)) => p.set_text(text),
            (Native::Palette(p), SemanticAction::Type(text)) => p.type_text(text),
            (Native::Palette(p), SemanticAction::Click) => p.activate(),
            (Native::Toasts(t), SemanticAction::Click) => t.click(false),
            (Native::Banner(b), SemanticAction::Click) => b.press(),
            (Native::Palette(_) | Native::Toasts(_) | Native::Banner(_), _) => {
                return Some(Err(ActionError::Unsupported(format!("{action:?} on {node:?}"))));
            }
            _ => return None,
        }
        Some(Ok(()))
    }

    /// The presented palette's drawn geometry, relative to the window.
    pub fn palette_layout(&self, node: NodeId) -> Option<PaletteLayout> {
        match self.0.try_borrow().ok()?.nodes.get(&node)? {
            Native::Palette(p) => p.layout_report(),
            _ => None,
        }
    }

    /// A key the palette's field editor would hand it.
    pub fn palette_key(&self, node: NodeId, key: PaletteKey) -> bool {
        match self.0.borrow().nodes.get(&node) {
            Some(Native::Palette(p)) => p.press(key),
            _ => false,
        }
    }

    /// The source tree's outline view.
    pub fn source_outline(&self, node: NodeId) -> Option<Retained<NSOutlineView>> {
        match self.0.try_borrow().ok()?.nodes.get(&node)? {
            Native::SourceTree(s) => Some(objc2::Message::retain(s.outline())),
            _ => None,
        }
    }

    /// The object the outline tracks row `id` by.
    pub fn source_item(&self, node: NodeId, id: &str) -> Option<Retained<NSString>> {
        match self.0.try_borrow().ok()?.nodes.get(&node)? {
            Native::SourceTree(s) => s.item(id),
            _ => None,
        }
    }

    /// Clicks trailing action `action` on row `id`.
    pub fn source_click_action(&self, node: NodeId, id: &str, action: &str) -> bool {
        match self.0.borrow().nodes.get(&node) {
            Some(Native::SourceTree(s)) => s.click_action(id, action),
            _ => false,
        }
    }

    /// The alpha of row `id`'s action buttons, and shows or hides them as the pointer would.
    pub fn source_actions(&self, node: NodeId, id: &str, hover: Option<bool>) -> Vec<f64> {
        match self.0.borrow().nodes.get(&node) {
            Some(Native::SourceTree(s)) => {
                if let Some(inside) = hover {
                    s.hover(id, inside);
                }
                s.action_alpha(id)
            }
            _ => Vec::new(),
        }
    }

    pub fn source_empty_title(&self, node: NodeId) -> Option<String> {
        match self.0.try_borrow().ok()?.nodes.get(&node)? {
            Native::SourceTree(s) => s.empty_title(),
            _ => None,
        }
    }

    /// The toast on screen: title, button label, frame in its layer, material class.
    pub fn toast_shown(&self, node: NodeId) -> Option<(String, Option<String>, NSRect, String)> {
        match self.0.try_borrow().ok()?.nodes.get(&node)? {
            Native::Toasts(t) => {
                let (title, button, frame) = t.shown()?;
                Some((title, button, frame, t.material_class().unwrap_or_default()))
            }
            _ => None,
        }
    }

    /// Clicks the shown toast's button (or its body).
    pub fn toast_click(&self, node: NodeId, button: bool) {
        if let Some(Native::Toasts(t)) = self.0.borrow().nodes.get(&node) {
            t.click(button);
        }
    }
}
