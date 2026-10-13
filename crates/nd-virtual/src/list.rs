//! The app-facing API: [`ListState`] holds an app's keys over a [`Core`], and `list()`,
//! `table()`, `grid()` and `tree()` turn it into one native container node whose children are
//! the rows the model wants rendered.
//!
//! Each rendered row is its own view ([`RowView`]), created when the row enters the render range
//! and dropped when it leaves. A view is a boundary in `nd-ui`: the owner re-rendering does not
//! re-render it, so a row renders again only when its version changes or it is invalidated
//! (requirement g). Rows render outside the owner's render and outside any native callback, and
//! presenters answer native data sources from the rows already rendered (3.13).

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::hash::Hash;
use std::rc::{Rc, Weak};

use nd_ui::prelude::*;
use nd_ui::{ContainerElement, EntityId, Key, Scope, StyleRefinement};

use crate::core::{Align, Alignment, Core, Follow, GridSpec, RowId};

/// What a list can be keyed by.
pub trait RowKey: Clone + Eq + Hash + fmt::Debug + 'static {}
impl<T: Clone + Eq + Hash + fmt::Debug + 'static> RowKey for T {}

/// The model, shared between an app's [`ListState`] and the backend's presenter. Compared by
/// identity in props.
#[derive(Clone)]
pub struct ListHandle(pub Rc<RefCell<Core>>);

impl PartialEq for ListHandle {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl fmt::Debug for ListHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ListHandle({:p})", Rc::as_ptr(&self.0))
    }
}

/// Which native container presents the rows.
#[derive(Clone, Debug, PartialEq)]
pub enum ListKind {
    /// A single column: `NSTableView` on macOS.
    List,
    /// Columns with a header: `NSTableView` on macOS.
    Table(Vec<Column>),
    /// Fixed-size cells in lanes: `NSCollectionView` on macOS.
    Grid,
    /// Rows with depth and disclosure: `NSOutlineView` on macOS.
    Tree,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SortDirection {
    #[default]
    Ascending,
    Descending,
}

/// A table column. Sorting stays the app's: a header click reports [`SortChanged`] and the app
/// re-sorts its keys.
#[derive(Clone, Debug, PartialEq)]
pub struct Column {
    pub id: SharedString,
    pub title: SharedString,
    pub width: f32,
    pub min_width: f32,
    pub resizable: bool,
    pub sortable: bool,
    pub sort: Option<SortDirection>,
}

pub fn column(id: impl Into<SharedString>, title: impl Into<SharedString>) -> Column {
    Column {
        id: id.into(),
        title: title.into(),
        width: 120.,
        min_width: 40.,
        resizable: true,
        sortable: false,
        sort: None,
    }
}

impl Column {
    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    pub fn min_width(mut self, width: f32) -> Self {
        self.min_width = width;
        self
    }

    pub fn fixed(mut self) -> Self {
        self.resizable = false;
        self
    }

    pub fn sortable(mut self) -> Self {
        self.sortable = true;
        self
    }

    pub fn sorted(mut self, direction: SortDirection) -> Self {
        self.sortable = true;
        self.sort = Some(direction);
        self
    }
}

/// How many rows the user can select.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Selection {
    #[default]
    None,
    Single,
    Multiple,
}

impl Selection {
    pub fn single() -> Self {
        Selection::Single
    }

    pub fn multiple() -> Self {
        Selection::Multiple
    }
}

/// Props of the list's container node.
#[derive(Clone, Debug, PartialEq)]
pub struct ListProps {
    pub handle: ListHandle,
    pub kind: ListKind,
    /// The model's generation when the owner last rendered: presenters re-read rows when it moves.
    pub generation: u64,
    pub selection: Selection,
    pub selected: Vec<RowId>,
    pub style: StyleRefinement,
}

/// Widget kind of the list container.
pub struct ListWidget;

/// Hung on each rendered row's root by [`RowView`], so presenters know which row a child is.
#[derive(Clone, Debug, PartialEq)]
pub struct RowTag {
    pub row: RowId,
    /// Tree rows: nesting depth, whether the row has children, whether they show.
    pub depth: u16,
    pub expandable: bool,
    pub expanded: bool,
}

/// Hung on each cell of a table row, with the column's index.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CellTag(pub usize);

// Events presenters emit on the list node.

/// The viewport moved or resized: the rows the owner should render may have changed.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ViewportChanged {
    pub at_start: bool,
    pub at_end: bool,
}

