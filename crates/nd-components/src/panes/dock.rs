//! Dock model: a split tree whose leaves are panels, each holding a stack of tabs. It is exactly
//! `PaneTree<DockPanel<T>>`, so ratios, directional focus, collapse on close and migration come
//! from [`super::model`] unchanged; this module adds the panel and tab vocabulary on top.
//!
//! The identity contract of the model holds here too: an op that changes nothing returns a clone
//! of the same `Rc`. Docking a panel where it already sits, activating the active tab or moving a
//! tab to its current index all hand back the tree they were given, so a drop handler that fires
//! twice cannot loop a render and persist cycle.

use std::collections::HashSet;

use nd_gpui::gpui::SharedString;
use serde::{Deserialize, Serialize};

use super::model::{
    Axis, Node, PaneId, PaneNode, PaneSnapshot, PaneTree, clamp_pane_ratio, empty_panes, find_pane_node, map_pane_node,
    migrate_panes_with, pane_leaves, parent_pane_split, seed_panes_with, serialize_panes,
};

pub use super::model::close_pane as close_panel;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DockZone {
    Left,
    Right,
    Top,
    Bottom,
    Center,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DockEdgeZone {
    Left,
    Right,
    Top,
    Bottom,
}

impl DockEdgeZone {
    /// Axis and side each edge splits on. Side 0 is the leading child, so left and top put the
    /// incoming panel first.
    fn layout(self) -> (Axis, usize) {
        match self {
            DockEdgeZone::Left => (Axis::Horizontal, 0),
            DockEdgeZone::Right => (Axis::Horizontal, 1),
            DockEdgeZone::Top => (Axis::Vertical, 0),
            DockEdgeZone::Bottom => (Axis::Vertical, 1),
        }
    }
}

impl From<DockEdgeZone> for DockZone {
    fn from(zone: DockEdgeZone) -> Self {
        match zone {
            DockEdgeZone::Left => DockZone::Left,
            DockEdgeZone::Right => DockZone::Right,
            DockEdgeZone::Top => DockZone::Top,
            DockEdgeZone::Bottom => DockZone::Bottom,
        }
    }
}

impl DockZone {
    pub fn edge(self) -> Option<DockEdgeZone> {
        match self {
            DockZone::Left => Some(DockEdgeZone::Left),
            DockZone::Right => Some(DockEdgeZone::Right),
            DockZone::Top => Some(DockEdgeZone::Top),
            DockZone::Bottom => Some(DockEdgeZone::Bottom),
            DockZone::Center => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DockTab<T> {
    pub id: SharedString,
    pub title: SharedString,
    /// Platform icon name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<SharedString>,
    pub data: T,
}

impl<T> DockTab<T> {
    pub fn new(id: impl Into<SharedString>, title: impl Into<SharedString>, data: T) -> Self {
        DockTab { id: id.into(), title: title.into(), icon: None, data }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DockPanel<T> {
    pub tabs: Vec<DockTab<T>>,
    pub active_tab_id: SharedString,
}

pub type DockModel<T> = PaneTree<DockPanel<T>>;
pub type DockSnapshot<T> = PaneSnapshot<DockPanel<T>>;

#[derive(Debug)]
pub struct DockTabLocation<'a, T> {
    pub panel_id: &'a PaneId,
    pub index: usize,
    pub tab: &'a DockTab<T>,
}

pub fn empty_dock<T>() -> DockModel<T> {
    empty_panes()
}

/// A panel over `tabs`, active tab defaulting to the first. An empty tab list is not a legal
/// panel ([`close_tab`] collapses the panel instead).
pub fn dock_panel_of<T>(tabs: Vec<DockTab<T>>, active_tab_id: Option<&str>) -> DockPanel<T> {
    let active = match active_tab_id {
        Some(id) if tabs.iter().any(|t| t.id.as_ref() == id) => SharedString::from(id.to_string()),
        _ => tabs.first().map(|t| t.id.clone()).unwrap_or_default(),
    };
    DockPanel { tabs, active_tab_id: active }
}

/// Balanced horizontal dock over one panel per tab group, panel ids `"1"` to `"n"`. Groups with
/// no tabs are dropped, so every leaf starts with at least one tab.
pub fn seed_dock<T>(groups: Vec<Vec<DockTab<T>>>) -> DockModel<T> {
    seed_dock_with(groups, |_, i| (i + 1).to_string())
}

pub fn seed_dock_with<T>(groups: Vec<Vec<DockTab<T>>>, id: impl Fn(&DockPanel<T>, usize) -> String) -> DockModel<T> {
    let panels: Vec<DockPanel<T>> =
        groups.into_iter().filter(|g| !g.is_empty()).map(|g| dock_panel_of(g, None)).collect();
    seed_panes_with(panels, id)
}

/// Panels in layout order, as `(panel id, panel)`.
pub fn dock_panels<T>(m: &DockModel<T>) -> Vec<(&PaneId, &DockPanel<T>)> {
    pane_leaves(m).into_iter().map(|l| (l.id, l.data)).collect()
}

pub fn find_dock_panel<'a, T>(m: &'a DockModel<T>, panel_id: &str) -> Option<&'a DockPanel<T>> {
    let node = find_pane_node(m.root()?, panel_id)?;
    match &**node {
        Node::Leaf { data, .. } => Some(data),
        Node::Split { .. } => None,
    }
}

pub fn find_dock_tab<'a, T>(m: &'a DockModel<T>, tab_id: &str) -> Option<DockTabLocation<'a, T>> {
    pane_leaves(m).into_iter().find_map(|leaf| {
        let index = leaf.data.tabs.iter().position(|t| t.id.as_ref() == tab_id)?;
        Some(DockTabLocation { panel_id: leaf.id, index, tab: &leaf.data.tabs[index] })
    })
}

pub fn active_dock_tab<T>(panel: &DockPanel<T>) -> Option<&DockTab<T>> {
    panel.tabs.iter().find(|t| t.id == panel.active_tab_id)
}

/// Index of the active tab. Falls back to 0 so a panel whose active id went stale still renders
/// a selected tab.
pub fn active_dock_tab_index<T>(panel: &DockPanel<T>) -> usize {
    panel.tabs.iter().position(|t| t.id == panel.active_tab_id).unwrap_or(0)
}

/// Drops `tab_id` from `panel`. `None` when that was the last tab, the caller's signal to
/// collapse the panel instead.
fn without_tab<T: Clone>(panel: &DockPanel<T>, tab_id: &str) -> Option<DockPanel<T>> {
    let Some(index) = panel.tabs.iter().position(|t| t.id.as_ref() == tab_id) else {
        return Some(panel.clone());
    };
    let mut tabs = panel.tabs.clone();
    tabs.remove(index);
    if tabs.is_empty() {
        return None;
    }
    // Closing the active tab hands over to the one that slid into its place, or to the one
    // before it when the last tab in the stack went.
    let active = if panel.active_tab_id.as_ref() == tab_id {
        tabs.get(index).unwrap_or_else(|| &tabs[index - 1]).id.clone()
    } else {
        panel.active_tab_id.clone()
    };
    Some(DockPanel { tabs, active_tab_id: active })
}

fn with_tab<T: Clone>(panel: &DockPanel<T>, tab: DockTab<T>, index: usize, activate: bool) -> DockPanel<T> {
    let mut tabs = panel.tabs.clone();
    let at = index.min(tabs.len());
    let active = if activate { tab.id.clone() } else { panel.active_tab_id.clone() };
    tabs.insert(at, tab);
    DockPanel { tabs, active_tab_id: active }
}

fn set_panel<T>(m: &DockModel<T>, panel_id: &str, panel: DockPanel<T>) -> DockModel<T> {
    let Some(root) = m.root() else { return m.clone() };
    let mut panel = Some(panel);
    let root = map_pane_node(root, panel_id, &mut |leaf| match panel.take() {
        Some(panel) => PaneNode::leaf(leaf.id().clone(), panel),
        None => leaf.clone(),
    });
    m.with_root(root)
}

/// Inserts `node` beside the leaf `target_id` on `zone`'s side, minting the split id from the
/// counter the way `split_pane` does.
fn insert_beside<T>(
    m: &DockModel<T>,
    target_id: &str,
    zone: DockEdgeZone,
    node: PaneNode<DockPanel<T>>,
) -> DockModel<T> {
    let Some(root) = m.root() else { return m.clone() };
    let (axis, side) = zone.layout();
    let split_id = format!("s{}", m.next_id());
    let root = map_pane_node(root, target_id, &mut |target| {
        let children = if side == 0 { [node.clone(), target.clone()] } else { [target.clone(), node.clone()] };
        PaneNode::split(split_id.clone(), axis, 0.5, children)
    });
    PaneTree::new(root, m.focused_id().clone(), m.next_id() + 1)
}

/// True when `panel_id` is already the immediate `zone`-side neighbour of `target_id`.
/// Re-docking there would rebuild an identical tree under a fresh split id.
fn already_docked<T>(m: &DockModel<T>, panel_id: &str, target_id: &str, zone: DockEdgeZone) -> bool {
    let Some(parent) = m.root().and_then(|root| parent_pane_split(root, target_id)) else {
        return false;
    };
    let Node::Split { axis: parent_axis, children, .. } = &**parent else {
        return false;
    };
    let (axis, side) = zone.layout();
    *parent_axis == axis && children[side].id().as_ref() == panel_id && children[1 - side].id().as_ref() == target_id
}

/// Adds a tab to an existing panel and activates it. A tab id already used anywhere in the dock
/// is rejected (same tree), since every op here addresses tabs by id.
pub fn add_tab<T: Clone>(m: &DockModel<T>, panel_id: &str, tab: DockTab<T>, index: Option<usize>) -> DockModel<T> {
    let Some(panel) = find_dock_panel(m, panel_id) else { return m.clone() };
    if find_dock_tab(m, &tab.id).is_some() {
        return m.clone();
    }
    let at = index.unwrap_or(panel.tabs.len());
    set_panel(m, panel_id, with_tab(panel, tab, at, true))
}

/// Closes a tab; the last tab in a panel closes the panel, which collapses the split and gives
/// the space back to the sibling.
pub fn close_tab<T: Clone>(m: &DockModel<T>, tab_id: &str) -> DockModel<T> {
    let Some(found) = find_dock_tab(m, tab_id) else { return m.clone() };
    let Some(panel) = find_dock_panel(m, found.panel_id) else { return m.clone() };
    match without_tab(panel, tab_id) {
        Some(next) => set_panel(m, found.panel_id, next),
        None => close_panel(m, found.panel_id),
    }
}

/// Selects a tab inside its own panel. Focus is a separate op (`focus_pane`): activating a tab
/// in a background panel should not steal the focus ring.
pub fn activate_tab<T: Clone>(m: &DockModel<T>, tab_id: &str) -> DockModel<T> {
    let Some(found) = find_dock_tab(m, tab_id) else { return m.clone() };
    let Some(panel) = find_dock_panel(m, found.panel_id) else { return m.clone() };
    if panel.active_tab_id.as_ref() == tab_id {
        return m.clone();
    }
    set_panel(m, found.panel_id, DockPanel { tabs: panel.tabs.clone(), active_tab_id: found.tab.id.clone() })
}

/// Moves a tab to `index` of `target_panel_id`. Within one panel this is a reorder and leaves
/// the active tab alone; across panels the tab is activated in its new home, which takes the
/// focus with it, and a source panel left empty collapses.
pub fn move_tab<T: Clone>(m: &DockModel<T>, tab_id: &str, target_panel_id: &str, index: Option<usize>) -> DockModel<T> {
    let Some(found) = find_dock_tab(m, tab_id) else { return m.clone() };
    let Some(target) = find_dock_panel(m, target_panel_id) else { return m.clone() };
    let Some(source) = find_dock_panel(m, found.panel_id) else { return m.clone() };

    if found.panel_id.as_ref() == target_panel_id {
        let mut rest = source.tabs.clone();
        rest.remove(found.index);
        let at = index.unwrap_or(rest.len()).min(rest.len());
        if at == found.index {
            return m.clone();
        }
        rest.insert(at, found.tab.clone());
        return set_panel(m, found.panel_id, DockPanel { tabs: rest, active_tab_id: source.active_tab_id.clone() });
    }

    let at = index.unwrap_or(target.tabs.len());
    let landed = set_panel(m, target_panel_id, with_tab(target, found.tab.clone(), at, true));
    let next = match without_tab(source, tab_id) {
        Some(drained) => set_panel(&landed, found.panel_id, drained),
        None => close_panel(&landed, found.panel_id),
    };
    next.with_focused(target_panel_id.to_string())
}

/// Pulls a tab out of its panel into a new panel beside it. A panel holding one tab has nothing
/// to undock, so that is a same-tree no-op.
pub fn undock_tab<T: Clone>(m: &DockModel<T>, tab_id: &str, zone: DockEdgeZone) -> DockModel<T> {
    let Some(found) = find_dock_tab(m, tab_id) else { return m.clone() };
    let Some(source) = find_dock_panel(m, found.panel_id) else { return m.clone() };
    if source.tabs.len() < 2 {
        return m.clone();
    }
    let Some(remaining) = without_tab(source, tab_id) else { return m.clone() };
    let drained = set_panel(m, found.panel_id, remaining);
    let panel_id = m.next_id().to_string();
    let leaf = PaneNode::leaf(panel_id.clone(), dock_panel_of(vec![found.tab.clone()], None));
    insert_beside(&drained, found.panel_id, zone, leaf).with_focused(panel_id)
}

/// Relocates a whole panel against another one. `Center` merges its tabs into the target's
/// stack; the four edges split the target and take that side. [`apply_dock_drop`] routes a
/// drop here.
pub fn dock_panel<T: Clone>(m: &DockModel<T>, panel_id: &str, target_panel_id: &str, zone: DockZone) -> DockModel<T> {
    if panel_id == target_panel_id {
        return m.clone();
    }
    let (Some(source), Some(target)) = (find_dock_panel(m, panel_id), find_dock_panel(m, target_panel_id)) else {
        return m.clone();
    };

    let Some(edge) = zone.edge() else {
        let mut tabs = target.tabs.clone();
        tabs.extend(source.tabs.iter().cloned());
        let merged = DockPanel { tabs, active_tab_id: source.active_tab_id.clone() };
        let next = close_panel(&set_panel(m, target_panel_id, merged), panel_id);
        return next.with_focused(target_panel_id.to_string());
    };

    if already_docked(m, panel_id, target_panel_id, edge) {
        return m.clone();
    }
    let removed = close_panel(m, panel_id);
    let leaf = PaneNode::leaf(panel_id.to_string(), source.clone());
    insert_beside(&removed, target_panel_id, edge, leaf).with_focused(panel_id.to_string())
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DockRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

const DEFAULT_EDGE: f32 = 0.25;

/// Which zone a pointer at (`px`, `py`) lands in over `rect`. Pure geometry: the outer `edge`
/// fraction (default 0.25, clamped to [0.01, 0.5]) of each side is that side's zone, the middle
/// is `Center`. A point past an edge still names that edge; a degenerate rect is all `Center`.
pub fn hit_test_dock_zone(rect: DockRect, px: f32, py: f32, edge: Option<f32>) -> DockZone {
    let positive = |v: f32| v.is_finite() && v > 0.0;
    if !positive(rect.width) || !positive(rect.height) || !px.is_finite() || !py.is_finite() {
        return DockZone::Center;
    }
    let band = match edge {
        Some(e) if e.is_finite() => e.clamp(0.01, 0.5),
        _ => DEFAULT_EDGE,
    };
    let u = (px - rect.x) / rect.width;
    let v = (py - rect.y) / rect.height;
    let distances = [(DockZone::Left, u), (DockZone::Right, 1.0 - u), (DockZone::Top, v), (DockZone::Bottom, 1.0 - v)];
    let mut nearest = distances[0];
    for candidate in distances {
        if candidate.1 < nearest.1 {
            nearest = candidate;
        }
    }
    if nearest.1 < band { nearest.0 } else { DockZone::Center }
}

/// Pixel extent of the whole dock surface. Drops report a point in the target panel's own
/// coordinate space and nothing on the wire says how big that panel is, so panel rects are
/// derived from the split ratios, which are fractions of this.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DockSize {
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DockDragKind {
    Tab,
    Panel,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DockDrag {
    pub kind: DockDragKind,
    pub id: SharedString,
}

const DRAG_PREFIX: &str = "nd-dock:";

/// The drag payload a dock drag source carries. Namespaced because the drop side also receives
/// plain text dragged in from other applications, which must not be read as a tab id.
pub fn dock_drag_payload(kind: DockDragKind, id: &str) -> String {
    let kind = match kind {
        DockDragKind::Tab => "tab",
        DockDragKind::Panel => "panel",
    };
    format!("{DRAG_PREFIX}{kind}:{id}")
}

/// The inverse and the guard: anything not minted by [`dock_drag_payload`] is `None`, which
/// every drop path treats as "not ours, ignore".
pub fn parse_dock_drag(payload: &str) -> Option<DockDrag> {
    let rest = payload.strip_prefix(DRAG_PREFIX)?;
    let (kind, id) = rest.split_once(':')?;
    if id.is_empty() {
        return None;
    }
    let kind = match kind {
        "tab" => DockDragKind::Tab,
        "panel" => DockDragKind::Panel,
        _ => return None,
    };
    Some(DockDrag { kind, id: id.to_string().into() })
}

/// Every panel's rect inside a dock of `size`, in layout order, from the split ratios alone.
pub fn dock_panel_rects<T>(m: &DockModel<T>, size: DockSize) -> Vec<(PaneId, DockRect)> {
    fn walk<T>(node: &PaneNode<DockPanel<T>>, rect: DockRect, out: &mut Vec<(PaneId, DockRect)>) {
        match &**node {
            Node::Leaf { id, .. } => out.push((id.clone(), rect)),
            Node::Split { axis, ratio, children, .. } => {
                let ratio = clamp_pane_ratio(*ratio);
                match axis {
                    Axis::Horizontal => {
                        let lead = rect.width * ratio;
                        walk(&children[0], DockRect { width: lead, ..rect }, out);
                        walk(&children[1], DockRect { x: rect.x + lead, width: rect.width - lead, ..rect }, out);
                    }
                    Axis::Vertical => {
                        let lead = rect.height * ratio;
                        walk(&children[0], DockRect { height: lead, ..rect }, out);
                        walk(&children[1], DockRect { y: rect.y + lead, height: rect.height - lead, ..rect }, out);
                    }
                }
            }
        }
    }
    let finite = |v: f32| if v.is_finite() { v.max(0.0) } else { 0.0 };
    let mut out = Vec::new();
    if let Some(root) = m.root() {
        walk(root, DockRect { x: 0.0, y: 0.0, width: finite(size.width), height: finite(size.height) }, &mut out);
    }
    out
}

/// Zone a drop point over `panel_id` lands in. `x` and `y` are panel-local, so the hit test runs
/// against a rect at the origin. No size, or a panel not in the model, means `Center`, the one
/// zone that needs no geometry.
pub fn dock_zone_at<T>(
    m: &DockModel<T>,
    panel_id: &str,
    x: f32,
    y: f32,
    size: Option<DockSize>,
    edge: Option<f32>,
) -> DockZone {
    let Some(size) = size else { return DockZone::Center };
    let Some((_, rect)) = dock_panel_rects(m, size).into_iter().find(|(id, _)| id.as_ref() == panel_id) else {
        return DockZone::Center;
    };
    hit_test_dock_zone(DockRect { x: 0.0, y: 0.0, width: rect.width, height: rect.height }, x, y, edge)
}

/// Turns one drop into one model op, and is the whole drop policy:
///
/// - a panel drag is [`dock_panel`], merging into the target's tab stack on `Center` and
///   splitting the target on an edge;
/// - a tab dropped on `Center` is [`move_tab`] into the target's stack;
/// - a tab dropped on an edge is [`move_tab`] followed by [`undock_tab`], landing it in a new
///   panel on that side. A tab alone in its panel takes the [`dock_panel`] path instead: the
///   panel IS that tab, so moving it keeps its id and its already-docked no-op.
///
/// A drag that ends where it started returns the same tree.
pub fn apply_dock_drop<T: Clone>(
    m: &DockModel<T>,
    payload: &str,
    target_panel_id: &str,
    zone: DockZone,
) -> DockModel<T> {
    let Some(drag) = parse_dock_drag(payload) else { return m.clone() };
    if drag.kind == DockDragKind::Panel {
        return dock_panel(m, &drag.id, target_panel_id, zone);
    }

    let Some(found) = find_dock_tab(m, &drag.id) else { return m.clone() };
    if find_dock_panel(m, target_panel_id).is_none() {
        return m.clone();
    }
    let Some(source) = find_dock_panel(m, found.panel_id) else { return m.clone() };
    let source_id = found.panel_id.clone();

    let Some(edge) = zone.edge() else {
        return if source_id.as_ref() == target_panel_id {
            m.clone()
        } else {
            move_tab(m, &drag.id, target_panel_id, None)
        };
    };
    if source.tabs.len() == 1 {
        return dock_panel(m, &source_id, target_panel_id, zone);
    }
    if source_id.as_ref() == target_panel_id {
        return undock_tab(m, &drag.id, edge);
    }
    undock_tab(&move_tab(m, &drag.id, target_panel_id, None), &drag.id, edge)
}

pub fn serialize_dock<T: Clone>(m: &DockModel<T>) -> DockSnapshot<T> {
    serialize_panes(m)
}

fn normalize_panel<T>(panel: DockPanel<T>, is_data: &dyn Fn(&T) -> bool) -> Option<DockPanel<T>> {
    if panel.tabs.is_empty() {
        return None;
    }
    if panel.tabs.iter().any(|t| t.id.is_empty() || !is_data(&t.data)) {
        return None;
    }
    let active = if panel.tabs.iter().any(|t| t.id == panel.active_tab_id) {
        panel.active_tab_id.clone()
    } else {
        panel.tabs[0].id.clone()
    };
    Some(DockPanel { tabs: panel.tabs, active_tab_id: active })
}

/// Rebuilds a dock from persisted data, on `migrate_panes` terms: anything malformed rejects the
/// whole tree back to [`empty_dock`]. A panel with no tabs, a tab without an id, tab data
/// failing `is_data` and a tab id used twice all count as malformed; an `active_tab_id` naming a
/// tab that is gone is repaired to the panel's first tab instead.
pub fn deserialize_dock<T>(raw: DockSnapshot<T>, is_data: impl Fn(&T) -> bool) -> DockModel<T> {
    let mut seen: HashSet<SharedString> = HashSet::new();
    migrate_panes_with(raw, |panel| {
        let panel = normalize_panel(panel, &is_data)?;
        for tab in &panel.tabs {
            if !seen.insert(tab.id.clone()) {
                return None;
            }
        }
        Some(panel)
    })
}
