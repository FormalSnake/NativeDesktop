//! The split-pane tree: a binary tree of splits and leaves plus a focused leaf and an id counter.
//!
//! Trees are immutable values shared through `Rc`. Every op takes `&PaneTree` and returns a
//! `PaneTree`; when nothing changed (unknown id, already focused, same clamped ratio) the result
//! is a clone of the SAME `Rc`, so [`PaneTree::same`] holds. That identity is what stops a native
//! divider echo of the current ratio from looping through a render and persist cycle. Subtrees an
//! op did not touch are shared with the input (`PaneNode::same`).

use std::ops::Deref;
use std::rc::Rc;

use nd_gpui::gpui::SharedString;
use serde::{Deserialize, Serialize};

pub type PaneId = SharedString;

/// The direction a split lays its two children out in. `Horizontal` puts side 0 on the left.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Axis {
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

impl Direction {
    fn axis(self) -> Axis {
        match self {
            Direction::Left | Direction::Right => Axis::Horizontal,
            Direction::Up | Direction::Down => Axis::Vertical,
        }
    }

    fn forward(self) -> bool {
        matches!(self, Direction::Right | Direction::Down)
    }
}

#[derive(Debug, PartialEq)]
pub enum Node<T> {
    Leaf { id: PaneId, data: T },
    Split { id: PaneId, axis: Axis, ratio: f32, children: [PaneNode<T>; 2] },
}

impl<T> Node<T> {
    pub fn id(&self) -> &PaneId {
        match self {
            Node::Leaf { id, .. } | Node::Split { id, .. } => id,
        }
    }

    pub fn is_leaf(&self) -> bool {
        matches!(self, Node::Leaf { .. })
    }
}

/// A shared, immutable tree node. Cloning is a refcount bump.
pub struct PaneNode<T>(Rc<Node<T>>);

impl<T> Clone for PaneNode<T> {
    fn clone(&self) -> Self {
        PaneNode(Rc::clone(&self.0))
    }
}

impl<T> Deref for PaneNode<T> {
    type Target = Node<T>;
    fn deref(&self) -> &Node<T> {
        &self.0
    }
}

impl<T: PartialEq> PartialEq for PaneNode<T> {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0) || *self.0 == *other.0
    }
}

impl<T: std::fmt::Debug> std::fmt::Debug for PaneNode<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl<T> PaneNode<T> {
    pub fn leaf(id: impl Into<PaneId>, data: T) -> Self {
        PaneNode(Rc::new(Node::Leaf { id: id.into(), data }))
    }

    pub fn split(id: impl Into<PaneId>, axis: Axis, ratio: f32, children: [PaneNode<T>; 2]) -> Self {
        PaneNode(Rc::new(Node::Split { id: id.into(), axis, ratio, children }))
    }

    /// True when both handles point at the same allocation.
    pub fn same(&self, other: &PaneNode<T>) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }

    fn with_children(&self, children: [PaneNode<T>; 2]) -> PaneNode<T> {
        match &*self.0 {
            Node::Split { id, axis, ratio, .. } => PaneNode::split(id.clone(), *axis, *ratio, children),
            Node::Leaf { .. } => self.clone(),
        }
    }
}

struct Inner<T> {
    root: Option<PaneNode<T>>,
    focused: PaneId,
    next_id: u64,
}

/// The whole model: root, focused leaf id (empty when there is no leaf) and the counter that
/// mints leaf ids (`"3"`) and split ids (`"s3"`).
pub struct PaneTree<T>(Rc<Inner<T>>);

impl<T> Clone for PaneTree<T> {
    fn clone(&self) -> Self {
        PaneTree(Rc::clone(&self.0))
    }
}

impl<T: PartialEq> PartialEq for PaneTree<T> {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
            || (self.0.focused == other.0.focused && self.0.next_id == other.0.next_id && self.0.root == other.0.root)
    }
}

impl<T: std::fmt::Debug> std::fmt::Debug for PaneTree<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PaneTree")
            .field("root", &self.0.root)
            .field("focused", &self.0.focused)
            .field("next_id", &self.0.next_id)
            .finish()
    }
}

impl<T> PaneTree<T> {
    pub fn new(root: Option<PaneNode<T>>, focused_id: impl Into<PaneId>, next_id: u64) -> Self {
        PaneTree(Rc::new(Inner { root, focused: focused_id.into(), next_id }))
    }

    pub fn root(&self) -> Option<&PaneNode<T>> {
        self.0.root.as_ref()
    }

