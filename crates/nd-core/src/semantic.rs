//! The semantic tree schema (ARCHITECTURE.md 10.1): what automation, locators, MCP snapshots,
//! incident bundles and accessibility read. `nd-ui` builds it from the retained tree plus the
//! backend's probes (`App::semantic_tree`); this module only defines the shape and its JSON.
//!
//! JSON field names are camelCase, node ids serialize as plain integers (`"ref"`), bounds as
//! `{x, y, width, height}` in logical points from the window content's top-left.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::{NodeId, Point, Rect, Size, WindowId};

/// What a node is to a user. Selectors name roles by their kebab-case form (`role=button`,
/// `role=text-field`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Role {
    Window,
    Group,
    Text,
    Button,
    Image,
    Canvas,
    TextField,
    Checkbox,
    Switch,
    Slider,
    List,
    ListItem,
    Table,
    Row,
    Cell,
    Tree,
    TreeItem,
    TabList,
    Tab,
    Menu,
    MenuItem,
    Toolbar,
    ScrollArea,
    WindowControls,
    Generic,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Window => "window",
            Role::Group => "group",
            Role::Text => "text",
            Role::Button => "button",
            Role::Image => "image",
            Role::Canvas => "canvas",
            Role::TextField => "text-field",
            Role::Checkbox => "checkbox",
            Role::Switch => "switch",
            Role::Slider => "slider",
            Role::List => "list",
            Role::ListItem => "list-item",
            Role::Table => "table",
            Role::Row => "row",
            Role::Cell => "cell",
            Role::Tree => "tree",
            Role::TreeItem => "tree-item",
            Role::TabList => "tab-list",
            Role::Tab => "tab",
            Role::Menu => "menu",
            Role::MenuItem => "menu-item",
            Role::Toolbar => "toolbar",
            Role::ScrollArea => "scroll-area",
            Role::WindowControls => "window-controls",
            Role::Generic => "generic",
        }
    }
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Role {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        role_str::parse(s).ok_or_else(|| format!("unknown role \"{s}\""))
    }
}

/// Roles parse through serde's own string deserializer, so the names stay in one place.
mod role_str {
    use serde::Deserialize;
    use serde::de::IntoDeserializer;
    use serde::de::value::{Error, StrDeserializer};

    pub fn parse(s: &str) -> Option<super::Role> {
        let de: StrDeserializer<'_, Error> = s.into_deserializer();
        super::Role::deserialize(de).ok()
    }
}

/// One node: a retained native node as a user perceives it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticNode {
    /// The retained node id. Stable for the node's life, never reused in the process.
    #[serde(rename = "ref", with = "node_id")]
    pub id: NodeId,
    pub role: Role,
    /// The node's kind: `root`, `view`, `text`, `canvas`, `image`, or a widget's type name
    /// (`Button`). Selectors match it with `type=`.
    pub kind: String,
    /// Accessible name: a button's title, a text run's text. Empty when it has none.
    pub name: String,
    /// The value a control holds (a field's text, a slider's position), as text.
    pub value: Option<String>,
    /// The element's explicit id (`button("save", ..)`, `div().id("sidebar")`).
    pub test_id: Option<String>,
    pub placeholder: Option<String>,
    pub enabled: bool,
    pub focused: bool,
    pub checked: Option<bool>,
    pub selected: Option<bool>,
    pub expanded: Option<bool>,
    /// In the window content's coordinates. `None` for a node that is not laid out yet.
    #[serde(with = "opt_rect")]
    pub bounds: Option<Rect>,
    /// Has real size, lies inside the window content and the window is visible.
    pub visible: bool,
    pub children: Vec<SemanticNode>,
}

impl SemanticNode {
    /// This node and every node under it, depth first in sibling order.
    pub fn iter(&self) -> impl Iterator<Item = &SemanticNode> {
        let mut stack = vec![self];
        std::iter::from_fn(move || {
            let node = stack.pop()?;
            stack.extend(node.children.iter().rev());
            Some(node)
        })
    }

    pub fn find(&self, id: NodeId) -> Option<&SemanticNode> {
        self.iter().find(|n| n.id == id)
    }

    /// The node's own name and the names of everything under it, space separated: what
    /// `has-text=` and `filter(has_text)` match.
    pub fn subtree_text(&self) -> String {
        self.iter().map(|n| n.name.as_str()).filter(|n| !n.is_empty()).collect::<Vec<_>>().join(" ")
    }
}

/// One window and its content.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticWindow {
    #[serde(with = "window_id")]
    pub id: WindowId,
    pub title: String,
    /// Key (receives keyboard input). `false` on backends that cannot tell.
    pub key: bool,
    pub main: bool,
    pub visible: bool,
    /// The window's frame in global top-left screen points, where the backend knows it.
    #[serde(with = "opt_rect")]
    pub frame: Option<Rect>,
    /// The content root, role `window`.
    pub root: SemanticNode,
}

/// Every window of the app, in window id order.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SemanticTree {
    pub windows: Vec<SemanticWindow>,
}

