//! The flight recorder's rings: inputs, frame timings, and per window the last committed
//! semantic tree (the automation schema) and the layout diffs between frames.

use std::collections::{BTreeMap, HashMap, VecDeque};

use nd_core::Rect;
use nd_core::semantic::SemanticNode;
use nd_core::semantic::SemanticWindow;
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

/// What a frame HUD shows: a window's frames over the last second.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FrameSummary {
    /// Frames committed per second.
    pub fps: f64,
    /// Mean time rendering, laying out and committing a frame.
    pub work_ms: f64,
    /// The slowest frame's work.
    pub worst_ms: f64,
    /// Frames whose work overran the display interval.
    pub dropped: usize,
    /// The display interval of the latest frame.
    pub interval_ms: f64,
}

impl FrameSummary {
    /// Over the frames due within a second of the latest, oldest first as they are recorded.
    pub fn of(frames: &[FrameRecord]) -> Self {
        let Some(last) = frames.last() else { return Self::default() };
        let recent: Vec<&FrameRecord> = frames.iter().filter(|f| last.target - f.target < 1.).collect();
        let span = last.target - recent[0].target;
        let fps = if span > 0. { (recent.len() - 1) as f64 / span } else { 0. };
        let work: Vec<f64> = recent.iter().map(|f| f.work_ms).collect();
        Self {
            fps,
            work_ms: work.iter().sum::<f64>() / work.len() as f64,
            worst_ms: work.iter().copied().fold(0., f64::max),
            dropped: recent.iter().filter(|f| f.missed).count(),
            interval_ms: last.interval_ms,
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
struct Flat {
    parent: Option<u64>,
    frame: Option<Rect>,
}

/// What changed between two committed trees, kept typed: it becomes JSON only when a bundle is
/// written, so recording costs a hash map walk per frame, not a serialization.
struct LayoutDiff {
    frame: u64,
    window: u64,
    added: Vec<(u64, Flat)>,
    changed: Vec<(u64, Flat, Flat)>,
    removed: Vec<u64>,
}

impl LayoutDiff {
    fn to_json(&self) -> Value {
        let added: Vec<Value> = self
            .added
            .iter()
            .map(|(id, now)| json!({ "id": id, "parent": now.parent, "frame": rect(now.frame) }))
            .collect();
        let changed: Vec<Value> = self
            .changed
            .iter()
            .map(|(id, was, now)| {
                json!({
                    "id": id,
                    "from": rect(was.frame),
                    "to": rect(now.frame),
                    "reparented": was.parent != now.parent,
                })
            })
            .collect();
        json!({
            "added": added,
            "changed": changed,
            "removed": self.removed,
            "frame": self.frame,
            "window": self.window,
        })
    }
}

#[derive(Default)]
struct WindowRecord {
    tree: Option<SemanticWindow>,
    flat: HashMap<u64, Flat>,
    frames: VecDeque<FrameRecord>,
    layout: VecDeque<LayoutDiff>,
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

fn flatten(node: &SemanticNode, parent: Option<u64>, out: &mut HashMap<u64, Flat>) {
    out.insert(node.id.0, Flat { parent, frame: node.bounds });
    for child in &node.children {
        flatten(child, Some(node.id.0), out);
    }
}

/// Nodes added, removed, moved or resized between two committed trees, sorted by id.
fn layout_diff(before: &HashMap<u64, Flat>, after: &HashMap<u64, Flat>, frame: u64, window: u64) -> LayoutDiff {
    let mut added = Vec::new();
    let mut changed = Vec::new();
    for (id, now) in after {
        match before.get(id) {
            None => added.push((*id, *now)),
            Some(was) if was != now => changed.push((*id, *was, *now)),
            Some(_) => {}
        }
    }
    let mut removed: Vec<u64> = before.keys().filter(|id| !after.contains_key(id)).copied().collect();
    added.sort_by_key(|a| a.0);
    changed.sort_by_key(|c| c.0);
    removed.sort_unstable();
    LayoutDiff { frame, window, added, changed, removed }
}

impl Recorder {
    pub fn input(&mut self, input: InputRecord) {
        push(&mut self.inputs, INPUTS, input);
    }

    pub fn frame(&mut self, window: u64, frame: FrameRecord, tree: Option<&SemanticWindow>) {
        let w = self.windows.entry(window).or_default();
        push(&mut w.frames, FRAMES, frame);
        let Some(tree) = tree else { return };
        let mut flat = HashMap::new();
        flatten(&tree.root, None, &mut flat);
        let diff = layout_diff(&w.flat, &flat, frame.frame, window);
        push(&mut w.layout, FRAMES, diff);
        w.flat = flat;
        w.tree = Some(tree.clone());
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
        self.windows.values().filter_map(|w| w.tree.as_ref()).filter_map(|t| serde_json::to_value(t).ok()).collect()
    }

    pub fn window_frames(&self, window: u64) -> Vec<FrameRecord> {
        self.windows.get(&window).map(|w| w.frames.iter().copied().collect()).unwrap_or_default()
    }

    pub fn frames(&self) -> BTreeMap<u64, Vec<FrameRecord>> {
        self.windows.iter().map(|(id, w)| (*id, w.frames.iter().copied().collect())).collect()
    }

    pub fn layout(&self) -> Vec<Value> {
        self.windows.values().flat_map(|w| w.layout.iter().map(LayoutDiff::to_json)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(frame: u64, target: f64, work_ms: f64) -> FrameRecord {
        FrameRecord { frame, target, interval_ms: 16.667, work_ms, missed: work_ms > 16.667 }
    }

    #[test]
    fn a_summary_covers_the_last_second() {
        // An old frame, then 61 frames at 60 Hz: the last 60 fall within the second.
        let mut frames = vec![frame(0, -5., 90.)];
        frames.extend((0..61).map(|i| frame(i + 1, f64::from(i as u32) / 60., if i == 30 { 25. } else { 4. })));
        let s = FrameSummary::of(&frames);
        assert!((s.fps - 60.).abs() < 0.01, "{s:?}");
        assert_eq!(s.dropped, 1);
        assert_eq!(s.worst_ms, 25.);
        assert!((s.work_ms - (4. * 59. + 25.) / 60.).abs() < 1e-9);
        assert_eq!(s.interval_ms, 16.667);
    }

    #[test]
    fn no_frames_summarize_to_nothing() {
        assert_eq!(FrameSummary::of(&[]), FrameSummary::default());
        assert_eq!(FrameSummary::of(&[frame(1, 0., 3.)]).fps, 0.);
    }
}