    pub fn focused_id(&self) -> &PaneId {
        &self.0.focused
    }

    pub fn next_id(&self) -> u64 {
        self.0.next_id
    }

    /// True when both handles point at the same allocation: the "nothing changed" test.
    pub fn same(&self, other: &PaneTree<T>) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }

    /// A new tree over the same root with another focused id. Does not validate `id`.
    pub fn with_focused(&self, id: impl Into<PaneId>) -> PaneTree<T> {
        PaneTree::new(self.0.root.clone(), id, self.0.next_id)
    }

    pub(crate) fn with_root(&self, root: Option<PaneNode<T>>) -> PaneTree<T> {
        PaneTree::new(root, self.0.focused.clone(), self.0.next_id)
    }
}

/// A leaf borrowed out of a tree.
#[derive(Debug)]
pub struct Leaf<'a, T> {
    pub id: &'a PaneId,
    pub data: &'a T,
}

/// Non-finite ratios panic a Debug host, so this is a safety boundary: non-finite becomes 0.5,
/// everything else clamps to [0.05, 0.95] so neither side collapses to an undraggable sliver.
pub fn clamp_pane_ratio(ratio: f32) -> f32 {
    if !ratio.is_finite() {
        return 0.5;
    }
    ratio.clamp(0.05, 0.95)
}

pub fn empty_panes<T>() -> PaneTree<T> {
    PaneTree::new(None, "", 1)
}

/// Parses `7` and `s7` the way minted ids look; anything else is not a counter value.
fn counter_value(id: &str) -> Option<u64> {
    let digits = id.strip_prefix('s').unwrap_or(id);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// Balanced horizontal-split tree over `leaves`, even column widths via per-split ratios. Leaf
/// ids are `"1"` to `"n"`.
pub fn seed_panes<T>(leaves: Vec<T>) -> PaneTree<T> {
    seed_panes_with(leaves, |_, i| (i + 1).to_string())
}

/// [`seed_panes`] with caller-chosen leaf ids. The id counter starts past every numeric id
/// issued, so ids minted later cannot collide with custom ones.
pub fn seed_panes_with<T>(leaves: Vec<T>, id: impl Fn(&T, usize) -> String) -> PaneTree<T> {
    if leaves.is_empty() {
        return empty_panes();
    }
    let count = leaves.len() as u64;
    let nodes: Vec<PaneNode<T>> = leaves
        .into_iter()
        .enumerate()
        .map(|(i, data)| {
            let leaf_id = id(&data, i);
            PaneNode::leaf(leaf_id, data)
        })
        .collect();
    let max_numeric = nodes.iter().filter_map(|n| counter_value(n.id())).max().unwrap_or(0);
    let mut counter = count.max(max_numeric) + 1;

    fn build<T>(nodes: &[PaneNode<T>], lo: usize, hi: usize, counter: &mut u64) -> PaneNode<T> {
        if hi - lo == 1 {
            return nodes[lo].clone();
        }
        let mid = lo + (hi - lo).div_ceil(2);
        let split_id = format!("s{counter}");
        *counter += 1;
        let ratio = (mid - lo) as f32 / (hi - lo) as f32;
        let a = build(nodes, lo, mid, counter);
        let b = build(nodes, mid, hi, counter);
        PaneNode::split(split_id, Axis::Horizontal, ratio, [a, b])
    }

    let root = build(&nodes, 0, nodes.len(), &mut counter);
    PaneTree::new(Some(root), nodes[0].id().clone(), counter)
}

/// The node `id` inside `node`'s subtree.
pub fn find_pane_node<'a, T>(node: &'a PaneNode<T>, id: &str) -> Option<&'a PaneNode<T>> {
    if node.id().as_ref() == id {
        return Some(node);
    }
    match &**node {
        Node::Split { children, .. } => find_pane_node(&children[0], id).or_else(|| find_pane_node(&children[1], id)),
        Node::Leaf { .. } => None,
    }
}

fn find_in<'a, T>(m: &'a PaneTree<T>, id: &str) -> Option<&'a PaneNode<T>> {
    m.root().and_then(|root| find_pane_node(root, id))
}

/// The split holding `id` as a direct child, or `None` when `id` is the root or absent.
pub fn parent_pane_split<'a, T>(root: &'a PaneNode<T>, id: &str) -> Option<&'a PaneNode<T>> {
    match &**root {
        Node::Leaf { .. } => None,
        Node::Split { children, .. } => {
            if children[0].id().as_ref() == id || children[1].id().as_ref() == id {
                return Some(root);
            }
            parent_pane_split(&children[0], id).or_else(|| parent_pane_split(&children[1], id))
        }
    }
}

