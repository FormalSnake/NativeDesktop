//! `nd-virtual` lists on AppKit (ARCHITECTURE.md 7): a view-based `NSTableView` for lists and
//! tables, an `NSOutlineView` for trees, inside an `NSScrollView` that keeps its own momentum and
//! rubber band.
//!
//! `nd-virtual` owns the height tree. AppKit's row height estimation is off
//! (`NSTableViewCanEstimateRowHeights`, registered in the app's defaults domain), so
//! `heightOfRow` answers from a snapshot of the model in O(1): measured or our estimate. Rows are
//! `nd-ui` nodes laid out by Taffy at the width the table gives them ([`ListNative::boundary_width`]);
//! their frames carry the measured height back. Data source calls answer from the snapshot and
//! the rows already rendered, never app code (3.13): a row AppKit shows before the app rendered
//! it gets an empty container, filled when the row arrives.
//!
//! Each transaction's commit applies, in one `beginUpdates`/`endUpdates`: the keyed diff as row
//! inserts and removals, then the heights that changed, then the clip origin the model's anchor
//! asks for, so rows above the viewport changing height or count never move what is on screen.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use nd_core::{NodeId, Rect, Size};
use nd_ui::events::Emit;
use nd_virtual::{
    ColumnResized, ListHandle, ListKind, ListProps, RowActivated, RowDisclosed, RowId, RowMeta, RowsSelected,
    Selection, SortChanged, SortDirection, ViewportChanged, diff_rows,
};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{AnyThread, ClassType, DefinedClass, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSBezierPath, NSCollectionView, NSCollectionViewDataSource, NSCollectionViewDelegate, NSCollectionViewFlowLayout,
    NSCollectionViewItem, NSColor, NSControlTextEditingDelegate, NSEvent, NSIndexPathNSCollectionViewAdditions,
    NSOutlineView, NSOutlineViewDataSource, NSOutlineViewDelegate, NSResponder, NSScrollView, NSTableColumn,
    NSTableColumnResizingOptions, NSTableRowView, NSTableView, NSTableViewAnimationOptions,
    NSTableViewColumnAutoresizingStyle, NSTableViewDataSource, NSTableViewDelegate, NSTableViewStyle, NSTrackingArea,
    NSTrackingAreaOptions, NSView, NSViewBoundsDidChangeNotification,
};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{
    MainThreadMarker, NSDictionary, NSEdgeInsets, NSIndexPath, NSIndexSet, NSInteger, NSMutableIndexSet,
    NSNotification, NSNotificationCenter, NSNumber, NSObject, NSObjectProtocol, NSPoint, NSRect, NSString,
    NSUserDefaults,
};

use crate::events::Wake;
use crate::look::ViewLook;
use crate::views::NdLayerView;

mod swipe;

pub(crate) use swipe::window_press;

/// Turns off AppKit's own row height estimation (on since macOS 13) for this process, in the
/// registration domain so nothing is written to the app's defaults.
fn disable_estimation() {
    thread_local! {
        static DONE: Cell<bool> = const { Cell::new(false) };
    }
    if DONE.with(|d| d.replace(true)) {
        return;
    }
    let key = NSString::from_str("NSTableViewCanEstimateRowHeights");
    let no = NSNumber::new_bool(false);
    let value: &AnyObject = no.as_ref();
    let defaults = NSDictionary::<NSString, AnyObject>::from_slices(&[&*key], &[value]);
    unsafe { NSUserDefaults::standardUserDefaults().registerDefaults(&defaults) };
}

/// The rows as AppKit was last told about them. The data source answers only from this, so it
/// stays consistent between the model changing and the commit that applies the change.
#[derive(Default)]
struct Snapshot {
    ids: Vec<RowId>,
    index: HashMap<RowId, usize>,
    heights: Vec<f64>,
    meta: Vec<RowMeta>,
    /// Tree: children of each row (`None` is the root), from depth.
    children: HashMap<Option<RowId>, Vec<RowId>>,
}

impl Snapshot {
    fn read(core: &nd_virtual::Core) -> Self {
        let ids = core.ids().to_vec();
        let index = ids.iter().enumerate().map(|(i, r)| (*r, i)).collect();
        let heights = (0..ids.len()).map(|i| core.height(i)).collect();
        let meta: Vec<RowMeta> = (0..ids.len()).map(|i| core.meta(i)).collect();
        let mut children: HashMap<Option<RowId>, Vec<RowId>> = HashMap::new();
        let mut parents: Vec<RowId> = Vec::new();
        for (i, id) in ids.iter().enumerate() {
            let depth = usize::from(meta[i].depth);
            parents.truncate(depth);
            children.entry(parents.last().copied()).or_default().push(*id);
            parents.push(*id);
        }
        Self { ids, index, heights, meta, children }
    }
}

/// A rendered row: the node at its root, its view, and for tables its cells' views by column.
struct RowNodes {
    view: Retained<NSView>,
    cells: HashMap<usize, (NodeId, Retained<NSView>)>,
}

pub struct Shared {
    mtm: MainThreadMarker,
    kind: RefCell<ListKind>,
    handle: ListHandle,
    snap: RefCell<Snapshot>,
    /// One container per row and column, created when AppKit first asks for the cell, kept for
    /// the row's life so a row that renders later lands in the cell already on screen.
    containers: RefCell<HashMap<(RowId, usize), Retained<NdLayerView>>>,
    rows: RefCell<HashMap<RowId, RowNodes>>,
    /// Each rendered row's swipe actions, from its tag; the table's row actions answer from it.
    actions: RefCell<HashMap<RowId, nd_virtual::RowActionsSpec>>,
    /// Outline items: one object per row, which the outline tracks by identity.
    items: RefCell<HashMap<RowId, Retained<NSNumber>>>,
    emit: Emit,
    wake: Wake,
    /// Above zero while nd drives the table; delegate callbacks then report nothing.
    programmatic: Cell<u32>,
    /// The model's offset after the last user scroll: a commit leaves the clip view alone while
    /// the model still agrees, so a rubber band or momentum is never fought.
    user_top: Cell<Option<f64>>,
    /// Rows the model wanted rendered when the presenter last asked the app.
    asked: RefCell<Vec<RowId>>,
    edges: Cell<(bool, bool)>,
    /// Points of the list under the window's bars at its top: model offset 0 sits this far down
    /// the clip view, so rows scroll under the bars (which blur them) and rest clear of them.
    bars: Cell<f64>,
    /// Points of the list under a bar floating over its foot: the last rows scroll up clear of it.
    foot: Cell<f64>,
    /// A list without selection never highlights a clicked row.
    selection: Cell<Selection>,
}

impl Shared {
    fn new(mtm: MainThreadMarker, props: &ListProps, emit: Emit, wake: Wake) -> Self {
        Self {
            mtm,
            kind: RefCell::new(props.kind.clone()),
            handle: props.handle.clone(),
            snap: RefCell::new(Snapshot::default()),
            containers: RefCell::default(),
            rows: RefCell::default(),
            actions: RefCell::default(),
            items: RefCell::default(),
            emit,
            wake,
            programmatic: Cell::new(0),
            user_top: Cell::new(None),
            asked: RefCell::default(),
            edges: Cell::new((false, false)),
            bars: Cell::new(0.),
            foot: Cell::new(0.),
            selection: Cell::new(props.selection),
        }
    }

    fn programmatic<R>(&self, f: impl FnOnce() -> R) -> R {
        self.programmatic.set(self.programmatic.get() + 1);
        let r = f();
        self.programmatic.set(self.programmatic.get() - 1);
        r
    }

