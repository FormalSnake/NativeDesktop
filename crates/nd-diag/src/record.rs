//! The flight recorder's rings: inputs, frame timings, trees and layout diffs per window.

use std::collections::{BTreeMap, HashMap, VecDeque};

use nd_core::Rect;
use nd_ui::TreeNode;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const INPUTS: usize = 200;
pub const FRAMES: usize = 120;

/// One input event as the framework's window received it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct InputRecord {
    /// Event timestamp, seconds on the host's monotonic clock.
    pub t: f64,
    /// The OS window number.
    pub window: i64,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<[f64; 2]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_code: Option<u16>,
    /// Typed characters; redacted in release builds unless `ND_DIAG_KEEP_TEXT=1`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    pub modifiers: u64,
}

/// One frame of one window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FrameRecord {
    pub frame: u64,
    /// When the frame was due on screen, seconds on the host's monotonic clock.
    pub target: f64,
    pub interval_ms: f64,
    /// Time spent rendering, laying out and committing.
    pub work_ms: f64,
    /// The work overran the display interval.
    pub missed: bool,
}

#[derive(Clone, Copy, PartialEq)]
struct Flat {
    parent: Option<u64>,
    frame: Option<Rect>,
}

#[derive(Default)]
struct WindowRecord {
    title: String,
    tree: Option<Value>,
    flat: HashMap<u64, Flat>,
    frames: VecDeque<FrameRecord>,
    layout: VecDeque<Value>,
}

#[derive(Default)]
pub(crate) struct Recorder {
    inputs: VecDeque<InputRecord>,
    windows: BTreeMap<u64, WindowRecord>,
}

fn push<T>(ring: &mut VecDeque<T>, cap: usize, item: T) {
    if ring.len() == cap {
        ring.pop_front();
    }
    ring.push_back(item);
}

fn rect(r: Option<Rect>) -> Value {
    r.map_or(Value::Null, |r| json!([r.origin.x, r.origin.y, r.size.width, r.size.height]))
}

/// The tree as `tree.json` and the automation schema carry it.
pub fn tree_json(node: &TreeNode) -> Value {
    let mut v = json!({ "id": node.id.0, "role": node.role, "kind": node.kind, "frame": rect(node.frame) });
    let map = v.as_object_mut().expect("object");
    if let Some(name) = &node.name {
        map.insert("name".into(), name.as_str().into());
    }
    if let Some(id) = &node.test_id {
        map.insert("testId".into(), id.as_str().into());
    }
    if node.focused {
        map.insert("focused".into(), true.into());
    }
    if !node.children.is_empty() {
        map.insert("children".into(), node.children.iter().map(tree_json).collect());
    }
    v
}

fn flatten(node: &TreeNode, parent: Option<u64>, out: &mut HashMap<u64, Flat>) {
    out.insert(node.id.0, Flat { parent, frame: node.frame });
    for child in &node.children {
        flatten(child, Some(node.id.0), out);
    }
}

/// What changed between two committed trees: nodes added, removed, moved or resized.
fn layout_diff(before: &HashMap<u64, Flat>, after: &HashMap<u64, Flat>) -> Value {
    let mut added: Vec<Value> = Vec::new();
    let mut changed: Vec<Value> = Vec::new();
    for (id, now) in after {
        match before.get(id) {
            None => added.push(json!({ "id": id, "parent": now.parent, "frame": rect(now.frame) })),
            Some(was) if was != now => changed.push(json!({
                "id": id,
                "from": rect(was.frame),
                "to": rect(now.frame),
                "reparented": was.parent != now.parent,
            })),
            Some(_) => {}
        }
    }
    let mut removed: Vec<u64> = before.keys().filter(|id| !after.contains_key(id)).copied().collect();
    let key = |v: &Value| v["id"].as_u64().unwrap_or(0);
    added.sort_by_key(key);
    changed.sort_by_key(key);
    removed.sort_unstable();
    json!({ "added": added, "changed": changed, "removed": removed })
}

impl Recorder {
    pub fn input(&mut self, input: InputRecord) {
        push(&mut self.inputs, INPUTS, input);
    }

    pub fn frame(&mut self, window: u64, title: &str, frame: FrameRecord, tree: Option<&TreeNode>) {
        let w = self.windows.entry(window).or_default();
        if w.title != title {
            w.title = title.to_owned();
        }
        push(&mut w.frames, FRAMES, frame);
        let Some(tree) = tree else { return };
        let mut flat = HashMap::new();
        flatten(tree, None, &mut flat);
        let mut diff = layout_diff(&w.flat, &flat);
        diff.as_object_mut().expect("object").insert("frame".into(), frame.frame.into());
        diff.as_object_mut().expect("object").insert("window".into(), window.into());
        push(&mut w.layout, FRAMES, diff);
        w.flat = flat;
        w.tree = Some(tree_json(tree));
    }

    pub fn close_window(&mut self, window: u64) {
        self.windows.remove(&window);
    }

    pub fn input_count(&self) -> usize {
        self.inputs.len()
    }

    pub fn inputs(&self) -> Vec<InputRecord> {
        self.inputs.iter().cloned().collect()
    }

    pub fn trees(&self) -> Vec<Value> {
        self.windows
            .iter()
            .map(|(id, w)| json!({ "window": id, "title": w.title, "root": w.tree.clone().unwrap_or(Value::Null) }))
            .collect()
    }

    pub fn frames(&self) -> BTreeMap<u64, Vec<FrameRecord>> {
        self.windows.iter().map(|(id, w)| (*id, w.frames.iter().copied().collect())).collect()
    }

    pub fn layout(&self) -> Vec<Value> {
        self.windows.values().flat_map(|w| w.layout.iter().cloned()).collect()
    }
}