fn first_leaf<T>(node: &PaneNode<T>) -> &PaneNode<T> {
    match &**node {
        Node::Leaf { .. } => node,
        Node::Split { children, .. } => first_leaf(&children[0]),
    }
}

/// Path-copies the spine down to `id`, replacing that node with `f(node)`. `None` when `id` is
/// not in the tree. Siblings along the spine are shared, not copied.
pub(crate) fn map_pane_node<T>(
    node: &PaneNode<T>,
    id: &str,
    f: &mut dyn FnMut(&PaneNode<T>) -> PaneNode<T>,
) -> Option<PaneNode<T>> {
    if node.id().as_ref() == id {
        return Some(f(node));
    }
    let Node::Split { children: [a, b], .. } = &**node else {
        return None;
    };
    if let Some(ma) = map_pane_node(a, id, f) {
        return Some(node.with_children([ma, b.clone()]));
    }
    map_pane_node(b, id, f).map(|mb| node.with_children([a.clone(), mb]))
}

/// Splits the leaf `pane_id` in two: the existing leaf keeps side 0, the new leaf (holding
/// `data`, id `id` or the next counter value) takes side 1 and the focus.
pub fn split_pane<T>(m: &PaneTree<T>, pane_id: &str, axis: Axis, data: T, id: Option<&str>) -> PaneTree<T> {
    let Some(root) = m.root() else { return m.clone() };
    if !find_pane_node(root, pane_id).is_some_and(|n| n.is_leaf()) {
        return m.clone();
    }
    let new_id: PaneId = id.map_or_else(|| m.next_id().to_string().into(), |s| s.to_string().into());
    let new_leaf = PaneNode::leaf(new_id.clone(), data);
    let split_id = format!("s{}", m.next_id());
    let root = map_pane_node(root, pane_id, &mut |leaf| {
        PaneNode::split(split_id.clone(), axis, 0.5, [leaf.clone(), new_leaf.clone()])
    });
    PaneTree::new(root, new_id, m.next_id() + 1)
}

enum Removal<T> {
    Absent,
    Itself,
    Done { node: PaneNode<T>, sibling: PaneNode<T> },
}

fn remove_from<T>(node: &PaneNode<T>, id: &str) -> Removal<T> {
    let Node::Split { children: [a, b], .. } = &**node else {
        return if node.id().as_ref() == id { Removal::Itself } else { Removal::Absent };
    };
    match remove_from(a, id) {
        Removal::Itself => return Removal::Done { node: b.clone(), sibling: b.clone() },
        Removal::Done { node: ra, sibling } => {
            return Removal::Done { node: node.with_children([ra, b.clone()]), sibling };
        }
        Removal::Absent => {}
    }
    match remove_from(b, id) {
        Removal::Itself => Removal::Done { node: a.clone(), sibling: a.clone() },
        Removal::Done { node: rb, sibling } => Removal::Done { node: node.with_children([a.clone(), rb]), sibling },
        Removal::Absent => Removal::Absent,
    }
}

/// Removes the leaf `pane_id`; its sibling subtree replaces the parent split. Closing the
/// focused leaf moves the focus to the sibling's first leaf.
pub fn close_pane<T>(m: &PaneTree<T>, pane_id: &str) -> PaneTree<T> {
    let Some(root) = m.root() else { return m.clone() };
    match remove_from(root, pane_id) {
        Removal::Absent => m.clone(),
        Removal::Itself => PaneTree::new(None, "", m.next_id()),
        Removal::Done { node, sibling } => {
            let focused = if m.focused_id().as_ref() == pane_id {
                first_leaf(&sibling).id().clone()
            } else {
                m.focused_id().clone()
            };
            PaneTree::new(Some(node), focused, m.next_id())
        }
    }
}

pub fn focus_pane<T>(m: &PaneTree<T>, pane_id: &str) -> PaneTree<T> {
    if m.focused_id().as_ref() == pane_id {
        return m.clone();
    }
    match find_in(m, pane_id) {
        Some(node) if node.is_leaf() => m.with_focused(pane_id.to_string()),
        _ => m.clone(),
    }
}

pub fn focus_pane_at<T>(m: &PaneTree<T>, index: usize) -> PaneTree<T> {
    match pane_id_at(m, index) {
        Some(id) => focus_pane(m, &id),
        None => m.clone(),
    }
}