    fn container(&self, row: RowId, column: usize) -> Retained<NdLayerView> {
        if let Some(c) = self.containers.borrow().get(&(row, column)) {
            return c.clone();
        }
        let c = NdLayerView::new(self.mtm, &ViewLook::default());
        if let Some(r) = self.rows.borrow().get(&row) {
            let view = if matches!(*self.kind.borrow(), ListKind::Table(_)) {
                r.cells.get(&column).map(|(_, v)| v.clone())
            } else {
                Some(r.view.clone())
            };
            if let Some(view) = view {
                c.addSubview(&view);
            }
        }
        self.containers.borrow_mut().insert((row, column), c.clone());
        c
    }

    fn item(&self, row: RowId) -> Retained<NSNumber> {
        self.items.borrow_mut().entry(row).or_insert_with(|| NSNumber::new_u64(row.0)).clone()
    }

    fn row_of_item(item: Option<&AnyObject>) -> Option<RowId> {
        item.and_then(|i| i.downcast_ref::<NSNumber>()).map(|n| RowId(n.as_u64()))
    }

    /// Asks the app for the rows the model wants when they are not the ones rendered, and
    /// reports edges crossed. Queues; the app handles it on a later turn.
    fn ask_for_rows(&self) {
        let (wanted, edges) = {
            let core = self.handle.0.borrow();
            let wanted: Vec<RowId> = core.render_range().filter_map(|i| core.row(i)).collect();
            (wanted, (core.at_start(), core.at_end()))
        };
        let rendered = self.rows.borrow();
        let missing = wanted.len() != rendered.len() || wanted.iter().any(|r| !rendered.contains_key(r));
        drop(rendered);
        let edges_changed = self.edges.replace(edges) != edges;
        if (missing && *self.asked.borrow() != wanted) || edges_changed {
            *self.asked.borrow_mut() = wanted;
            self.emit.emit(ViewportChanged { at_start: edges.0, at_end: edges.1 });
            self.wake.wake();
        }
    }
}

#[derive(Default)]
pub struct HoverRowIvars {
    hovered: Cell<bool>,
    area: RefCell<Option<Retained<NSTrackingArea>>>,
}

define_class!(
    // SAFETY: NSTableRowView has no subclassing requirements and this type does not implement Drop.
    #[unsafe(super(NSTableRowView, NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = HoverRowIvars]
    pub struct NdHoverRowView;

    impl NdHoverRowView {
        #[unsafe(method(updateTrackingAreas))]
        fn update_tracking_areas(&self) {
            let _: () = unsafe { msg_send![super(self), updateTrackingAreas] };
            if let Some(old) = self.ivars().area.take() {
                self.removeTrackingArea(&old);
            }
            let options = NSTrackingAreaOptions::MouseEnteredAndExited
                | NSTrackingAreaOptions::ActiveInKeyWindow
                | NSTrackingAreaOptions::InVisibleRect;
            let area = unsafe {
                NSTrackingArea::initWithRect_options_owner_userInfo(
                    NSTrackingArea::alloc(),
                    NSRect::ZERO,
                    options,
                    Some(self),
                    None,
                )
            };
            self.addTrackingArea(&area);
            self.ivars().area.replace(Some(area));
        }

        #[unsafe(method(mouseEntered:))]
        fn mouse_entered(&self, _event: &NSEvent) {
            self.set_hovered(true);
        }

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, _event: &NSEvent) {
            self.set_hovered(false);
        }

        #[unsafe(method(prepareForReuse))]
        fn prepare_for_reuse(&self) {
            let _: () = unsafe { msg_send![super(self), prepareForReuse] };
            self.ivars().hovered.set(false);
        }

        #[unsafe(method(drawBackgroundInRect:))]
        fn draw_background(&self, rect: NSRect) {
            let _: () = unsafe { msg_send![super(self), drawBackgroundInRect: rect] };
            if !self.ivars().hovered.get() || self.isSelected() {
                return;
            }
            let path = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(self.bounds(), 6., 6.);
            NSColor::quaternarySystemFillColor().setFill();
            path.fill();
        }
    }
);

define_class!(
    // SAFETY: NSTableRowView has no subclassing requirements and this type does not implement Drop.
    #[unsafe(super(NSTableRowView, NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    pub struct NdListRowView;

    impl NdListRowView {
        /// The rounded highlight inset from the row's ends that Notes' and Mail's lists draw.
        /// The table stays full width (`NSTableViewStyle::Inset` would narrow the cells under
        /// rows nd already laid out at the column's width).
        #[unsafe(method(drawSelectionInRect:))]
        fn draw_selection(&self, _rect: NSRect) {
            let b = self.bounds();
            let inset = NSRect::new(
                objc2_foundation::NSPoint::new(b.origin.x + 10., b.origin.y),
                objc2_foundation::NSSize::new((b.size.width - 20.).max(0.), b.size.height),
            );
            let color = if self.isEmphasized() {
                NSColor::selectedContentBackgroundColor()
            } else {
                NSColor::unemphasizedSelectedContentBackgroundColor()
            };
            color.setFill();
            NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(inset, 8., 8.).fill();
        }

        #[unsafe(method(setSelected:))]
        fn set_selected(&self, selected: bool) {
            let _: () = unsafe { msg_send![super(self), setSelected: selected] };
            redraw_text(self);
        }

        #[unsafe(method(setEmphasized:))]
        fn set_emphasized(&self, emphasized: bool) {
            let _: () = unsafe { msg_send![super(self), setEmphasized: emphasized] };
            redraw_text(self);
        }
    }
);

/// The row's text recolours with its selection (`NdTextView::drawn`); layer-backed, it only
/// redraws when asked.
fn redraw_text(view: &NSView) {
    for sub in view.subviews() {
        if sub.downcast_ref::<crate::views::NdTextView>().is_some() {
            sub.setNeedsDisplay(true);
        }
        redraw_text(&sub);
    }
}

impl NdHoverRowView {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(HoverRowIvars::default());
        unsafe { msg_send![super(this), init] }
    }

    fn set_hovered(&self, hovered: bool) {
        if self.ivars().hovered.replace(hovered) != hovered {
            self.setNeedsDisplay(true);
        }
    }
}

