//! The page's accessibility tree, mirrored from Chromium's serialized updates
//! (`on_accessibility_tree_change`) so the view can expose it to the platform (on macOS,
//! VoiceOver and automation through `NSAccessibilityElement` children). Covered: roles, names,
//! values and frames of the nodes, and their nesting; actions and live-region notifications are
//! not.

use std::collections::HashMap;

use cef::*;
use serde_json::{Map, Number, Value as Json};

/// A CEF value as JSON.
pub(crate) fn to_json(v: &Value) -> Json {
    match v.get_type() {
        ValueType::BOOL => Json::Bool(v.bool() != 0),
        ValueType::INT => Json::Number(v.int().into()),
        ValueType::DOUBLE => Number::from_f64(v.double()).map(Json::Number).unwrap_or(Json::Null),
        ValueType::STRING => Json::String(super::text(&v.string())),
        ValueType::LIST => v.list().map(|l| list_json(&l)).unwrap_or(Json::Null),
        ValueType::DICTIONARY => v.dictionary().map(|d| dict_json(&d)).unwrap_or(Json::Null),
        _ => Json::Null,
    }
}

fn list_json(l: &ListValue) -> Json {
    Json::Array((0..l.size()).filter_map(|i| l.value(i)).map(|v| to_json(&v)).collect())
}

fn dict_json(d: &DictionaryValue) -> Json {
    let mut keys = CefStringList::new();
    d.keys(Some(&mut keys));
    let mut map = Map::new();
    for key in keys {
        if let Some(v) = d.value(Some(&CefString::from(key.as_str()))) {
            map.insert(key, to_json(&v));
        }
    }
    Json::Object(map)
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Node {
    pub role: String,
    pub name: String,
    pub value: String,
    /// Relative to `container`, CSS pixels.
    pub bounds: (f64, f64, f64, f64),
    pub container: Option<i64>,
    pub children: Vec<i64>,
}

#[derive(Debug, Default)]
pub(crate) struct Tree {
    pub nodes: HashMap<i64, Node>,
    pub root: Option<i64>,
}

fn bounds(b: &Json) -> (f64, f64, f64, f64) {
    let n = |k: &str| b[k].as_f64().unwrap_or(0.);
    (n("x"), n("y"), n("width"), n("height"))
}

impl Tree {
    pub(crate) fn apply(&mut self, change: &Json) {
        for update in change["updates"].as_array().into_iter().flatten() {
            if let Some(clear) = update["node_id_to_clear"].as_i64().filter(|id| *id != 0) {
                self.clear(clear);
            }
            if let Some(root) = update["root_id"].as_i64().filter(|id| *id != 0) {
                self.root = Some(root);
            }
            for n in update["nodes"].as_array().into_iter().flatten() {
                let Some(id) = n["id"].as_i64() else { continue };
                let attrs = &n["attributes"];
                let text = |k: &str| attrs[k].as_str().unwrap_or_default().to_owned();
                let node = Node {
                    role: n["role"].as_str().unwrap_or_default().to_owned(),
                    name: text("name"),
                    value: text("value"),
                    bounds: bounds(&n["location"]),
                    container: n["offset_container_id"].as_i64().filter(|c| *c > 0),
                    children: n["child_ids"].as_array().into_iter().flatten().filter_map(Json::as_i64).collect(),
                };
                self.nodes.insert(id, node);
            }
        }
    }

    pub(crate) fn locations(&mut self, change: &Json) {
        for c in change.as_array().into_iter().flatten() {
            let (Some(id), loc) = (c["id"].as_i64(), &c["new_location"]) else { continue };
            if let Some(node) = self.nodes.get_mut(&id) {
                node.bounds = bounds(&loc["bounds"]);
                node.container = loc["offset_container_id"].as_i64().filter(|c| *c > 0);
            }
        }
    }

    fn clear(&mut self, id: i64) {
        if let Some(node) = self.nodes.remove(&id) {
            for child in node.children {
                self.clear(child);
            }
        }
    }

    /// The node's frame in the page, CSS pixels from its top-left.
    pub(crate) fn absolute(&self, id: i64) -> (f64, f64, f64, f64) {
        let Some(node) = self.nodes.get(&id) else { return (0., 0., 0., 0.) };
        let (mut x, mut y, w, h) = node.bounds;
        let mut container = node.container;
        let mut guard = 0;
        while let Some(c) = container.filter(|c| *c != id && guard < 64) {
            let Some(parent) = self.nodes.get(&c) else { break };
            x += parent.bounds.0;
            y += parent.bounds.1;
            container = parent.container;
            guard += 1;
        }
        (x, y, w, h)
    }
}

/// The AppKit role for a Chromium role; `None` for nodes that only carry text runs of their
/// parent and are left out.
#[cfg(target_os = "macos")]
pub(crate) fn ns_role(role: &str) -> Option<&'static str> {
    Some(match role {
        "inlineTextBox" | "none" | "ignored" => return None,
        "rootWebArea" | "webArea" => "AXWebArea",
        "link" => "AXLink",
        "button" | "toggleButton" => "AXButton",
        "heading" => "AXHeading",
        "staticText" | "labelText" => "AXStaticText",
        "textField" | "searchBox" | "textFieldWithComboBox" => "AXTextField",
        "image" => "AXImage",
        "checkBox" | "switch" => "AXCheckBox",
        "radioButton" => "AXRadioButton",
        "comboBoxSelect" | "popUpButton" => "AXPopUpButton",
        "list" | "listBox" => "AXList",
        "table" | "grid" => "AXTable",
        "row" => "AXRow",
        "cell" | "gridCell" => "AXCell",
        "slider" => "AXSlider",
        _ => "AXGroup",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn updates_build_the_tree_and_frames_add_up_containers() {
        let mut tree = Tree::default();
        tree.apply(&json!({"updates": [{"root_id": 1, "nodes": [
            {"id": 1, "role": "rootWebArea", "attributes": {"name": "Page"}, "location": {"x": 0, "y": 0, "width": 800, "height": 600}, "child_ids": [2]},
            {"id": 2, "role": "button", "attributes": {"name": "Go"}, "location": {"x": 10, "y": 20, "width": 50, "height": 30}, "offset_container_id": 3, "child_ids": []},
            {"id": 3, "role": "genericContainer", "location": {"x": 100, "y": 5, "width": 10, "height": 10}, "child_ids": []}
        ]}]}));
        assert_eq!(tree.root, Some(1));
        assert_eq!(tree.nodes[&2].name, "Go");
        assert_eq!(tree.absolute(2), (110., 25., 50., 30.));
        tree.locations(&json!([{"id": 2, "new_location": {"bounds": {"x": 0, "y": 0, "width": 5, "height": 5}}}]));
        assert_eq!(tree.absolute(2), (0., 0., 5., 5.));
        tree.apply(&json!({"updates": [{"node_id_to_clear": 1, "nodes": []}]}));
        assert!(!tree.nodes.contains_key(&2));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn roles_map_to_appkit() {
        assert_eq!(ns_role("link"), Some("AXLink"));
        assert_eq!(ns_role("inlineTextBox"), None);
        assert_eq!(ns_role("article"), Some("AXGroup"));
    }
}
