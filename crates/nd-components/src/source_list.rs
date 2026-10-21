//! `source_tree()`: the sidebar's hierarchical navigation list, data-driven. On macOS an
//! `NSOutlineView` in the source list style (system selection, group rows, disclosure, row
//! metrics); on GPUI backends the skin's navigation list.
//!
//! Nodes are flat: each names its parent by id, roots name none, and order is list order among
//! siblings. Selection and expansion are the app's: a click reports [`SourceSelected`], a
//! disclosure click reports [`SourceExpanded`] or [`SourceCollapsed`], and nothing changes on
//! screen that the app does not feed back, so an unrelated re-render never collapses a branch
//! the user opened. Rows may carry trailing actions (declared once with
//! [`SourceTree::actions`], referenced per node by id), shown on hover or always.
//!
//! `source_list()` is the same widget built from sections of nested items, for sidebars that
//! are a fixed set of places.
//!
//! With [`SourceTree::draggable_rows`] rows drag as a [`DragItem`] of that kind, within the list,
//! to another window, or onto any target taking the kind; with [`SourceTree::accepts_drops`] the
//! list takes drags, marks the slot between rows with a line and reports [`Dropped`] with that
//! [`DropSlot`]. Moving the row is the app's, as selection is.

use std::collections::HashMap;

use nd_core::semantic::Role;
use nd_ui::backend::Props;
use nd_ui::prelude::*;
use nd_ui::{DragEnded, DragItem, DragOver, DropSlot, Dropped, Key, NativeElement, StyleRefinement};

use crate::icon::Icon;

#[derive(Clone, Debug, PartialEq)]
pub struct SourceNode {
    pub id: SharedString,
    pub parent: Option<SharedString>,
    pub title: SharedString,
    /// A second, smaller line under the title.
    pub caption: Option<SharedString>,
    pub icon: Option<Icon>,
    pub badge: Option<SharedString>,
    /// A group row: a section header, never selectable, no disclosure.
    pub section: bool,
    /// Shows a disclosure before any child is listed (children load on expand).
    pub has_children: bool,
    pub expanded: bool,
    pub selectable: bool,
    /// Trailing actions, by id from the widget's action catalog.
    pub actions: Vec<SharedString>,
    /// The row drags when the list's rows do. Sections never drag.
    pub movable: bool,
}

/// A row with `id` showing `title`.
pub fn source_node(id: impl Into<SharedString>, title: impl Into<SharedString>) -> SourceNode {
    SourceNode {
        id: id.into(),
        parent: None,
        title: title.into(),
        caption: None,
        icon: None,
        badge: None,
        section: false,
        has_children: false,
        expanded: false,
        selectable: true,
        actions: Vec::new(),
        movable: true,
    }
}

/// A section header; rows name it as their parent. Expanded unless the app collapses it.
pub fn source_section(id: impl Into<SharedString>, title: impl Into<SharedString>) -> SourceNode {
    SourceNode { section: true, expanded: true, selectable: false, ..source_node(id, title) }
}

impl SourceNode {
    pub fn parent(mut self, parent: impl Into<SharedString>) -> Self {
        self.parent = Some(parent.into());
        self
    }

    pub fn caption(mut self, caption: impl Into<SharedString>) -> Self {
        self.caption = Some(caption.into());
        self
    }

    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn badge(mut self, badge: impl Into<SharedString>) -> Self {
        self.badge = Some(badge.into());
        self
    }

    pub fn has_children(mut self, has: bool) -> Self {
        self.has_children = has;
        self
    }

    pub fn expanded(mut self, expanded: bool) -> Self {
        self.expanded = expanded;
        self
    }

    pub fn selectable(mut self, selectable: bool) -> Self {
        self.selectable = selectable;
        self
    }

    pub fn actions(mut self, ids: impl IntoIterator<Item = impl Into<SharedString>>) -> Self {
        self.actions = ids.into_iter().map(Into::into).collect();
        self
    }

    pub fn movable(mut self, movable: bool) -> Self {
        self.movable = movable;
        self
    }
}

impl FluentBuilder for SourceNode {}

/// One trailing row affordance.
#[derive(Clone, Debug, PartialEq)]
pub struct SourceAction {
    pub id: SharedString,
    pub icon: Icon,
    /// Shown as text instead of the icon.
    pub label: Option<SharedString>,
    pub tooltip: Option<SharedString>,
    pub destructive: bool,
}

pub fn source_action(id: impl Into<SharedString>, icon: Icon) -> SourceAction {
    SourceAction { id: id.into(), icon, label: None, tooltip: None, destructive: false }
}