/// The user changed the selection.
#[derive(Clone, Debug, PartialEq)]
pub struct RowsSelected(pub Vec<RowId>);

/// The user activated a row (double click, Return).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RowActivated(pub RowId);

/// The user expanded or collapsed a tree row.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RowDisclosed {
    pub row: RowId,
    pub expanded: bool,
}

/// The user resized a column (by index).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColumnResized {
    pub column: usize,
    pub width: f32,
}

/// The user clicked a sortable column's header.
#[derive(Clone, Debug, PartialEq)]
pub struct SortChanged {
    pub column: SharedString,
    pub direction: SortDirection,
}

// Events the app handles.

/// The user selected `key` (the last row clicked); `keys` is the whole selection.
#[derive(Clone, Debug, PartialEq)]
pub struct SelectEvent<K> {
    pub key: K,
    pub keys: Vec<K>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ActivateEvent<K> {
    pub key: K,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExpandEvent<K> {
    pub key: K,
    pub expanded: bool,
}

/// One node of a tree: its key, its parent (`None` for a root) and a version.
#[derive(Clone, Debug, PartialEq)]
pub struct TreeNode<K> {
    pub key: K,
    pub parent: Option<K>,
    pub version: u64,
}

type Handler<E> = Rc<dyn Fn(&E, &mut Window, &mut App)>;
type RenderFn<K> = Rc<dyn Fn(K, &mut Window, &mut App) -> AnyElement>;
type CellFn<K> = Rc<dyn Fn(K, usize, &mut Window, &mut App) -> AnyElement>;

#[derive(Clone)]
enum Renderer<K> {
    Row(RenderFn<K>),
    Cells(CellFn<K>, Vec<f32>),
}

#[derive(Clone, Copy, Default)]
struct Meta {
    depth: u16,
    expandable: bool,
}

struct Inner<K: RowKey> {
    core: Rc<RefCell<Core>>,
    owner: EntityId,
    keys: Vec<K>,
    ids: HashMap<K, RowId>,
    by_id: HashMap<RowId, K>,
    versions: HashMap<RowId, u64>,
    meta: HashMap<RowId, Meta>,
    next_id: u64,
    /// The rendered rows: the model's render range, in order.
    window: Vec<RowId>,
    views: HashMap<RowId, Entity<RowView<K>>>,
    renderer: Option<Renderer<K>>,
    selected: Vec<K>,
    /// Tree: every node, and the keys whose children show.
    tree: Option<Vec<TreeNode<K>>>,
    expanded: HashSet<K>,
    near: (bool, bool),
}

/// An app's list: its keys, scroll position, selection and rendered rows. Cheap to clone; every
/// clone is the same list.
pub struct ListState<K: RowKey> {
    inner: Rc<RefCell<Inner<K>>>,
}

impl<K: RowKey> Clone for ListState<K> {
    fn clone(&self) -> Self {
        Self { inner: self.inner.clone() }
    }
}

impl<K: RowKey> ListState<K> {
    /// A list owned by the view `cx` belongs to: it re-renders that view when the rows to show
    /// change.
    pub fn new<T: 'static>(cx: &mut Context<T>) -> Self {
        let owner = cx.entity_id();
        Self {
            inner: Rc::new(RefCell::new(Inner {
                core: Rc::new(RefCell::new(Core::new())),
                owner,
                keys: Vec::new(),
                ids: HashMap::new(),
                by_id: HashMap::new(),
                versions: HashMap::new(),
                meta: HashMap::new(),
                next_id: 1,
                window: Vec::new(),
                views: HashMap::new(),
                renderer: None,
                selected: Vec::new(),
                tree: None,
                expanded: HashSet::new(),
                near: (false, false),
            })),
        }
    }

    pub fn follow(self, follow: Follow) -> Self {
        self.core().borrow_mut().set_follow(follow);
        self
    }

    pub fn alignment(self, alignment: Alignment) -> Self {
        self.core().borrow_mut().set_alignment(alignment);
        if alignment == Alignment::Bottom {
            let follow = self.core().borrow().follow();
            self.core().borrow_mut().set_follow(follow);
        }
        self
    }

    /// Height assumed for rows not measured yet.
    pub fn estimated_row_height(self, height: Pixels) -> Self {
        self.core().borrow_mut().set_estimate(f64::from(height.as_f32()));
        self
    }

    /// Rows rendered beyond each edge of the viewport, in viewports (1 by default).
    pub fn overscan(self, viewports: f64) -> Self {
        self.core().borrow_mut().set_overscan(viewports);
        self
    }

    /// The model; presenters and tests read it.
    pub fn handle(&self) -> ListHandle {
        ListHandle(self.core())
    }

    fn core(&self) -> Rc<RefCell<Core>> {
        self.inner.borrow().core.clone()
    }

    pub fn len(&self) -> usize {
        self.inner.borrow().keys.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.borrow().keys.is_empty()
    }

    /// The keys in order (a tree's visible rows).
    pub fn items(&self) -> Vec<K> {
        self.inner.borrow().keys.clone()
    }

    pub fn key_of(&self, row: RowId) -> Option<K> {
        self.inner.borrow().by_id.get(&row).cloned()
    }

    pub fn row_of(&self, key: &K) -> Option<RowId> {
        self.inner.borrow().ids.get(key).copied()
    }

    /// Replaces the rows by key. Rows that stay keep their height and the scroll position stays
    /// on its row; there is no reset. Without versions every rendered row renders again; use
    /// [`Self::set_items_versioned`] so only changed rows do.
    pub fn set_items(&mut self, keys: impl IntoIterator<Item = K>, cx: &mut App) {
        self.apply(keys.into_iter().map(|k| (k, None, Meta::default())).collect(), cx);
    }

    /// As [`Self::set_items`], with a version per key: a rendered row renders again only when
    /// its version changed.
    pub fn set_items_versioned(&mut self, rows: impl IntoIterator<Item = (K, u64)>, cx: &mut App) {
        self.apply(rows.into_iter().map(|(k, v)| (k, Some(v), Meta::default())).collect(), cx);
    }

    /// A tree: nodes in display order among siblings, parents before children. Only rows under
    /// expanded parents are listed; expansion is kept here by key.
    pub fn set_tree(&mut self, nodes: impl IntoIterator<Item = TreeNode<K>>, cx: &mut App) {
        self.inner.borrow_mut().tree = Some(nodes.into_iter().collect());
        self.flatten_tree(cx);
    }

    pub fn is_expanded(&self, key: &K) -> bool {
        self.inner.borrow().expanded.contains(key)
    }

    pub fn set_expanded(&mut self, key: K, expanded: bool, cx: &mut App) {
        let changed = {
            let mut inner = self.inner.borrow_mut();
            if expanded { inner.expanded.insert(key) } else { inner.expanded.remove(&key) }
        };
        if changed {
            self.flatten_tree(cx);
            cx.notify(self.inner.borrow().owner);
        }
    }

    fn flatten_tree(&mut self, cx: &mut App) {
        let rows = {
            let inner = self.inner.borrow();
            let Some(nodes) = inner.tree.as_ref() else { return };
            let mut children: HashMap<Option<&K>, Vec<&TreeNode<K>>> = HashMap::new();
            for n in nodes {
                children.entry(n.parent.as_ref()).or_default().push(n);
            }
            let mut rows = Vec::new();
            let mut stack: Vec<(&TreeNode<K>, u16)> =
                children.get(&None).map(|r| r.iter().rev().map(|n| (*n, 0)).collect()).unwrap_or_default();
            while let Some((node, depth)) = stack.pop() {
                let kids = children.get(&Some(&node.key));
                let expandable = kids.is_some_and(|k| !k.is_empty());
                rows.push((node.key.clone(), Some(node.version), Meta { depth, expandable }));
                if expandable && inner.expanded.contains(&node.key) {
                    stack.extend(kids.into_iter().flatten().rev().map(|n| (*n, depth + 1)));
                }
            }
            rows
        };
        self.apply(rows, cx);
    }

    fn apply(&mut self, rows: Vec<(K, Option<u64>, Meta)>, cx: &mut App) {
        let mut stale = Vec::new();
        {
            let mut guard = self.inner.borrow_mut();
            let inner = &mut *guard;
            let mut ids = Vec::with_capacity(rows.len());
            let mut next_ids = HashMap::with_capacity(rows.len());
            let mut keys = Vec::with_capacity(rows.len());
            for (key, version, meta) in rows {
                let id = match inner.ids.get(&key) {
                    Some(id) => *id,
                    None => {
                        let id = RowId(inner.next_id);
                        inner.next_id += 1;
                        id
                    }
                };
                let changed = match version {
                    Some(v) => inner.versions.insert(id, v).is_some_and(|old| old != v),
                    None => true,
                };
                let meta_changed = inner
                    .meta
                    .insert(id, meta)
                    .is_some_and(|old| old.depth != meta.depth || old.expandable != meta.expandable);
                if (changed || meta_changed) && inner.views.contains_key(&id) {
                    stale.push(id);
                }
                ids.push(id);
                next_ids.insert(key.clone(), id);
                keys.push(key);
            }
            inner.by_id = next_ids.iter().map(|(k, id)| (*id, k.clone())).collect();
            inner.versions.retain(|id, _| inner.by_id.contains_key(id));
            inner.meta.retain(|id, _| inner.by_id.contains_key(id));
            inner.ids = next_ids;
            inner.keys = keys;
            let ids_now = &inner.ids;
            inner.selected.retain(|k| ids_now.contains_key(k));
            inner.core.borrow_mut().set_rows(ids);
            let mut core = inner.core.borrow_mut();
            for id in &stale {
                core.invalidate(*id);
            }
        }
        for id in stale {
            let view = self.inner.borrow().views.get(&id).map(|v| v.entity_id());
            if let Some(view) = view {
                cx.notify(view);
            }
        }
        self.sync(cx);
        cx.notify(self.inner.borrow().owner);
    }

    /// Renders `key`'s row again and remeasures it, as after its content changed.
    pub fn invalidate(&mut self, key: &K, cx: &mut App) {
        let inner = self.inner.borrow();
        let Some(&id) = inner.ids.get(key) else { return };
        inner.core.borrow_mut().invalidate(id);
        if let Some(view) = inner.views.get(&id) {
            let view = view.entity_id();
            drop(inner);
            cx.notify(view);
        }
    }

    /// Renders every rendered row again, as after a setting their output depends on changed.
    pub fn invalidate_all(&mut self, cx: &mut App) {
        let views: Vec<EntityId> = {
            let inner = self.inner.borrow();
            let mut core = inner.core.borrow_mut();
            for id in inner.views.keys() {
                core.invalidate(*id);
            }
            inner.views.values().map(|v| v.entity_id()).collect()
        };
        for view in views {
            cx.notify(view);
        }
    }

    /// Scrolls `key` into place; `offset` moves the result (negative leaves room above it).
    /// Animated on the frame clock when the distance is short.
    pub fn scroll_to(&mut self, key: K, align: Align, offset: Pixels, cx: &mut App) {
        let Some(id) = self.row_of(&key) else { return };
        self.core().borrow_mut().scroll_to(id, align, f64::from(offset.as_f32()), true);
        self.sync(cx);
        cx.notify(self.inner.borrow().owner);
    }

    /// As [`Self::scroll_to`], without the animation.
    pub fn jump_to(&mut self, key: K, align: Align, offset: Pixels, cx: &mut App) {
        let Some(id) = self.row_of(&key) else { return };
        self.core().borrow_mut().scroll_to(id, align, f64::from(offset.as_f32()), false);
        self.sync(cx);
        cx.notify(self.inner.borrow().owner);
    }

    pub fn scroll_to_top(&mut self, cx: &mut App) {
        self.core().borrow_mut().scroll_to_start();
        self.sync(cx);
        cx.notify(self.inner.borrow().owner);
    }

    /// Jumps to the end; with `Follow::Tail` the list stays there as rows arrive.
    pub fn scroll_to_end(&mut self, cx: &mut App) {
        self.core().borrow_mut().scroll_to_end();
        self.sync(cx);
        cx.notify(self.inner.borrow().owner);
    }

    /// A user scroll by `dy` points, as a wheel would; tests and automation use it.
    pub fn scroll_by(&mut self, dy: f64, cx: &mut App) {
        self.core().borrow_mut().scroll_by(dy);
        self.sync(cx);
        cx.notify(self.inner.borrow().owner);
    }

    pub fn is_following_tail(&self) -> bool {
        self.core().borrow().following()
    }

    pub fn select(&mut self, key: K, cx: &mut App) {
        self.set_selected(vec![key], cx);
    }

    pub fn set_selected(&mut self, keys: Vec<K>, cx: &mut App) {
        let mut inner = self.inner.borrow_mut();
        let keys: Vec<K> = keys.into_iter().filter(|k| inner.ids.contains_key(k)).collect();
        if inner.selected != keys {
            inner.selected = keys;
            let owner = inner.owner;
            drop(inner);
            cx.notify(owner);
        }
    }

    pub fn selected(&self) -> Vec<K> {
        self.inner.borrow().selected.clone()
    }

    /// The rows rendered now, in order.
    pub fn rendered(&self) -> Vec<K> {
        let inner = self.inner.borrow();
        inner.window.iter().filter_map(|id| inner.by_id.get(id).cloned()).collect()
    }

    /// The row's view while it is rendered: its render count and node are the row's.
    pub fn row_view(&self, key: &K) -> Option<EntityId> {
        let inner = self.inner.borrow();
        inner.views.get(inner.ids.get(key)?).map(|v| v.entity_id())
    }

    /// Brings the rendered rows in line with the model's render range: creates views for rows
    /// that entered it, drops those that left, and re-renders the owner when the set changed.
    pub fn sync(&self, cx: &mut App) -> bool {
        let wanted: Vec<RowId> = {
            let inner = self.inner.borrow();
            let core = inner.core.borrow();
            core.render_range().filter_map(|i| core.row(i)).collect()
        };
        let mut inner = self.inner.borrow_mut();
        if inner.window == wanted {
            return false;
        }
        let keep: HashSet<RowId> = wanted.iter().copied().collect();
        inner.views.retain(|id, _| keep.contains(id));
        let weak = Rc::downgrade(&self.inner);
        let missing: Vec<(RowId, K)> = wanted
            .iter()
            .filter(|id| !inner.views.contains_key(id))
            .filter_map(|id| inner.by_id.get(id).map(|k| (*id, k.clone())))
            .collect();
        inner.window = wanted;
        let owner = inner.owner;
        drop(inner);
        for (id, key) in missing {
            let list = weak.clone();
            let view = cx.new(|_| RowView { key, row: id, list });
            self.inner.borrow_mut().views.insert(id, view);
        }
        cx.notify(owner);
        true
    }

    /// The viewport moved: renders the rows it now needs and reports edges crossed.
    fn on_viewport(&self, event: &ViewportChanged, cx: &mut App) -> (bool, bool) {
        self.sync(cx);
        let mut inner = self.inner.borrow_mut();
        let (was_start, was_end) = inner.near;
        inner.near = (event.at_start, event.at_end);
        (event.at_start && !was_start, event.at_end && !was_end)
    }

    fn props(&self, kind: ListKind, selection: Selection, style: StyleRefinement) -> ListProps {
        let inner = self.inner.borrow();
        let generation = inner.core.borrow().generation();
        let selected = inner.selected.iter().filter_map(|k| inner.ids.get(k).copied()).collect();
        ListProps { handle: ListHandle(inner.core.clone()), kind, generation, selection, selected, style }
    }

    fn children(&self) -> Vec<AnyElement> {
        let inner = self.inner.borrow();
        inner.window.iter().filter_map(|id| inner.views.get(id)).map(|v| v.clone().into_any_element()).collect()
    }
}

/// One rendered row: calls the list's row closure for its key and tags the output with the row's
/// id. Re-renders only when notified, never because its owner did.
pub struct RowView<K: RowKey> {
    key: K,
    row: RowId,
    list: Weak<RefCell<Inner<K>>>,
}

impl<K: RowKey> Render for RowView<K> {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(list) = self.list.upgrade() else { return div().into_any_element() };
        let (renderer, meta, expanded) = {
            let inner = list.borrow();
            let meta = inner.meta.get(&self.row).copied().unwrap_or_default();
            (inner.renderer.clone(), meta, inner.expanded.contains(&self.key))
        };
        let tag = RowTag { row: self.row, depth: meta.depth, expandable: meta.expandable, expanded };
        let row = div().scope(Scope::new(tag)).w_full().flex().flex_col();
        let app: &mut App = cx;
        match renderer {
            Some(Renderer::Row(render)) => row.child(render(self.key.clone(), window, app)),
            Some(Renderer::Cells(render, widths)) => {
                let cells = widths.iter().enumerate().map(|(i, w)| {
                    div()
                        .id(format!("cell-{i}"))
                        .scope(Scope::new(CellTag(i)))
                        .w(px(*w))
                        .flex_shrink_0()
                        .overflow_hidden()
                        .child(render(self.key.clone(), i, window, app))
                        .into_any_element()
                });
                row.flex_row().children(cells.collect::<Vec<_>>())
            }
            None => row,
        }
        .into_any_element()
    }
}