pub struct ListIvars {
    shared: Rc<Shared>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements and this type does not implement Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = ListIvars]
    pub struct NdListSource;

    unsafe impl NSObjectProtocol for NdListSource {}
    unsafe impl NSControlTextEditingDelegate for NdListSource {}

    unsafe impl NSTableViewDataSource for NdListSource {
        #[unsafe(method(numberOfRowsInTableView:))]
        fn number_of_rows(&self, _table: &NSTableView) -> NSInteger {
            self.ivars().shared.snap.borrow().ids.len() as NSInteger
        }
    }

    unsafe impl NSTableViewDelegate for NdListSource {
        #[unsafe(method(tableView:heightOfRow:))]
        fn height_of_row(&self, _table: &NSTableView, row: NSInteger) -> CGFloat {
            self.ivars().shared.snap.borrow().heights.get(row as usize).copied().unwrap_or(1.).max(1.)
        }

        #[unsafe(method_id(tableView:viewForTableColumn:row:))]
        fn view_for(
            &self,
            table: &NSTableView,
            column: Option<&NSTableColumn>,
            row: NSInteger,
        ) -> Option<Retained<NSView>> {
            let shared = &self.ivars().shared;
            let id = shared.snap.borrow().ids.get(row as usize).copied();
            let col = column.map_or(0, |c| table.columnWithIdentifier(&c.identifier()).max(0) as usize);
            id.map(|id| Retained::into_super(shared.container(id, col)))
        }

        /// A table's rows get the hover fill Music's song tables have, a plain list's rows the
        /// rounded selection of Notes' list; a source list keeps AppKit's own row view.
        #[unsafe(method_id(tableView:rowViewForRow:))]
        fn row_view_for(&self, table: &NSTableView, _row: NSInteger) -> Option<Retained<NSTableRowView>> {
            let shared = &self.ivars().shared;
            let own = shared.selection.get() != Selection::None && table.style() != NSTableViewStyle::SourceList;
            let mtm = MainThreadMarker::from(self);
            own.then(|| match *shared.kind.borrow() {
                ListKind::Table(_) => Retained::into_super(NdHoverRowView::new(mtm)),
                _ => Retained::into_super(unsafe { msg_send![NdListRowView::alloc(mtm), init] }),
            })
        }

        #[unsafe(method(tableViewSelectionDidChange:))]
        fn selection_changed(&self, notification: &NSNotification) {
            let shared = &self.ivars().shared;
            if shared.programmatic.get() > 0 {
                return;
            }
            let Some(table) = notification.object().and_then(|o| o.downcast::<NSTableView>().ok()) else { return };
            let selected = table.selectedRowIndexes();
            let snap = shared.snap.borrow();
            let rows: Vec<RowId> = (0..snap.ids.len()).filter(|i| selected.containsIndex(*i)).map(|i| snap.ids[i]).collect();
            drop(snap);
            shared.emit.emit(RowsSelected(rows));
            shared.wake.wake();
        }

        #[unsafe(method(tableView:shouldSelectRow:))]
        fn should_select(&self, _table: &NSTableView, _row: NSInteger) -> bool {
            self.ivars().shared.selection.get() != Selection::None
        }

        #[unsafe(method(tableView:didClickTableColumn:))]
        fn clicked_column(&self, table: &NSTableView, column: &NSTableColumn) {
            let shared = &self.ivars().shared;
            let index = table.columnWithIdentifier(&column.identifier());
            let ListKind::Table(columns) = &*shared.kind.borrow() else { return };
            let Some(c) = columns.get(index.max(0) as usize).filter(|c| c.sortable) else { return };
            let direction = match c.sort {
                Some(SortDirection::Ascending) => SortDirection::Descending,
                _ => SortDirection::Ascending,
            };
            shared.emit.emit(SortChanged { column: c.id.clone(), direction });
            shared.wake.wake();
        }

        #[unsafe(method(tableViewColumnDidResize:))]
        fn column_resized(&self, notification: &NSNotification) {
            let shared = &self.ivars().shared;
            if shared.programmatic.get() > 0 {
                return;
            }
            let Some(info) = notification.userInfo() else { return };
            let Some(column) = info
                .objectForKey(&NSString::from_str("NSTableColumn"))
                .and_then(|c| c.downcast::<NSTableColumn>().ok())
            else {
                return;
            };
            let Some(table) = column.tableView() else { return };
            let index = table.columnWithIdentifier(&column.identifier());
            if index >= 0 {
                shared.emit.emit(ColumnResized { column: index as usize, width: column.width() as f32 });
                shared.wake.wake();
            }
        }
    }

    unsafe impl NSOutlineViewDataSource for NdListSource {
        #[unsafe(method(outlineView:numberOfChildrenOfItem:))]
        fn number_of_children(&self, _outline: &NSOutlineView, item: Option<&AnyObject>) -> NSInteger {
            let snap = self.ivars().shared.snap.borrow();
            snap.children.get(&Shared::row_of_item(item)).map_or(0, Vec::len) as NSInteger
        }

        #[unsafe(method_id(outlineView:child:ofItem:))]
        fn child(&self, _outline: &NSOutlineView, index: NSInteger, item: Option<&AnyObject>) -> Retained<AnyObject> {
            let shared = &self.ivars().shared;
            let row = shared.snap.borrow().children[&Shared::row_of_item(item)][index as usize];
            Retained::into_super(Retained::into_super(Retained::into_super(shared.item(row))))
        }

        #[unsafe(method(outlineView:isItemExpandable:))]
        fn expandable(&self, _outline: &NSOutlineView, item: &AnyObject) -> bool {
            let snap = self.ivars().shared.snap.borrow();
            Shared::row_of_item(Some(item)).and_then(|r| snap.index.get(&r)).is_some_and(|i| snap.meta[*i].expandable)
        }
    }

    unsafe impl NSOutlineViewDelegate for NdListSource {
        #[unsafe(method(outlineView:heightOfRowByItem:))]
        fn height_of_item(&self, _outline: &NSOutlineView, item: &AnyObject) -> CGFloat {
            let snap = self.ivars().shared.snap.borrow();
            Shared::row_of_item(Some(item)).and_then(|r| snap.index.get(&r)).map_or(1., |i| snap.heights[*i]).max(1.)
        }

        #[unsafe(method_id(outlineView:viewForTableColumn:item:))]
        fn view_for_item(
            &self,
            _outline: &NSOutlineView,
            _column: Option<&NSTableColumn>,
            item: &AnyObject,
        ) -> Option<Retained<NSView>> {
            Shared::row_of_item(Some(item)).map(|row| Retained::into_super(self.ivars().shared.container(row, 0)))
        }

        #[unsafe(method(outlineView:shouldSelectItem:))]
        fn should_select_item(&self, _outline: &NSOutlineView, _item: &AnyObject) -> bool {
            self.ivars().shared.selection.get() != Selection::None
        }

        #[unsafe(method(outlineViewSelectionDidChange:))]
        fn outline_selection_changed(&self, notification: &NSNotification) {
            let shared = &self.ivars().shared;
            if shared.programmatic.get() > 0 {
                return;
            }
            let Some(outline) = notification.object().and_then(|o| o.downcast::<NSOutlineView>().ok()) else {
                return;
            };
            let selected = outline.selectedRowIndexes();
            let rows: Vec<RowId> = (0..outline.numberOfRows().max(0) as usize)
                .filter(|i| selected.containsIndex(*i))
                .filter_map(|i| Shared::row_of_item(outline.itemAtRow(i as NSInteger).as_deref()))
                .collect();
            shared.emit.emit(RowsSelected(rows));
            shared.wake.wake();
        }

        #[unsafe(method(outlineViewItemDidExpand:))]
        fn did_expand(&self, notification: &NSNotification) {
            self.disclosed(notification, true);
        }

        #[unsafe(method(outlineViewItemDidCollapse:))]
        fn did_collapse(&self, notification: &NSNotification) {
            self.disclosed(notification, false);
        }
    }

    impl NdListSource {
        #[unsafe(method(boundsChanged:))]
        fn bounds_changed(&self, notification: &NSNotification) {
            let shared = &self.ivars().shared;
            if shared.programmatic.get() > 0 {
                return;
            }
            let Some(clip) = notification.object().and_then(|o| o.downcast::<objc2_app_kit::NSClipView>().ok()) else {
                return;
            };
            let y = clip.bounds().origin.y + shared.bars.get();
            let top = {
                let mut core = shared.handle.0.borrow_mut();
                core.scroll_to_offset(y);
                core.scroll_top()
            };
            shared.user_top.set(Some(top));
            // AppKit makes the edge effect's view once rows first scroll under the bars.
            if shared.bars.get() > 0.
                && let Some(scroll) = unsafe { clip.superview() }.and_then(|v| v.downcast::<NSScrollView>().ok())
                && let Some(shown) = crate::scaffold::NdPaneView::edge_shown(&scroll)
            {
                crate::edge::set(&scroll, shown, false);
            }
            shared.ask_for_rows();
        }

        #[unsafe(method(activate:))]
        fn activate(&self, sender: Option<&AnyObject>) {
            let shared = &self.ivars().shared;
            let Some(table) = sender.and_then(|s| s.downcast_ref::<NSTableView>()) else { return };
            let row = table.clickedRow();
            let id = (row >= 0).then(|| shared.snap.borrow().ids.get(row as usize).copied()).flatten();
            if let Some(id) = id {
                shared.emit.emit(RowActivated(id));
                shared.wake.wake();
            }
        }
    }
);