impl SourceAction {
    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn tooltip(mut self, tooltip: impl Into<SharedString>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }

    pub fn destructive(mut self) -> Self {
        self.destructive = true;
        self
    }

    /// What assistive technology reads for the action.
    pub fn accessible_label(&self) -> SharedString {
        self.label.clone().or_else(|| self.tooltip.clone()).unwrap_or_else(|| self.id.clone())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ActionVisibility {
    /// While the pointer is over the row.
    #[default]
    Hover,
    Always,
}

/// What the list shows with no rows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EmptyState {
    pub icon: Option<Icon>,
    pub title: SharedString,
    pub description: Option<SharedString>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SourceTreeProps {
    pub nodes: Vec<SourceNode>,
    pub actions: Vec<SourceAction>,
    pub selected: Option<SharedString>,
    pub action_visibility: ActionVisibility,
    pub empty: Option<EmptyState>,
    /// Rows drag as items of this kind.
    pub drag_kind: Option<SharedString>,
    /// Kinds the list takes; it then shows where a drop would land.
    pub accepts: Vec<SharedString>,
    pub style: StyleRefinement,
}

impl SourceTreeProps {
    /// Whether row `i` drags.
    pub fn drags(&self, i: usize) -> bool {
        self.drag_kind.is_some() && !self.nodes[i].section && self.nodes[i].movable
    }

    /// The item row `i` drags as.
    pub fn drag_item(&self, i: usize) -> Option<DragItem> {
        self.drag_kind.clone().filter(|_| self.drags(i)).map(|kind| DragItem::new(kind, self.nodes[i].id.clone()))
    }
}

/// The user selected the row with this id.
#[derive(Clone, Debug, PartialEq)]
pub struct SourceSelected {
    pub id: SharedString,
}

/// Double click or Return on a row.
#[derive(Clone, Debug, PartialEq)]
pub struct SourceActivated {
    pub id: SharedString,
}

/// The user opened a row's disclosure. The app sets the node's `expanded`.
#[derive(Clone, Debug, PartialEq)]
pub struct SourceExpanded {
    pub id: SharedString,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SourceCollapsed {
    pub id: SharedString,
}

/// A trailing action on row `id` was clicked.
#[derive(Clone, Debug, PartialEq)]
pub struct SourceActionClicked {
    pub id: SharedString,
    pub action: SharedString,
}

/// Widget kind of `source_tree` and `source_list`.
pub struct SourceTreeKind;

pub struct SourceTree(NativeElement<SourceTreeProps>);

fn describe(props: &Props) -> (Role, String) {
    let _ = props;
    (Role::Tree, String::new())
}

pub fn source_tree(id: impl Into<SharedString>) -> SourceTree {
    nd_ui::semantics::describe_widget::<SourceTreeKind>(describe);
    let props = SourceTreeProps {
        nodes: Vec::new(),
        actions: Vec::new(),
        selected: None,
        action_visibility: ActionVisibility::Hover,
        empty: None,
        drag_kind: None,
        accepts: Vec::new(),
        style: StyleRefinement::default(),
    };
    SourceTree(NativeElement::new::<SourceTreeKind>(Key::Id(id.into()), props).size_full())
}

impl SourceTree {
    pub fn nodes(mut self, nodes: impl IntoIterator<Item = SourceNode>) -> Self {
        self.0.props_mut().nodes = nodes.into_iter().collect();
        self
    }

    /// The action catalog rows reference by id.
    pub fn actions(mut self, actions: impl IntoIterator<Item = SourceAction>) -> Self {
        self.0.props_mut().actions = actions.into_iter().collect();
        self
    }

    pub fn selected(mut self, id: impl Into<SharedString>) -> Self {
        self.0.props_mut().selected = Some(id.into());
        self
    }

    pub fn selected_opt(mut self, id: Option<SharedString>) -> Self {
        self.0.props_mut().selected = id;
        self
    }

    pub fn action_visibility(mut self, visibility: ActionVisibility) -> Self {
        self.0.props_mut().action_visibility = visibility;
        self
    }

    pub fn empty_state(
        mut self,
        icon: Option<Icon>,
        title: impl Into<SharedString>,
        description: Option<SharedString>,
    ) -> Self {
        self.0.props_mut().empty = Some(EmptyState { icon, title: title.into(), description });
        self
    }

    /// A titled group of rows; nested items are expanded.
    pub fn section(mut self, title: impl Into<SharedString>, items: impl IntoIterator<Item = SourceItem>) -> Self {
        let title = title.into();
        let props = self.0.props_mut();
        let id: SharedString = format!("\u{1}section:{}", props.nodes.iter().filter(|n| n.section).count()).into();
        props.nodes.push(source_section(id.clone(), title));
        for item in items {
            item.flatten(Some(&id), &mut props.nodes);
        }
        self
    }

    /// Rows with no group row above them; nested items are expanded.
    pub fn items(mut self, items: impl IntoIterator<Item = SourceItem>) -> Self {
        let props = self.0.props_mut();
        for item in items {
            item.flatten(None, &mut props.nodes);
        }
        self
    }

    pub fn on_select(self, f: impl Fn(&SourceSelected, &mut Window, &mut App) + 'static) -> Self {
        SourceTree(self.0.on(f))
    }

    pub fn on_activate(self, f: impl Fn(&SourceActivated, &mut Window, &mut App) + 'static) -> Self {
        SourceTree(self.0.on(f))
    }

    pub fn on_expand(self, f: impl Fn(&SourceExpanded, &mut Window, &mut App) + 'static) -> Self {
        SourceTree(self.0.on(f))
    }

    pub fn on_collapse(self, f: impl Fn(&SourceCollapsed, &mut Window, &mut App) + 'static) -> Self {
        SourceTree(self.0.on(f))
    }

    pub fn on_action(self, f: impl Fn(&SourceActionClicked, &mut Window, &mut App) + 'static) -> Self {
        SourceTree(self.0.on(f))
    }

    /// Rows drag as [`DragItem`]s of `kind` with the row's id, except sections and rows that
    /// are not [`movable`](SourceNode::movable).
    pub fn draggable_rows(mut self, kind: impl Into<SharedString>) -> Self {
        self.0.props_mut().drag_kind = Some(kind.into());
        self
    }

    /// The list takes drags of these kinds (its own rows' kind, to reorder them).
    pub fn accepts_drops(mut self, kinds: impl IntoIterator<Item = impl Into<SharedString>>) -> Self {
        let kinds: Vec<SharedString> = kinds.into_iter().map(Into::into).collect();
        self.0.props_mut().accepts = kinds.clone();
        SourceTree(self.0.accepts_drops(kinds))
    }

    pub fn on_drag_over(self, f: impl Fn(&DragOver, &mut Window, &mut App) + 'static) -> Self {
        SourceTree(self.0.on(f))
    }

    /// A drag was dropped on the list; its `slot` says where among the rows.
    pub fn on_drop(self, f: impl Fn(&Dropped, &mut Window, &mut App) + 'static) -> Self {
        SourceTree(self.0.on(f))
    }

    /// A row's drag is over: dropped, cancelled, or released outside the app's windows.
    pub fn on_drag_end(self, f: impl Fn(&DragEnded, &mut Window, &mut App) + 'static) -> Self {
        SourceTree(self.0.on(f))
    }
}

impl Styled for SourceTree {
    fn style(&mut self) -> &mut StyleRefinement {
        self.0.style()
    }
}

impl FluentBuilder for SourceTree {}

impl IntoElement for SourceTree {
    fn into_any_element(mut self) -> AnyElement {
        let style = self.0.style().clone();
        self.0.props_mut().style = style;
        self.0.into_any_element()
    }
}

/// A nested row for `source_list()`.
#[derive(Clone, Debug, PartialEq)]
pub struct SourceItem {
    pub id: SharedString,
    pub title: SharedString,
    pub icon: Option<Icon>,
    pub badge: Option<SharedString>,
    pub children: Vec<SourceItem>,
}

/// One row: an id the app selects by, a title, and optionally an icon, a badge and children.
pub fn source_item(id: impl Into<SharedString>, title: impl Into<SharedString>) -> SourceItem {
    SourceItem { id: id.into(), title: title.into(), icon: None, badge: None, children: Vec::new() }
}

impl SourceItem {
    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn badge(mut self, badge: impl Into<SharedString>) -> Self {
        self.badge = Some(badge.into());
        self
    }

    pub fn children(mut self, children: impl IntoIterator<Item = SourceItem>) -> Self {
        self.children = children.into_iter().collect();
        self
    }

    fn flatten(self, parent: Option<&SharedString>, out: &mut Vec<SourceNode>) {
        let mut node = source_node(self.id.clone(), self.title).expanded(!self.children.is_empty());
        node.parent = parent.cloned();
        node.icon = self.icon;
        node.badge = self.badge;
        out.push(node);
        for child in self.children {
            child.flatten(Some(&self.id), out);
        }
    }
}

/// A sidebar of fixed places in titled sections: [`SourceTree::section`] and
/// [`SourceTree::items`] fill it.
pub fn source_list(id: impl Into<SharedString>) -> SourceTree {
    source_tree(id)
}

/// The flat node list as a tree: children per parent in list order. A node whose parent is not
/// listed is a root, as is one that would close a cycle.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SourceIndex {
    /// Node indices of the roots.
    pub roots: Vec<usize>,
    /// Node indices of each node's children, by node index.
    pub children: Vec<Vec<usize>>,
    pub by_id: HashMap<SharedString, usize>,
}

impl SourceIndex {
    pub fn new(nodes: &[SourceNode]) -> Self {
        let mut by_id = HashMap::with_capacity(nodes.len());
        for (i, n) in nodes.iter().enumerate() {
            by_id.entry(n.id.clone()).or_insert(i);
        }
        let mut children = vec![Vec::new(); nodes.len()];
        let mut roots = Vec::new();
        // Parent links attached so far: a link that would close a cycle is never attached.
        let mut attached: Vec<Option<usize>> = vec![None; nodes.len()];
        for (i, n) in nodes.iter().enumerate() {
            if by_id.get(&n.id) != Some(&i) {
                continue; // A duplicate id: the first one wins.
            }
            let parent = n.parent.as_ref().and_then(|p| by_id.get(p)).copied();
            match parent {
                Some(p) if !Self::reaches(&attached, p, i) => {
                    attached[i] = Some(p);
                    children[p].push(i);
                }
                _ => roots.push(i),
            }
        }
        Self { roots, children, by_id }
    }

    /// Whether walking attached parent links up from `from` reaches `to`.
    fn reaches(attached: &[Option<usize>], from: usize, to: usize) -> bool {
        let mut at = Some(from);
        let mut steps = 0;
        while let Some(i) = at {
            if i == to {
                return true;
            }
            steps += 1;
            if steps > attached.len() {
                return false;
            }
            at = attached[i];
        }
        false
    }

    pub fn expandable(&self, nodes: &[SourceNode], i: usize) -> bool {
        !nodes[i].section && (nodes[i].has_children || !self.children[i].is_empty())
    }

    /// The rows on screen, in order, with their depth: children of collapsed rows are hidden.
    /// Section rows sit at depth 0 and their rows at depth 0 too, like Finder's sidebar.
    pub fn visible(&self, nodes: &[SourceNode]) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        for &r in &self.roots {
            self.walk(nodes, r, 0, &mut out);
        }
        out
    }

    /// Where a drop lands when the pointer is over visible row `row` (an index into
    /// [`Self::visible`]), in its upper half or not; `None` past the last row. A section header
    /// takes it as its first row, an open row with children in its lower half as its first child,
    /// anything else before or after the row among its siblings. The index counts the children
    /// as they are now, the dragged row included.
    pub fn drop_slot(
        &self,
        nodes: &[SourceNode],
        visible: &[(usize, usize)],
        row: Option<usize>,
        upper: bool,
    ) -> DropSlot {
        let parent_of = |i: usize| nodes[i].parent.clone().filter(|p| self.by_id.contains_key(p));
        let siblings = |parent: &Option<SharedString>| match parent {
            Some(p) => self.children[self.by_id[p]].clone(),
            None => self.roots.clone(),
        };
        let Some(&(i, _)) = row.and_then(|r| visible.get(r)) else {
            // Past the last row: after the last row of the last section, or the last root.
            let Some(&(last, _)) = visible.last() else { return DropSlot { parent: None, index: 0 } };
            let parent = if nodes[last].section { Some(nodes[last].id.clone()) } else { parent_of(last) };
            let index = siblings(&parent).len();
            return DropSlot { parent, index };
        };
        let opens = nodes[i].expanded && !self.children[i].is_empty();
        if nodes[i].section || (!upper && opens) {
            return DropSlot { parent: Some(nodes[i].id.clone()), index: 0 };
        }
        let parent = parent_of(i);
        let at = siblings(&parent).iter().position(|&s| s == i).unwrap_or(0);
        DropSlot { parent, index: if upper { at } else { at + 1 } }
    }

    fn walk(&self, nodes: &[SourceNode], i: usize, depth: usize, out: &mut Vec<(usize, usize)>) {
        out.push((i, depth));
        if nodes[i].expanded {
            let child_depth = if nodes[i].section { depth } else { depth + 1 };
            for &c in &self.children[i] {
                self.walk(nodes, c, child_depth, out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(nodes: &[SourceNode], rows: &[(usize, usize)]) -> Vec<(String, usize)> {
        rows.iter().map(|(i, d)| (nodes[*i].id.to_string(), *d)).collect()
    }

    #[test]
    fn flat_nodes_form_a_tree_in_list_order() {
        let nodes = vec![
            source_section("lib", "Library"),
            source_node("inbox", "Inbox").parent("lib"),
            source_node("work", "Work").parent("lib").expanded(true),
            source_node("q3", "Q3").parent("work"),
            source_node("old", "Old").parent("work"),
            source_node("loose", "Loose"),
        ];
        let index = SourceIndex::new(&nodes);
        assert_eq!(index.roots, [0, 5]);
        assert_eq!(
            ids(&nodes, &index.visible(&nodes)),
            [("lib", 0), ("inbox", 0), ("work", 0), ("q3", 1), ("old", 1), ("loose", 0)]
                .map(|(i, d)| (i.to_owned(), d))
        );
        assert!(index.expandable(&nodes, 2));
        assert!(!index.expandable(&nodes, 0), "sections have no disclosure");
    }

    #[test]
    fn collapsed_rows_hide_their_children() {
        let nodes = vec![source_node("a", "A"), source_node("b", "B").parent("a")];
        let index = SourceIndex::new(&nodes);
        assert_eq!(index.visible(&nodes).len(), 1);
        assert!(index.expandable(&nodes, 0));
    }

    #[test]
    fn unknown_parents_cycles_and_duplicates_degrade_to_roots() {
        let nodes = vec![
            source_node("a", "A").parent("b"),
            source_node("b", "B").parent("a"),
            source_node("c", "C").parent("missing"),
            source_node("c", "C again"),
        ];
        let index = SourceIndex::new(&nodes);
        assert_eq!(index.roots, [1, 2], "the cycle is cut at the node closing it");
        assert_eq!(index.children[1], [0]);
    }

    #[test]
    fn drop_slots_fall_between_siblings_and_into_sections() {
        let nodes = vec![
            source_section("pinned", "Pinned"),
            source_node("p1", "P1").parent("pinned"),
            source_section("tabs", "Tabs"),
            source_node("t1", "T1").parent("tabs"),
            source_node("t2", "T2").parent("tabs").expanded(true),
            source_node("t2a", "T2a").parent("t2"),
        ];
        let index = SourceIndex::new(&nodes);
        let rows = index.visible(&nodes);
        let slot = |row, upper| index.drop_slot(&nodes, &rows, row, upper);
        let at = |p: &str, i| DropSlot { parent: Some(p.into()), index: i };
        assert_eq!(slot(Some(0), true), at("pinned", 0), "a section header takes it first");
        assert_eq!(slot(Some(1), false), at("pinned", 1));
        assert_eq!(slot(Some(3), true), at("tabs", 0));
        assert_eq!(slot(Some(3), false), at("tabs", 1));
        assert_eq!(slot(Some(4), false), at("t2", 0), "an open row's lower half is its first child");
        assert_eq!(slot(Some(5), false), at("t2", 1));
        assert_eq!(slot(None, false), at("t2", 1), "past the end is after the last row");
        let flat = vec![source_node("a", "A"), source_node("b", "B")];
        let index = SourceIndex::new(&flat);
        let rows = index.visible(&flat);
        assert_eq!(index.drop_slot(&flat, &rows, Some(1), true), DropSlot { parent: None, index: 1 });
        assert_eq!(index.drop_slot(&flat, &rows, None, false), DropSlot { parent: None, index: 2 });
    }

    #[test]
    fn lazy_children_still_show_a_disclosure() {
        let nodes = vec![source_node("remote", "Remote").has_children(true)];
        assert!(SourceIndex::new(&nodes).expandable(&nodes, 0));
    }

    #[test]
    fn sections_of_nested_items_flatten_expanded() {
        let tree = source_list("s")
            .section("Mail", [source_item("inbox", "Inbox").children([source_item("vip", "VIP")])])
            .items([source_item("trash", "Trash")]);
        let nodes = &tree.0.props().nodes;
        let index = SourceIndex::new(nodes);
        assert_eq!(index.visible(nodes).len(), 4);
        assert!(nodes[0].section && !nodes[0].selectable);
        assert_eq!(nodes[2].parent.as_deref(), Some("inbox"));
    }
}