/// A virtualized container of any [`ListKind`] under construction.
pub struct List<K: RowKey> {
    id: SharedString,
    state: ListState<K>,
    kind: ListKind,
    renderer: Renderer<K>,
    style: StyleRefinement,
    selection: Selection,
    empty: Option<AnyElement>,
    on_select: Option<Handler<SelectEvent<K>>>,
    on_activate: Option<Handler<ActivateEvent<K>>>,
    on_expand: Option<Handler<ExpandEvent<K>>>,
    on_near_start: Option<Handler<()>>,
    on_near_end: Option<Handler<()>>,
    on_column_resize: Option<Handler<ColumnResized>>,
    on_sort: Option<Handler<SortChanged>>,
}

fn build<K: RowKey>(id: SharedString, state: &ListState<K>, kind: ListKind, renderer: Renderer<K>) -> List<K> {
    List {
        id,
        state: state.clone(),
        kind,
        renderer,
        style: StyleRefinement::default(),
        selection: Selection::None,
        empty: None,
        on_select: None,
        on_activate: None,
        on_expand: None,
        on_near_start: None,
        on_near_end: None,
        on_column_resize: None,
        on_sort: None,
    }
}

fn row_fn<K: RowKey, E: IntoElement>(render: impl Fn(K, &mut Window, &mut App) -> E + 'static) -> RenderFn<K> {
    Rc::new(move |k, w, cx| render(k, w, cx).into_any_element())
}