impl NdListSource {
    fn disclosed(&self, notification: &NSNotification, expanded: bool) {
        let shared = &self.ivars().shared;
        if shared.programmatic.get() > 0 {
            return;
        }
        let Some(info) = notification.userInfo() else { return };
        let item = info.objectForKey(&NSString::from_str("NSObject"));
        if let Some(row) = Shared::row_of_item(item.as_deref()) {
            shared.emit.emit(RowDisclosed { row, expanded });
            shared.wake.wake();
        }
    }
}

pub(crate) struct TableNative {
    pub scroll: Retained<NSScrollView>,
    table: Retained<NSTableView>,
    source: Retained<NdListSource>,
    shared: Rc<Shared>,
    props: ListProps,
    /// The model generation AppKit last saw.
    generation: Option<u64>,
    /// Rows whose measured height changed since the last commit.
    heights_dirty: HashSet<RowId>,
    /// Row and cell nodes, to their row.
    node_rows: HashMap<NodeId, RowId>,
    cell_nodes: HashMap<NodeId, (RowId, usize)>,
    size: Size,
    /// The width rows were last laid out at; a new one asks nd to lay them out again.
    row_width: f32,
}

impl TableNative {
    pub fn new(mtm: MainThreadMarker, emit: Emit, wake: Wake, props: &ListProps) -> Self {
        disable_estimation();
        let shared = Rc::new(Shared::new(mtm, props, emit, wake));
        let source = NdListSource::alloc(mtm).set_ivars(ListIvars { shared: shared.clone() });
        let source: Retained<NdListSource> = unsafe { msg_send![super(source), init] };
        let table: Retained<NSTableView> = if props.kind == ListKind::Tree {
            let outline = NSOutlineView::new(mtm);
            outline.setIndentationPerLevel(14.);
            outline.setAutoresizesOutlineColumn(false);
            Retained::into_super(outline)
        } else {
            Retained::into_super(swipe::NdTableView::new(mtm, shared.clone()))
        };
        table.setUsesAutomaticRowHeights(false);
        table.setIntercellSpacing(objc2_foundation::NSSize::new(0., 0.));
        table.setStyle(if props.inset { NSTableViewStyle::SourceList } else { NSTableViewStyle::FullWidth });
        table.setAllowsEmptySelection(true);
        // Columns of data alternate row backgrounds, as Finder's list view does (HIG, Lists and tables).
        table.setUsesAlternatingRowBackgroundColors(matches!(props.kind, ListKind::Table(_)) && !props.inset);
        table.setColumnAutoresizingStyle(match props.kind {
            ListKind::Table(_) => NSTableViewColumnAutoresizingStyle::LastColumnOnlyAutoresizingStyle,
            _ => NSTableViewColumnAutoresizingStyle::UniformColumnAutoresizingStyle,
        });
        unsafe {
            table.setTarget(Some(&source));
            table.setDoubleAction(Some(sel!(activate:)));
        }
        let scroll = NSScrollView::new(mtm);
        scroll.setDocumentView(Some(&table));
        scroll.setHasVerticalScroller(true);
        scroll.setDrawsBackground(false);
        scroll.setAutomaticallyAdjustsContentInsets(false);
        let clip = scroll.contentView();
        clip.setPostsBoundsChangedNotifications(true);
        unsafe {
            NSNotificationCenter::defaultCenter().addObserver_selector_name_object(
                &source,
                sel!(boundsChanged:),
                Some(NSViewBoundsDidChangeNotification),
                Some(&clip),
            );
        }
        let this = Self {
            scroll,
            table,
            source,
            shared,
            props: props.clone(),
            generation: None,
            heights_dirty: HashSet::new(),
            node_rows: HashMap::new(),
            cell_nodes: HashMap::new(),
            size: Size::default(),
            row_width: 0.,
        };
        // Adding the columns resizes the table and moves the clip view; that is setup, not a
        // user scroll, and reported as one it stopped a bottom-aligned list's tail follow.
        this.shared.programmatic(|| this.set_columns());
        unsafe {
            this.table.setDataSource(Some(ProtocolObject::from_ref(&*this.source)));
            this.table.setDelegate(Some(ProtocolObject::from_ref(&*this.source)));
        }
        this
    }

    pub fn view(&self) -> &NSView {
        &self.scroll
    }

    /// The table view, for tests and automation.
    pub fn table(&self) -> &NSTableView {
        &self.table
    }

    fn set_columns(&self) {
        let mtm = self.shared.mtm;
        for c in self.table.tableColumns().iter() {
            self.table.removeTableColumn(&c);
        }
        match &self.props.kind {
            ListKind::Table(columns) => {
                for (i, c) in columns.iter().enumerate() {
                    let column = NSTableColumn::initWithIdentifier(
                        NSTableColumn::alloc(mtm),
                        &NSString::from_str(&format!("nd.{i}")),
                    );
                    column.setTitle(&NSString::from_str(&c.title));
                    column.setWidth(f64::from(c.width));
                    column.setMinWidth(f64::from(c.min_width));
                    column.setResizingMask(if c.resizable {
                        NSTableColumnResizingOptions::UserResizingMask | NSTableColumnResizingOptions::AutoresizingMask
                    } else {
                        NSTableColumnResizingOptions::NoResizing
                    });
                    self.table.addTableColumn(&column);
                }
            }
            kind => {
                let column = NSTableColumn::initWithIdentifier(NSTableColumn::alloc(mtm), &NSString::from_str("nd.0"));
                column.setResizingMask(NSTableColumnResizingOptions::AutoresizingMask);
                self.table.addTableColumn(&column);
                self.table.setHeaderView(None);
                if *kind == ListKind::Tree
                    && let Some(outline) = self.table.downcast_ref::<NSOutlineView>()
                {
                    unsafe { outline.setOutlineTableColumn(Some(&column)) };
                }
            }
        }
    }

    pub fn update(&mut self, props: &ListProps) {
        let columns_changed = match (&self.props.kind, &props.kind) {
            (ListKind::Table(a), ListKind::Table(b)) => {
                a.iter().map(|c| (&c.id, &c.title, c.resizable)).ne(b.iter().map(|c| (&c.id, &c.title, c.resizable)))
            }
            (a, b) => a != b,
        };
        let widths_changed = match (&self.props.kind, &props.kind) {
            (ListKind::Table(a), ListKind::Table(b)) => a.iter().map(|c| c.width).ne(b.iter().map(|c| c.width)),
            _ => false,
        };
        let sort_changed = match (&self.props.kind, &props.kind) {
            (ListKind::Table(a), ListKind::Table(b)) => a.iter().map(|c| c.sort).ne(b.iter().map(|c| c.sort)),
            _ => false,
        };
        *self.shared.kind.borrow_mut() = props.kind.clone();
        let old = std::mem::replace(&mut self.props, props.clone());
        if columns_changed {
            self.shared.programmatic(|| self.set_columns());
        } else if widths_changed || sort_changed {
            let ListKind::Table(columns) = &props.kind else { return };
            let table_columns = self.table.tableColumns();
            self.shared.programmatic(|| {
                for (c, tc) in columns.iter().zip(table_columns.iter()) {
                    if (tc.width() - f64::from(c.width)).abs() > 0.5 {
                        tc.setWidth(f64::from(c.width));
                    }
                    let indicator = c.sort.map(|d| match d {
                        SortDirection::Ascending => "NSAscendingSortIndicator",
                        SortDirection::Descending => "NSDescendingSortIndicator",
                    });
                    let image = indicator.and_then(|n| objc2_app_kit::NSImage::imageNamed(&NSString::from_str(n)));
                    self.table.setIndicatorImage_inTableColumn(image.as_deref(), &tc);
                }
            });
        }
        self.shared.selection.set(props.selection);
        if old.selected != props.selected || old.selection != props.selection {
            self.select();
        }
    }