/// The id of the `index`th leaf in layout order.
pub fn pane_id_at<T>(m: &PaneTree<T>, index: usize) -> Option<PaneId> {
    pane_leaves(m).get(index).map(|leaf| leaf.id.clone())
}

fn collect_path<T>(node: &PaneNode<T>, id: &str, path: &mut Vec<PaneNode<T>>) -> bool {
    path.push(node.clone());
    if node.id().as_ref() == id {
        return true;
    }
    if let Node::Split { children, .. } = &**node
        && (collect_path(&children[0], id, path) || collect_path(&children[1], id, path))
    {
        return true;
    }
    path.pop();
    false
}

/// Directional focus by walking ancestors: finds the nearest ancestor split on the move's axis
/// where the focused subtree sits on the departing side, then descends into the other child
/// along the shared edge. Exact for a binary tree, no layout round-trip.
pub fn focus_neighbor<T>(m: &PaneTree<T>, dir: Direction) -> PaneTree<T> {
    let Some(root) = m.root() else { return m.clone() };
    if m.focused_id().is_empty() {
        return m.clone();
    }
    let mut path = Vec::new();
    if !collect_path(root, m.focused_id(), &mut path) {
        return m.clone();
    }
    let axis = dir.axis();
    let forward = dir.forward();
    for i in (0..path.len() - 1).rev() {
        let Node::Split { axis: split_axis, children, .. } = &*path[i] else { continue };
        let child_index = if children[0].same(&path[i + 1]) { 0 } else { 1 };
        if *split_axis != axis || child_index != usize::from(!forward) {
            continue;
        }
        let mut target = children[usize::from(forward)].clone();
        while let Node::Split { axis: a, children, .. } = &*target.clone() {
            target = if *a == axis { children[usize::from(!forward)].clone() } else { children[0].clone() };
        }
        return focus_pane(m, target.id());
    }
    m.clone()
}

pub fn set_pane_ratio<T>(m: &PaneTree<T>, split_id: &str, ratio: f32) -> PaneTree<T> {
    let Some(root) = m.root() else { return m.clone() };
    let Some(node) = find_pane_node(root, split_id) else { return m.clone() };
    let Node::Split { ratio: current, .. } = &**node else { return m.clone() };
    let clamped = clamp_pane_ratio(ratio);
    if clamped == *current {
        return m.clone();
    }
    let root = map_pane_node(root, split_id, &mut |split| match &**split {
        Node::Split { id, axis, children, .. } => PaneNode::split(id.clone(), *axis, clamped, children.clone()),
        Node::Leaf { .. } => split.clone(),
    });
    m.with_root(root)
}

/// Replaces a leaf's data with `f(current)`. Equal data is a same-reference no-op.
pub fn update_pane<T: PartialEq>(m: &PaneTree<T>, pane_id: &str, f: impl FnOnce(&T) -> T) -> PaneTree<T> {
    let Some(root) = m.root() else { return m.clone() };
    let Some(node) = find_pane_node(root, pane_id) else { return m.clone() };
    let Node::Leaf { data, .. } = &**node else { return m.clone() };
    let next = f(data);
    if next == *data {
        return m.clone();
    }
    let mut next = Some(next);
    let root = map_pane_node(root, pane_id, &mut |leaf| match next.take() {
        Some(data) => PaneNode::leaf(leaf.id().clone(), data),
        None => leaf.clone(),
    });
    m.with_root(root)
}

/// Leaves in layout order (side 0 before side 1, depth-first).
pub fn pane_leaves<T>(m: &PaneTree<T>) -> Vec<Leaf<'_, T>> {
    fn walk<'a, T>(node: &'a PaneNode<T>, out: &mut Vec<Leaf<'a, T>>) {
        match &**node {
            Node::Leaf { id, data } => out.push(Leaf { id, data }),
            Node::Split { children, .. } => {
                walk(&children[0], out);
                walk(&children[1], out);
            }
        }
    }
    let mut out = Vec::new();
    if let Some(root) = m.root() {
        walk(root, &mut out);
    }
    out
}