/// A virtualized list: `render_row` runs only for rows the model renders, outside the owner's
/// render and outside native callbacks.
pub fn list<K: RowKey, E: IntoElement>(
    id: impl Into<SharedString>,
    state: &ListState<K>,
    render_row: impl Fn(K, &mut Window, &mut App) -> E + 'static,
) -> List<K> {
    build(id.into(), state, ListKind::List, Renderer::Row(row_fn(render_row))).size_full()
}

/// A table: one cell per column, `render_cell(key, column_index, ..)`.
pub fn table<K: RowKey, E: IntoElement>(
    id: impl Into<SharedString>,
    state: &ListState<K>,
    columns: Vec<Column>,
    render_cell: impl Fn(K, usize, &mut Window, &mut App) -> E + 'static,
) -> List<K> {
    let widths = columns.iter().map(|c| c.width).collect();
    let cell: CellFn<K> = Rc::new(move |k, c, w, cx| render_cell(k, c, w, cx).into_any_element());
    build(id.into(), state, ListKind::Table(columns), Renderer::Cells(cell, widths)).size_full()
}

/// Fixed-size cells in as many lanes as fit the width.
pub fn grid<K: RowKey, E: IntoElement>(
    id: impl Into<SharedString>,
    state: &ListState<K>,
    item_size: (Pixels, Pixels),
    render_item: impl Fn(K, &mut Window, &mut App) -> E + 'static,
) -> List<K> {
    let (w, h) = (f64::from(item_size.0.as_f32()), f64::from(item_size.1.as_f32()));
    {
        let core = state.core();
        let mut core = core.borrow_mut();
        let gap = core.grid().map_or(8., |g| g.gap);
        core.set_grid(Some(GridSpec { item_width: w, item_height: h, gap }));
    }
    build(id.into(), state, ListKind::Grid, Renderer::Row(row_fn(render_item))).size_full()
}

