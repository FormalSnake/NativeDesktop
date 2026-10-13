//! A read-only copy of a window's retained tree, for incident bundles and the automation
//! `getTree` (ARCHITECTURE.md 9.3, 10.1).

use nd_core::{NodeId, Rect, WindowId};

use super::{RNode, SlotKind, Tree};
use crate::app::App;
use crate::backend::NodeKind;
use crate::element::{Button, ButtonProps, Key, TextProps};

#[derive(Clone, Debug, PartialEq)]
pub struct TreeNode {
    pub id: NodeId,
    pub role: &'static str,
    /// `root`, `view`, `text`, `canvas`, `image`, or the widget's Rust type name.
    pub kind: &'static str,
    pub name: Option<String>,
    /// The element's explicit id, from `.id(..)` or a widget constructor.
    pub test_id: Option<String>,
    /// Relative to the parent, as last committed.
    pub frame: Option<Rect>,
    pub focused: bool,
    pub children: Vec<TreeNode>,
}

impl Tree {
    fn snapshot_node(&self, id: NodeId, focused: Option<NodeId>) -> TreeNode {
        let node: &RNode = &self.nodes[&id];
        let SlotKind::Node(kind) = node.ident.kind else { unreachable!("native nodes are never views") };
        let props = &*node.props;
        let (role, kind, name) = match kind {
            NodeKind::Root => ("window", "root", None),
            NodeKind::View => ("group", "view", None),
            NodeKind::Text => ("text", "text", props.downcast_ref::<TextProps>().map(|p| p.text.to_string())),
            NodeKind::Canvas => ("canvas", "canvas", None),
            NodeKind::Image => ("image", "image", None),
            NodeKind::Widget(w) if w.type_id == std::any::TypeId::of::<Button>() => {
                ("button", w.name, props.downcast_ref::<ButtonProps>().map(|p| p.label.to_string()))
            }
            NodeKind::Widget(w) => ("generic", w.name, None),
        };
        TreeNode {
            id,
            role,
            kind,
            name,
            test_id: match &node.ident.key {
                Key::Id(id) => Some(id.to_string()),
                _ => None,
            },
            frame: node.frame,
            focused: focused == Some(id),
            children: node.native.iter().map(|c| self.snapshot_node(*c, focused)).collect(),
        }
    }
}

impl App {
    /// The window's retained tree as last committed. `None` for an unknown window or while a
    /// handler holds it.
    pub fn tree_snapshot(&self, window: WindowId) -> Option<TreeNode> {
        let tree = self.windows.get(&window)?.tree.as_ref()?;
        Some(tree.snapshot_node(tree.root, tree.dispatch.focused()))
    }
}