    fn select(&self) {
        self.table.setAllowsMultipleSelection(self.props.selection == Selection::Multiple);
        let snap = self.shared.snap.borrow();
        let set = NSMutableIndexSet::new();
        for row in &self.props.selected {
            if let Some(i) = snap.index.get(row) {
                set.addIndex(*i);
            }
        }
        drop(snap);
        if self.table.selectedRowIndexes().isEqualToIndexSet(&set) {
            return;
        }
        self.shared.programmatic(|| self.table.selectRowIndexes_byExtendingSelection(&set, false));
    }

    /// A row the owner rendered: its root node and view. Lands in its container if AppKit
    /// already shows that row.
    pub fn attach_row(&mut self, node: NodeId, row: RowId, view: &NSView) {
        self.node_rows.insert(node, row);
        let table = matches!(self.props.kind, ListKind::Table(_));
        if !table && let Some(c) = self.shared.containers.borrow().get(&(row, 0)) {
            c.addSubview(view);
        }
        use objc2::Message;
        self.shared.rows.borrow_mut().insert(row, RowNodes { view: view.retain(), cells: HashMap::new() });
        self.heights_dirty.insert(row);
    }

    pub fn detach_row(&mut self, node: NodeId) {
        let Some(row) = self.node_rows.remove(&node) else { return };
        self.shared.actions.borrow_mut().remove(&row);
        if let Some(table) = self.table.downcast_ref::<swipe::NdTableView>() {
            table.forget(row);
        }
        if let Some(r) = self.shared.rows.borrow_mut().remove(&row) {
            r.view.removeFromSuperview();
            for (n, v) in r.cells.values() {
                v.removeFromSuperview();
                self.cell_nodes.remove(n);
            }
        }
    }

    pub fn is_table(&self) -> bool {
        matches!(self.props.kind, ListKind::Table(_))
    }

    pub fn set_row_actions(&mut self, row: RowId, actions: Option<nd_virtual::RowActionsSpec>) {
        let mut map = self.shared.actions.borrow_mut();
        match actions {
            Some(a) => map.insert(row, a),
            None => map.remove(&row),
        };
    }

    /// Titles of the row actions a swipe on the table's `row` reveals on one edge.
    pub fn row_action_titles(&self, row: usize, leading: bool) -> Vec<String> {
        let Some(id) = self.shared.snap.borrow().ids.get(row).copied() else { return Vec::new() };
        let actions = self.shared.actions.borrow();
        let Some(a) = actions.get(&id) else { return Vec::new() };
        let edge = if leading { &a.leading } else { &a.trailing };
        edge.iter().map(|s| s.title.to_string()).collect()
    }

    pub fn is_row(&self, node: NodeId) -> bool {
        self.node_rows.contains_key(&node)
    }

    /// A table cell (a child of a row) goes into its column's container.
    pub fn attach_cell(&mut self, row_node: NodeId, node: NodeId, column: usize, view: &NSView) {
        let Some(&row) = self.node_rows.get(&row_node) else { return };
        self.cell_nodes.insert(node, (row, column));
        if let Some(c) = self.shared.containers.borrow().get(&(row, column)) {
            c.addSubview(view);
        }
        use objc2::Message;
        if let Some(r) = self.shared.rows.borrow_mut().get_mut(&row) {
            r.cells.insert(column, (node, view.retain()));
        }
    }

    pub fn is_cell(&self, node: NodeId) -> bool {
        self.cell_nodes.contains_key(&node)
    }

    pub fn detach_cell(&mut self, node: NodeId) {
        let Some((row, column)) = self.cell_nodes.remove(&node) else { return };
        if let Some(r) = self.shared.rows.borrow_mut().get_mut(&row)
            && let Some((_, v)) = r.cells.remove(&column)
        {
            v.removeFromSuperview();
        }
    }

    /// The width a row lays out at: the column's, less the outline's indentation.
    pub fn boundary_width(&self, node: NodeId) -> Option<f32> {
        let row = *self.node_rows.get(&node)?;
        let columns = self.table.tableColumns();
        let width = match &self.props.kind {
            ListKind::Table(cs) => {
                cs.iter().zip(columns.iter()).map(|(c, tc)| (tc.width() as f32).max(c.min_width)).sum::<f32>()
            }
            ListKind::Tree => {
                let depth = {
                    let snap = self.shared.snap.borrow();
                    snap.index.get(&row).map_or(0, |i| snap.meta[*i].depth)
                };
                let outline = self.table.downcast_ref::<NSOutlineView>()?;
                let indent = outline.indentationPerLevel() as f32 * (f32::from(depth) + 1.) + 4.;
                (columns.firstObject()?.width() as f32 - indent).max(0.)
            }
            _ => columns.firstObject().map_or(self.size.width, |c| c.width() as f32),
        };
        Some(width.max(1.))
    }

    /// A cell's frame: its size, at its container's origin.
    pub fn set_cell_frame(&mut self, node: NodeId, frame: Rect) {
        let Some((row, column)) = self.cell_nodes.get(&node).copied() else { return };
        let rows = self.shared.rows.borrow();
        let Some((_, view)) = rows.get(&row).and_then(|r| r.cells.get(&column)) else { return };
        let rect = crate::views::ns_rect(Rect::new(0., 0., frame.size.width, frame.size.height));
        if view.frame() != rect {
            view.setFrame(rect);
        }
    }

    /// A row's frame: its measured height goes to the model, its view sits at its container's
    /// origin.
    pub fn set_row_frame(&mut self, node: NodeId, frame: Rect) {
        let Some(&row) = self.node_rows.get(&node) else { return };
        if self.props.handle.0.borrow_mut().report_height(row, f64::from(frame.size.height)) {
            self.heights_dirty.insert(row);
        }
        let rows = self.shared.rows.borrow();
        let Some(r) = rows.get(&row) else { return };
        let rect = crate::views::ns_rect(Rect::new(0., 0., frame.size.width, frame.size.height));
        if r.view.frame() != rect {
            r.view.setFrame(rect);
        }
    }

    /// The list's own frame. Returns true when rows must lay out again at a new width.
    pub fn set_frame(&mut self, frame: Rect) {
        let rect = crate::views::ns_rect(frame);
        // Resizing moves the clip view's bounds, and that is not the user scrolling: reported
        // as one, the first frame of a bottom-aligned list scrolled it to its top and stopped
        // tail follow.
        self.shared.programmatic(|| {
            if self.scroll.frame() != rect {
                self.scroll.setFrame(rect);
            }
            self.scroll.tile();
        });
        self.size = frame.size;
    }