/// A tree over [`ListState::set_tree`]; disclosure and indentation are the presenter's.
pub fn tree<K: RowKey, E: IntoElement>(
    id: impl Into<SharedString>,
    state: &ListState<K>,
    render_row: impl Fn(K, &mut Window, &mut App) -> E + 'static,
) -> List<K> {
    build(id.into(), state, ListKind::Tree, Renderer::Row(row_fn(render_row))).size_full()
}

impl<K: RowKey> List<K> {
    pub fn selection(mut self, selection: Selection) -> Self {
        self.selection = selection;
        self
    }

    /// Shown in place of the rows while the list is empty.
    pub fn empty(mut self, empty: impl IntoElement) -> Self {
        self.empty = Some(empty.into_any_element());
        self
    }

    /// Gap between grid cells.
    pub fn gap(self, gap: Pixels) -> Self {
        {
            let core = self.state.core();
            let mut core = core.borrow_mut();
            if let Some(mut g) = core.grid() {
                g.gap = f64::from(gap.as_f32());
                core.set_grid(Some(g));
            }
        }
        self
    }

    pub fn estimated_row_height(self, height: Pixels) -> Self {
        self.state.core().borrow_mut().set_estimate(f64::from(height.as_f32()));
        self
    }

    pub fn on_select(mut self, f: impl Fn(&SelectEvent<K>, &mut Window, &mut App) + 'static) -> Self {
        self.on_select = Some(Rc::new(f));
        if self.selection == Selection::None {
            self.selection = Selection::Single;
        }
        self
    }