/// Structural equality: ids, kinds and axes, ignoring ratios and leaf data. This is the
/// "structural change" test for a persist-on-close flush.
pub fn same_pane_shape<T>(a: Option<&PaneNode<T>>, b: Option<&PaneNode<T>>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => {
            if a.same(b) {
                return true;
            }
            match (&**a, &**b) {
                (Node::Leaf { id: x, .. }, Node::Leaf { id: y, .. }) => x == y,
                (
                    Node::Split { id: x, axis: ax, children: ca, .. },
                    Node::Split { id: y, axis: ay, children: cb, .. },
                ) => {
                    x == y
                        && ax == ay
                        && same_pane_shape(Some(&ca[0]), Some(&cb[0]))
                        && same_pane_shape(Some(&ca[1]), Some(&cb[1]))
                }
                _ => false,
            }
        }
        _ => false,
    }
}

fn default_ratio() -> f32 {
    0.5
}

/// Plain-data form of a node, as persisted. Keys match the old JSON form (`kind`, `orientation`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum SnapshotNode<T> {
    Leaf {
        id: PaneId,
        data: T,
    },
    Split {
        id: PaneId,
        orientation: Axis,
        #[serde(default = "default_ratio")]
        ratio: f32,
        children: Box<[SnapshotNode<T>; 2]>,
    },
}

/// Plain-data form of a [`PaneTree`], as persisted.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaneSnapshot<T> {
    #[serde(default = "no_root", skip_serializing_if = "Option::is_none")]
    pub root: Option<SnapshotNode<T>>,
    #[serde(default)]
    pub focused_id: PaneId,
    #[serde(default)]
    pub next_id: u64,
}

fn no_root<T>() -> Option<SnapshotNode<T>> {
    None
}

pub fn serialize_panes<T: Clone>(m: &PaneTree<T>) -> PaneSnapshot<T> {
    fn copy<T: Clone>(node: &PaneNode<T>) -> SnapshotNode<T> {
        match &**node {
            Node::Leaf { id, data } => SnapshotNode::Leaf { id: id.clone(), data: data.clone() },
            Node::Split { id, axis, ratio, children } => SnapshotNode::Split {
                id: id.clone(),
                orientation: *axis,
                ratio: *ratio,
                children: Box::new([copy(&children[0]), copy(&children[1])]),
            },
        }
    }
    PaneSnapshot { root: m.root().map(copy), focused_id: m.focused_id().clone(), next_id: m.next_id() }
}

/// Rebuilds a tree from persisted data. A node with an empty id rejects the whole tree back to
/// [`empty_panes`]; ratios are clamped, focus falls back to the first leaf, and the counter is
/// recomputed past every numeric id.
pub fn migrate_panes<T>(raw: PaneSnapshot<T>) -> PaneTree<T> {
    migrate_panes_with(raw, Some)
}

/// [`migrate_panes`] where `accept` sees each leaf's data in layout order and may repair it
/// (`Some`) or reject the whole tree (`None`).
pub fn migrate_panes_with<T>(raw: PaneSnapshot<T>, mut accept: impl FnMut(T) -> Option<T>) -> PaneTree<T> {
    fn convert<T>(
        node: SnapshotNode<T>,
        accept: &mut dyn FnMut(T) -> Option<T>,
        max_numeric: &mut u64,
    ) -> Option<PaneNode<T>> {
        match node {
            SnapshotNode::Leaf { id, data } => {
                if id.is_empty() {
                    return None;
                }
                let data = accept(data)?;
                *max_numeric = (*max_numeric).max(counter_value(&id).unwrap_or(0));
                Some(PaneNode::leaf(id, data))
            }
            SnapshotNode::Split { id, orientation, ratio, children } => {
                if id.is_empty() {
                    return None;
                }
                let [a, b] = *children;
                let a = convert(a, accept, max_numeric)?;
                let b = convert(b, accept, max_numeric)?;
                *max_numeric = (*max_numeric).max(counter_value(&id).unwrap_or(0));
                Some(PaneNode::split(id, orientation, clamp_pane_ratio(ratio), [a, b]))
            }
        }
    }

    let mut max_numeric = 0;
    let root = match raw.root {
        None => None,
        Some(node) => match convert(node, &mut accept, &mut max_numeric) {
            Some(root) => Some(root),
            None => return empty_panes(),
        },
    };
    let model = PaneTree::new(root, "", 1);
    let leaves = pane_leaves(&model);
    let focused = if leaves.iter().any(|l| *l.id == raw.focused_id) {
        raw.focused_id.clone()
    } else {
        leaves.first().map(|l| l.id.clone()).unwrap_or_default()
    };
    let next_id = raw.next_id.max(max_numeric + 1).max(1);
    PaneTree::new(model.root().cloned(), focused, next_id)
}