    /// Applies what changed since the last commit in one table update, then puts the clip view
    /// where the model's anchor says. Returns true when rows need laying out again (the width
    /// they get changed).
    pub fn commit(&mut self) -> bool {
        let shared = self.shared.clone();
        let clip = self.scroll.contentView();
        let visible = clip.bounds().size;
        let header = if matches!(self.props.kind, ListKind::Table(_)) {
            self.table.headerView().map_or(0., |h| h.frame().size.height)
        } else {
            0.
        };
        let bars = bars(&self.scroll);
        shared.bars.set(bars);
        let foot = foot(&self.scroll);
        shared.foot.set(foot);
        {
            let mut core = self.props.handle.0.borrow_mut();
            core.set_viewport(f64::from(self.size.width), (visible.height - header - bars - foot).max(0.));
        }
        let generation = self.props.handle.0.borrow().generation();
        let structural = self.generation != Some(generation);
        let tree = self.props.kind == ListKind::Tree;
        shared.programmatic(|| {
            if structural {
                self.generation = Some(generation);
                let next = Snapshot::read(&self.props.handle.0.borrow());
                let old_ids = std::mem::take(&mut shared.snap.borrow_mut().ids);
                let diff = diff_rows(&old_ids, &next.ids);
                {
                    let mut containers = shared.containers.borrow_mut();
                    containers.retain(|(row, _), _| next.index.contains_key(row));
                    shared.items.borrow_mut().retain(|row, _| next.index.contains_key(row));
                }
                *shared.snap.borrow_mut() = next;
                if tree {
                    self.reload_tree();
                } else if old_ids.is_empty() || diff.removed.len() + diff.inserted.len() > 2000 {
                    self.table.reloadData();
                } else if !diff.is_empty() {
                    let removed = index_set(&diff.removed);
                    let inserted = index_set(&diff.inserted);
                    self.table.beginUpdates();
                    self.table.removeRowsAtIndexes_withAnimation(&removed, NSTableViewAnimationOptions::EffectFade);
                    self.table.insertRowsAtIndexes_withAnimation(&inserted, NSTableViewAnimationOptions::EffectFade);
                    self.table.endUpdates();
                }
                self.heights_dirty.clear();
                // Heights may have changed for rows that stayed, too.
                let n = shared.snap.borrow().ids.len();
                if n > 0 && !old_ids.is_empty() {
                    let all = NSIndexSet::indexSetWithIndexesInRange(objc2_foundation::NSRange::new(0, n));
                    self.table.noteHeightOfRowsWithIndexesChanged(&all);
                }
                self.select();
            } else if !self.heights_dirty.is_empty() {
                let mut changed = Vec::new();
                {
                    let core = self.props.handle.0.borrow();
                    let mut snap = shared.snap.borrow_mut();
                    for row in self.heights_dirty.drain() {
                        if let (Some(&i), Some(ci)) = (snap.index.get(&row), core.index_of(row)) {
                            let h = core.height(ci);
                            if (snap.heights[i] - h).abs() > 0.01 {
                                snap.heights[i] = h;
                                changed.push(i);
                            }
                        }
                    }
                }
                if !changed.is_empty() {
                    changed.sort_unstable();
                    self.table.noteHeightOfRowsWithIndexesChanged(&index_set(&changed));
                }
            }
            // Re-tiled now, not on the next display pass, so the document height the clip view
            // is placed against is the rows' sum.
            self.table.tile();
            self.place_clip();
        });
        shared.ask_for_rows();
        let width = self.row_width_now();
        let relayout = (width - self.row_width).abs() > 0.5;
        self.row_width = width;
        relayout
    }

    fn row_width_now(&self) -> f32 {
        let columns = self.table.tableColumns();
        match &self.props.kind {
            ListKind::Table(_) => columns.iter().map(|c| c.width() as f32).sum(),
            _ => columns.firstObject().map_or(0., |c| c.width() as f32),
        }
    }

    fn reload_tree(&self) {
        let Some(outline) = self.table.downcast_ref::<NSOutlineView>() else { return };
        outline.reloadData();
        let expanded: Vec<RowId> = {
            let snap = self.shared.snap.borrow();
            snap.ids.iter().zip(&snap.meta).filter(|(_, m)| m.expanded && m.expandable).map(|(r, _)| *r).collect()
        };
        for row in expanded {
            let item = self.shared.item(row);
            unsafe { outline.expandItem(Some(&item)) };
        }
    }

    fn place_clip(&self) {
        place_clip(&self.scroll, &self.shared, &self.props.handle);
    }
}

/// Lays the model out and moves the clip view to its offset, unless the user's own scroll already
/// put it there. A bottom-aligned list shorter than the viewport rests on the bottom edge through
/// the scroll view's top inset.
fn place_clip(scroll: &NSScrollView, shared: &Shared, handle: &ListHandle) {
    let layout = handle.0.borrow_mut().layout();
    let clip = scroll.contentView();
    let bars = shared.bars.get();
    let inset = (-layout.scroll_top).max(0.) + bars;
    let foot = shared.foot.get();
    let current = scroll.contentInsets();
    if (current.top - inset).abs() > 0.25 || (current.bottom - foot).abs() > 0.25 {
        scroll.setContentInsets(NSEdgeInsets { top: inset, left: 0., bottom: foot, right: 0. });
    }
    if bars > 0.
        && let Some(shown) = crate::scaffold::NdPaneView::edge_shown(scroll)
    {
        crate::edge::set(scroll, shown, false);
    }
    let origin = clip.bounds().origin;
    let target = layout.scroll_top - bars;
    let model_moved = match shared.user_top.take() {
        Some(user) => (user - layout.scroll_top).abs() > 0.25,
        None => true,
    };
    if model_moved && (origin.y - target).abs() > 0.25 {
        clip.scrollToPoint(NSPoint::new(origin.x, target));
        scroll.reflectScrolledClipView(&clip);
    } else if !model_moved {
        shared.user_top.set(Some(layout.scroll_top));
    }
}

impl Drop for TableNative {
    fn drop(&mut self) {
        unsafe {
            NSNotificationCenter::defaultCenter().removeObserver(&self.source);
            self.table.setDataSource(None);
            self.table.setDelegate(None);
        }
    }
}

/// How far the window's bars (a toolbar over full-size content) reach down over the scroll view.
fn bars(scroll: &NSScrollView) -> f64 {
    scroll.safeAreaInsets().top.max(0.)
}

/// How far a bar floating over the pane's foot reaches up over the scroll view.
fn foot(scroll: &NSScrollView) -> f64 {
    scroll.safeAreaInsets().bottom.max(0.)
}

fn index_set(indices: &[usize]) -> Retained<NSIndexSet> {
    let set = NSMutableIndexSet::new();
    for i in indices {
        set.addIndex(*i);
    }
    Retained::into_super(set)
}

/// A grid item: a plain view the data source puts the row's container in.
pub struct GridItemIvars;

define_class!(
    // SAFETY: NSCollectionViewItem may be subclassed; loadView only sets the view.
    #[unsafe(super(NSCollectionViewItem, objc2_app_kit::NSViewController, objc2_app_kit::NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = GridItemIvars]
    pub struct NdGridItem;

    impl NdGridItem {
        #[unsafe(method(loadView))]
        fn load_view(&self) {
            let view = NdLayerView::new(MainThreadMarker::from(self), &ViewLook::default());
            self.setView(&view);
        }
    }
);

pub struct GridIvars {
    shared: Rc<Shared>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements and this type does not implement Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = GridIvars]
    pub struct NdGridSource;

    unsafe impl NSObjectProtocol for NdGridSource {}

    unsafe impl NSCollectionViewDataSource for NdGridSource {
        #[unsafe(method(numberOfSectionsInCollectionView:))]
        fn sections(&self, _cv: &NSCollectionView) -> NSInteger {
            1
        }

        #[unsafe(method(collectionView:numberOfItemsInSection:))]
        fn items(&self, _cv: &NSCollectionView, _section: NSInteger) -> NSInteger {
            self.ivars().shared.snap.borrow().ids.len() as NSInteger
        }

        #[unsafe(method_id(collectionView:itemForRepresentedObjectAtIndexPath:))]
        fn item(&self, cv: &NSCollectionView, path: &NSIndexPath) -> Retained<NSCollectionViewItem> {
            let shared = &self.ivars().shared;
            let item = cv.makeItemWithIdentifier_forIndexPath(&NSString::from_str(GRID_ITEM), path);
            let view = item.view();
            for sub in view.subviews().iter() {
                sub.removeFromSuperview();
            }
            let row = shared.snap.borrow().ids.get(path.item() as usize).copied();
            if let Some(row) = row {
                let container = shared.container(row, 0);
                container.setFrame(view.bounds());
                container.setAutoresizingMask(
                    objc2_app_kit::NSAutoresizingMaskOptions::ViewWidthSizable
                        | objc2_app_kit::NSAutoresizingMaskOptions::ViewHeightSizable,
                );
                view.addSubview(&container);
            }
            item
        }
    }

    unsafe impl NSCollectionViewDelegate for NdGridSource {
        #[unsafe(method(collectionView:didSelectItemsAtIndexPaths:))]
        fn did_select(&self, cv: &NSCollectionView, _paths: &objc2_foundation::NSSet<NSIndexPath>) {
            self.report_selection(cv);
        }

        #[unsafe(method(collectionView:didDeselectItemsAtIndexPaths:))]
        fn did_deselect(&self, cv: &NSCollectionView, _paths: &objc2_foundation::NSSet<NSIndexPath>) {
            self.report_selection(cv);
        }
    }

    impl NdGridSource {
        #[unsafe(method(boundsChanged:))]
        fn bounds_changed(&self, notification: &NSNotification) {
            let shared = &self.ivars().shared;
            if shared.programmatic.get() > 0 {
                return;
            }
            let Some(clip) = notification.object().and_then(|o| o.downcast::<objc2_app_kit::NSClipView>().ok()) else {
                return;
            };
            let y = clip.bounds().origin.y + shared.bars.get();
            let top = {
                let mut core = shared.handle.0.borrow_mut();
                core.scroll_to_offset(y);
                core.scroll_top()
            };
            shared.user_top.set(Some(top));
            shared.ask_for_rows();
        }
    }
);