    pub fn on_activate(mut self, f: impl Fn(&ActivateEvent<K>, &mut Window, &mut App) + 'static) -> Self {
        self.on_activate = Some(Rc::new(f));
        self
    }

    pub fn on_expand(mut self, f: impl Fn(&ExpandEvent<K>, &mut Window, &mut App) + 'static) -> Self {
        self.on_expand = Some(Rc::new(f));
        self
    }

    /// The viewport reached the first rows: load older ones (infinite scroll up).
    pub fn on_near_start(mut self, f: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_near_start = Some(Rc::new(move |_: &(), w: &mut Window, cx: &mut App| f(w, cx)));
        self
    }

    pub fn on_near_end(mut self, f: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_near_end = Some(Rc::new(move |_: &(), w: &mut Window, cx: &mut App| f(w, cx)));
        self
    }

    pub fn on_column_resize(mut self, f: impl Fn(&ColumnResized, &mut Window, &mut App) + 'static) -> Self {
        self.on_column_resize = Some(Rc::new(f));
        self
    }

    pub fn on_sort(mut self, f: impl Fn(&SortChanged, &mut Window, &mut App) + 'static) -> Self {
        self.on_sort = Some(Rc::new(f));
        self
    }
}

impl<K: RowKey> Styled for List<K> {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl<K: RowKey> FluentBuilder for List<K> {}

impl<K: RowKey> IntoElement for List<K> {
    fn into_any_element(self) -> AnyElement {
        let state = self.state;
        if state.is_empty()
            && let Some(empty) = self.empty
        {
            let mut d = div().id(format!("{}-empty", self.id)).flex().flex_col().items_center().justify_center();
            *d.style() = self.style.clone();
            return d.flex().flex_col().items_center().justify_center().child(empty).into_any_element();
        }
        state.inner.borrow_mut().renderer = Some(self.renderer);
        let props = state.props(self.kind, self.selection, self.style.clone());
        let mut el = ContainerElement::new::<ListWidget>(Key::Id(self.id), props);
        *el.style() = self.style;
        let s = state.clone();
        let (near_start, near_end) = (self.on_near_start, self.on_near_end);
        let el = el.children(state.children()).on(move |e: &ViewportChanged, window, cx| {
            let (start, end) = s.on_viewport(e, cx);
            if start && let Some(f) = &near_start {
                f(&(), window, cx);
            }
            if end && let Some(f) = &near_end {
                f(&(), window, cx);
            }
        });
        let s = state.clone();
        let on_select = self.on_select;
        let el = el.on(move |e: &RowsSelected, window, cx| {
            let keys: Vec<K> = e.0.iter().filter_map(|r| s.key_of(*r)).collect();
            s.clone().set_selected(keys.clone(), cx);
            if let (Some(f), Some(key)) = (&on_select, keys.last().cloned()) {
                f(&SelectEvent { key, keys }, window, cx);
            }
        });
        let s = state.clone();
        let on_activate = self.on_activate;
        let el = el.on(move |e: &RowActivated, window, cx| {
            if let (Some(f), Some(key)) = (&on_activate, s.key_of(e.0)) {
                f(&ActivateEvent { key }, window, cx);
            }
        });
        let s = state.clone();
        let on_expand = self.on_expand;
        let el = el.on(move |e: &RowDisclosed, window, cx| {
            let Some(key) = s.key_of(e.row) else { return };
            s.clone().set_expanded(key.clone(), e.expanded, cx);
            if let Some(f) = &on_expand {
                f(&ExpandEvent { key, expanded: e.expanded }, window, cx);
            }
        });
        let on_resize = self.on_column_resize;
        let el = el.on(move |e: &ColumnResized, window, cx| {
            if let Some(f) = &on_resize {
                f(e, window, cx);
            }
        });
        let on_sort = self.on_sort;
        el.on(move |e: &SortChanged, window, cx| {
            if let Some(f) = &on_sort {
                f(e, window, cx);
            }
        })
        .into_any_element()
    }
}