impl SemanticTree {
    pub fn window(&self, id: WindowId) -> Option<&SemanticWindow> {
        self.windows.iter().find(|w| w.id == id)
    }

    /// Every node of every window, window by window, depth first.
    pub fn nodes(&self) -> impl Iterator<Item = (&SemanticWindow, &SemanticNode)> {
        self.windows.iter().flat_map(|w| w.root.iter().map(move |n| (w, n)))
    }

    pub fn find(&self, id: NodeId) -> Option<(&SemanticWindow, &SemanticNode)> {
        self.nodes().find(|(_, n)| n.id == id)
    }

    /// A compact text form, one line per node: `- button "Add one" [ref=e12] [testid=add]`.
    /// What the MCP `snapshot` tool returns and what a failing locator prints.
    pub fn snapshot(&self) -> String {
        let mut out = String::new();
        for window in &self.windows {
            let state = [(window.key, " [key]"), (!window.visible, " [hidden]")];
            out.push_str(&format!("window \"{}\" [id={}]", window.title, window.id.0));
            for (on, label) in state {
                if on {
                    out.push_str(label);
                }
            }
            out.push('\n');
            for child in &window.root.children {
                snapshot_node(child, 1, &mut out);
            }
        }
        out
    }
}

fn snapshot_node(node: &SemanticNode, depth: usize, out: &mut String) {
    out.push_str(&"  ".repeat(depth));
    out.push_str("- ");
    out.push_str(node.role.as_str());
    if !node.name.is_empty() {
        out.push_str(&format!(" {:?}", node.name));
    }
    out.push_str(&format!(" [ref=e{}]", node.id.0));
    if let Some(id) = &node.test_id {
        out.push_str(&format!(" [testid={id}]"));
    }
    if let Some(value) = &node.value {
        out.push_str(&format!(" [value={value:?}]"));
    }
    for (on, label) in [
        (!node.enabled, "disabled"),
        (node.focused, "focused"),
        (!node.visible, "hidden"),
        (node.checked == Some(true), "checked"),
        (node.selected == Some(true), "selected"),
        (node.expanded == Some(true), "expanded"),
    ] {
        if on {
            out.push_str(&format!(" [{label}]"));
        }
    }
    out.push('\n');
    for child in &node.children {
        snapshot_node(child, depth + 1, out);
    }
}

mod node_id {
    use super::*;

    pub fn serialize<S: Serializer>(id: &NodeId, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u64(id.0)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<NodeId, D::Error> {
        u64::deserialize(d).map(NodeId)
    }
}

mod window_id {
    use super::*;

    pub fn serialize<S: Serializer>(id: &WindowId, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u64(id.0)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<WindowId, D::Error> {
        u64::deserialize(d).map(WindowId)
    }
}

#[derive(Serialize, Deserialize)]
struct Bounds {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

mod opt_rect {
    use super::*;

    pub fn serialize<S: Serializer>(rect: &Option<Rect>, s: S) -> Result<S::Ok, S::Error> {
        rect.map(|r| Bounds { x: r.origin.x, y: r.origin.y, width: r.size.width, height: r.size.height }).serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Rect>, D::Error> {
        Ok(Option::<Bounds>::deserialize(d)?
            .map(|b| Rect { origin: Point { x: b.x, y: b.y }, size: Size { width: b.width, height: b.height } }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: u64, role: Role, name: &str, children: Vec<SemanticNode>) -> SemanticNode {
        SemanticNode {
            id: NodeId(id),
            role,
            kind: "view".into(),
            name: name.into(),
            value: None,
            test_id: None,
            placeholder: None,
            enabled: true,
            focused: false,
            checked: None,
            selected: None,
            expanded: None,
            bounds: Some(Rect::new(0., 0., 10., 10.)),
            visible: true,
            children,
        }
    }

    #[test]
    fn json_round_trip_and_wire_names() {
        let tree = SemanticTree {
            windows: vec![SemanticWindow {
                id: WindowId(1),
                title: "Hello".into(),
                key: true,
                main: true,
                visible: true,
                frame: None,
                root: node(1, Role::Window, "Hello", vec![node(2, Role::TextField, "", vec![])]),
            }],
        };
        let json = serde_json_value(&tree);
        assert!(json.contains("\"ref\":2"), "{json}");
        assert!(json.contains("\"role\":\"text-field\""), "{json}");
        assert!(json.contains("\"testId\":null"), "{json}");
        assert!(json.contains("\"bounds\":{\"x\":0.0,\"y\":0.0,\"width\":10.0,\"height\":10.0}"), "{json}");
        let back: SemanticTree = ::serde_json::from_str(&json).unwrap();
        assert_eq!(back, tree);
    }

    #[test]
    fn roles_parse_from_selector_names() {
        assert_eq!("button".parse::<Role>(), Ok(Role::Button));
        assert_eq!("list-item".parse::<Role>(), Ok(Role::ListItem));
        assert!("nope".parse::<Role>().is_err());
        assert_eq!(Role::TextField.to_string(), "text-field");
    }

    fn serde_json_value(tree: &SemanticTree) -> String {
        ::serde_json::to_string(tree).unwrap()
    }
}