const GRID_ITEM: &str = "nd.grid.item";

impl NdGridSource {
    fn report_selection(&self, cv: &NSCollectionView) {
        let shared = &self.ivars().shared;
        if shared.programmatic.get() > 0 {
            return;
        }
        let snap = shared.snap.borrow();
        let mut rows: Vec<(usize, RowId)> = cv
            .selectionIndexPaths()
            .iter()
            .filter_map(|p| snap.ids.get(p.item() as usize).map(|r| (p.item() as usize, *r)))
            .collect();
        drop(snap);
        rows.sort_unstable();
        shared.emit.emit(RowsSelected(rows.into_iter().map(|(_, r)| r).collect()));
        shared.wake.wake();
    }
}

/// A grid on `NSCollectionView` with a flow layout of fixed-size items. The model's lanes use
/// the flow layout's own rule (as many items as fit with the gap between them), so rows and the
/// anchor agree with what AppKit places.
pub(crate) struct GridNative {
    pub scroll: Retained<NSScrollView>,
    collection: Retained<NSCollectionView>,
    source: Retained<NdGridSource>,
    shared: Rc<Shared>,
    props: ListProps,
    generation: Option<u64>,
    node_rows: HashMap<NodeId, RowId>,
    size: Size,
}

impl GridNative {
    pub fn new(mtm: MainThreadMarker, emit: Emit, wake: Wake, props: &ListProps) -> Self {
        let shared = Rc::new(Shared::new(mtm, props, emit, wake));
        let source = NdGridSource::alloc(mtm).set_ivars(GridIvars { shared: shared.clone() });
        let source: Retained<NdGridSource> = unsafe { msg_send![super(source), init] };
        let collection = NSCollectionView::new(mtm);
        let layout = NSCollectionViewFlowLayout::new(mtm);
        if let Some(g) = props.handle.0.borrow().grid() {
            layout.setItemSize(objc2_foundation::NSSize::new(g.item_width, g.item_height));
            layout.setMinimumInteritemSpacing(g.gap);
            layout.setMinimumLineSpacing(g.gap);
        }
        layout.setSectionInset(NSEdgeInsets { top: 0., left: 0., bottom: 0., right: 0. });
        collection.setCollectionViewLayout(Some(&layout));
        unsafe {
            collection.registerClass_forItemWithIdentifier(Some(NdGridItem::class()), &NSString::from_str(GRID_ITEM));
            collection.setDataSource(Some(ProtocolObject::from_ref(&*source)));
            collection.setDelegate(Some(ProtocolObject::from_ref(&*source)));
        }
        collection.setSelectable(props.selection != Selection::None);
        collection.setAllowsMultipleSelection(props.selection == Selection::Multiple);
        let scroll = NSScrollView::new(mtm);
        scroll.setDocumentView(Some(&collection));
        scroll.setHasVerticalScroller(true);
        scroll.setDrawsBackground(false);
        scroll.setAutomaticallyAdjustsContentInsets(false);
        let clip = scroll.contentView();
        clip.setPostsBoundsChangedNotifications(true);
        unsafe {
            NSNotificationCenter::defaultCenter().addObserver_selector_name_object(
                &source,
                sel!(boundsChanged:),
                Some(NSViewBoundsDidChangeNotification),
                Some(&clip),
            );
        }
        Self {
            scroll,
            collection,
            source,
            shared,
            props: props.clone(),
            generation: None,
            node_rows: HashMap::new(),
            size: Size::default(),
        }
    }

    pub fn collection(&self) -> &NSCollectionView {
        &self.collection
    }

    pub fn update(&mut self, props: &ListProps) {
        self.collection.setSelectable(props.selection != Selection::None);
        self.collection.setAllowsMultipleSelection(props.selection == Selection::Multiple);
        let changed = self.props.selected != props.selected;
        self.props = props.clone();
        if changed {
            self.select();
        }
    }

    fn select(&self) {
        let snap = self.shared.snap.borrow();
        let paths: Vec<Retained<NSIndexPath>> = self
            .props
            .selected
            .iter()
            .filter_map(|r| snap.index.get(r))
            .map(|i| NSIndexPath::indexPathForItem_inSection(*i as NSInteger, 0))
            .collect();
        drop(snap);
        let set = objc2_foundation::NSSet::from_retained_slice(&paths);
        self.shared.programmatic(|| self.collection.setSelectionIndexPaths(&set));
    }

    pub fn attach_row(&mut self, node: NodeId, row: RowId, view: &NSView) {
        self.node_rows.insert(node, row);
        if let Some(c) = self.shared.containers.borrow().get(&(row, 0)) {
            c.addSubview(view);
        }
        use objc2::Message;
        self.shared.rows.borrow_mut().insert(row, RowNodes { view: view.retain(), cells: HashMap::new() });
    }

    pub fn detach_row(&mut self, node: NodeId) {
        let Some(row) = self.node_rows.remove(&node) else { return };
        if let Some(r) = self.shared.rows.borrow_mut().remove(&row) {
            r.view.removeFromSuperview();
        }
    }

    pub fn is_row(&self, node: NodeId) -> bool {
        self.node_rows.contains_key(&node)
    }

    pub fn boundary_width(&self, node: NodeId) -> Option<f32> {
        self.node_rows.get(&node)?;
        self.props.handle.0.borrow().grid().map(|g| g.item_width as f32)
    }

    pub fn set_row_frame(&mut self, node: NodeId, frame: Rect) {
        let Some(&row) = self.node_rows.get(&node) else { return };
        let rows = self.shared.rows.borrow();
        let Some(r) = rows.get(&row) else { return };
        let rect = crate::views::ns_rect(Rect::new(0., 0., frame.size.width, frame.size.height));
        if r.view.frame() != rect {
            r.view.setFrame(rect);
        }
    }

    pub fn set_frame(&mut self, frame: Rect) {
        let rect = crate::views::ns_rect(frame);
        // Resizing moves the clip view's bounds, and that is not the user scrolling: reported
        // as one, the first frame of a bottom-aligned list scrolled it to its top and stopped
        // tail follow.
        self.shared.programmatic(|| {
            if self.scroll.frame() != rect {
                self.scroll.setFrame(rect);
            }
            self.scroll.tile();
        });
        self.size = frame.size;
    }

    pub fn commit(&mut self) -> bool {
        let shared = self.shared.clone();
        let clip = self.scroll.contentView().bounds().size;
        let bars = bars(&self.scroll);
        shared.bars.set(bars);
        let foot = foot(&self.scroll);
        shared.foot.set(foot);
        self.props.handle.0.borrow_mut().set_viewport(clip.width, (clip.height - bars - foot).max(0.));
        let generation = self.props.handle.0.borrow().generation();
        shared.programmatic(|| {
            if self.generation != Some(generation) {
                self.generation = Some(generation);
                let next = Snapshot::read(&self.props.handle.0.borrow());
                shared.containers.borrow_mut().retain(|(row, _), _| next.index.contains_key(row));
                *shared.snap.borrow_mut() = next;
                self.collection.reloadData();
                self.select();
            }
            // Sized now, so the clip view is placed against the grid's real height.
            self.collection.layoutSubtreeIfNeeded();
            place_clip(&self.scroll, &shared, &self.props.handle);
        });
        shared.ask_for_rows();
        false
    }
}

impl Drop for GridNative {
    fn drop(&mut self) {
        unsafe {
            NSNotificationCenter::defaultCenter().removeObserver(&self.source);
            self.collection.setDataSource(None);
            self.collection.setDelegate(None);
        }
    }
}

/// A list's native presenter: a table (lists, tables, trees) or a collection view (grids).
pub(crate) enum ListNative {
    Table(TableNative),
    Grid(GridNative),
}

impl ListNative {
    pub fn new(mtm: MainThreadMarker, emit: Emit, wake: Wake, props: &ListProps) -> Self {
        match props.kind {
            ListKind::Grid => ListNative::Grid(GridNative::new(mtm, emit, wake, props)),
            _ => ListNative::Table(TableNative::new(mtm, emit, wake, props)),
        }
    }

    pub fn view(&self) -> &NSView {
        match self {
            ListNative::Table(t) => t.view(),
            ListNative::Grid(g) => &g.scroll,
        }
    }

    pub fn scroll(&self) -> &NSScrollView {
        match self {
            ListNative::Table(t) => &t.scroll,
            ListNative::Grid(g) => &g.scroll,
        }
    }

    /// Scrolls the way a wheel would: the clip view moves, and the bounds change it posts is
    /// what the model hears, so tail follow and edge loading run their user-scroll path.
    pub fn scroll_by(&self, dx: f64, dy: f64) {
        let scroll = self.scroll();
        let clip = scroll.contentView();
        let Some(document) = scroll.documentView() else { return };
        let (doc, view) = (document.frame().size, clip.bounds().size);
        let inset = scroll.contentInsets().top;
        let origin = clip.bounds().origin;
        let x = (origin.x + dx).clamp(0., (doc.width - view.width).max(0.));
        let y = (origin.y + dy).clamp(-inset, (doc.height - view.height).max(-inset));
        clip.scrollToPoint(NSPoint::new(x, y));
        scroll.reflectScrolledClipView(&clip);
    }

    /// The document view: the table, outline or collection view.
    pub fn document(&self) -> &NSView {
        match self {
            ListNative::Table(t) => t.table(),
            ListNative::Grid(g) => g.collection(),
        }
    }

    pub fn update(&mut self, props: &ListProps) {
        match self {
            ListNative::Table(t) => t.update(props),
            ListNative::Grid(g) => g.update(props),
        }
    }

    pub fn attach_row(&mut self, node: NodeId, row: RowId, view: &NSView) {
        match self {
            ListNative::Table(t) => t.attach_row(node, row, view),
            ListNative::Grid(g) => g.attach_row(node, row, view),
        }
    }

    pub fn detach_row(&mut self, node: NodeId) {
        match self {
            ListNative::Table(t) => t.detach_row(node),
            ListNative::Grid(g) => g.detach_row(node),
        }
    }

    pub fn is_row(&self, node: NodeId) -> bool {
        match self {
            ListNative::Table(t) => t.is_row(node),
            ListNative::Grid(g) => g.is_row(node),
        }
    }

    pub fn is_table(&self) -> bool {
        matches!(self, ListNative::Table(t) if t.is_table())
    }

    /// A click on the row `node` renders, as the user's: AppKit selects it (alone) and scrolls
    /// it into view, and the selection is reported; `activate` reports the double click's
    /// activation too. `None` when the node is no row of this list.
    pub fn click_row(&self, node: NodeId, activate: bool) -> Option<()> {
        let (shared, row) = match self {
            ListNative::Table(t) => (&t.shared, *t.node_rows.get(&node)?),
            ListNative::Grid(g) => (&g.shared, *g.node_rows.get(&node)?),
        };
        match self {
            ListNative::Table(t) => {
                // The row's place in the table: an outline's rows move as branches open.
                let view = shared.rows.borrow().get(&row).map(|r| r.view.clone());
                let index = view.map_or(-1, |v| t.table.rowForView(&v));
                let index = if index >= 0 { index } else { *shared.snap.borrow().index.get(&row)? as NSInteger };
                let set = NSIndexSet::indexSetWithIndex(index as usize);
                shared.programmatic(|| t.table.selectRowIndexes_byExtendingSelection(&set, false));
                t.table.scrollRowToVisible(index);
            }
            ListNative::Grid(g) => {
                let index = *shared.snap.borrow().index.get(&row)? as NSInteger;
                let path = NSIndexPath::indexPathForItem_inSection(index, 0);
                let set = objc2_foundation::NSSet::from_retained_slice(std::slice::from_ref(&path));
                shared.programmatic(|| g.collection.setSelectionIndexPaths(&set));
            }
        }
        shared.emit.emit(RowsSelected(vec![row]));
        if activate {
            shared.emit.emit(RowActivated(row));
        }
        shared.wake.wake();
        Some(())
    }

    /// Grids have no row actions; tables, lists and trees show them natively.
    pub fn set_row_actions(&mut self, row: RowId, actions: Option<nd_virtual::RowActionsSpec>) {
        if let ListNative::Table(t) = self {
            t.set_row_actions(row, actions);
        }
    }

    pub fn row_action_titles(&self, row: usize, leading: bool) -> Vec<String> {
        match self {
            ListNative::Table(t) => t.row_action_titles(row, leading),
            ListNative::Grid(_) => Vec::new(),
        }
    }

    pub fn is_cell(&self, node: NodeId) -> bool {
        matches!(self, ListNative::Table(t) if t.is_cell(node))
    }

    pub fn attach_cell(&mut self, row_node: NodeId, node: NodeId, column: usize, view: &NSView) {
        if let ListNative::Table(t) = self {
            t.attach_cell(row_node, node, column, view);
        }
    }

    pub fn detach_cell(&mut self, node: NodeId) {
        if let ListNative::Table(t) = self {
            t.detach_cell(node);
        }
    }

    pub fn boundary_width(&self, node: NodeId) -> Option<f32> {
        match self {
            ListNative::Table(t) => t.boundary_width(node),
            ListNative::Grid(g) => g.boundary_width(node),
        }
    }

    pub fn set_cell_frame(&mut self, node: NodeId, frame: Rect) {
        if let ListNative::Table(t) = self {
            t.set_cell_frame(node, frame);
        }
    }

    pub fn set_row_frame(&mut self, node: NodeId, frame: Rect) {
        match self {
            ListNative::Table(t) => t.set_row_frame(node, frame),
            ListNative::Grid(g) => g.set_row_frame(node, frame),
        }
    }

    pub fn set_frame(&mut self, frame: Rect) {
        match self {
            ListNative::Table(t) => {
                t.set_frame(frame);
            }
            ListNative::Grid(g) => g.set_frame(frame),
        }
    }

    pub fn commit(&mut self) -> bool {
        match self {
            ListNative::Table(t) => t.commit(),
            ListNative::Grid(g) => g.commit(),
        }
    }
}
