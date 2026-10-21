//! `source_tree()` / `source_list()` as AppKit's source list: an `NSOutlineView` in
//! `NSTableViewStyle::SourceList` inside an `NSScrollView`, so selection, group rows, disclosure,
//! indentation, row height and the sidebar material are the system's.
//!
//! The outline tracks rows by object identity, so each node id keeps one `NSString` for the
//! widget's life: a nodes update reuses it, and the outline keeps open branches open and the
//! selection on its row. The data source answers from the props it was last given (3.13: no app
//! code in a data source). Programmatic selection, expansion and reloads never report back; a
//! user's selection, disclosure click, double click or action click is reported once.
//!
//! Rows drag in AppKit's own dragging session (the row's image follows the pointer across
//! windows), with the item on the pasteboard as [`DRAG_TYPE`]. The outline does not register as
//! a destination: the windows do, and nd hit-tests targets, so a row and an nd element drop on
//! the same targets. The slot line is a view in the outline placed by [`SourceTreeNative::drop_slot`].

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use nd_components::{
    ActionVisibility, SourceAction, SourceActionClicked, SourceActivated, SourceCollapsed, SourceExpanded, SourceIndex,
    SourceNode, SourceSelected, SourceTreeProps,
};
use nd_ui::events::Emit;
use nd_ui::prelude::SharedString;
use nd_ui::{DragItem, DragPoint, DragSource, DropSlot, GesturePhase};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{AllocAnyThread, DefinedClass, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAccessibility, NSBackgroundStyle, NSBezelStyle, NSButton, NSColor, NSControlTextEditingDelegate, NSDragOperation,
    NSDraggingSession, NSEvent, NSFont, NSFontWeightRegular, NSImageView, NSLayoutConstraint, NSLineBreakMode,
    NSOutlineView, NSOutlineViewDataSource, NSOutlineViewDelegate, NSPasteboardItem, NSPasteboardWriting, NSResponder,
    NSScrollView, NSTableCellView, NSTableColumn, NSTableRowView, NSTableViewDataSource, NSTableViewDelegate,
    NSTableViewStyle, NSTextField, NSTrackingArea, NSTrackingAreaOptions, NSUserInterfaceItemIdentification, NSView,
};
use objc2_foundation::{
    MainThreadMarker, NSArray, NSIndexSet, NSInteger, NSNotification, NSObject, NSObjectProtocol, NSPoint, NSRect,
    NSSize, NSString,
};

use crate::events::Wake;

/// What the data source answers from.
#[derive(Default)]
struct Data {
    nodes: Vec<SourceNode>,
    index: SourceIndex,
    /// One object per node id, kept while the id is listed.
    objects: HashMap<SharedString, Retained<NSString>>,
    actions: HashMap<SharedString, SourceAction>,
    hover: bool,
    drag_kind: Option<SharedString>,
}

impl Data {
    fn node_index(&self, item: Option<&AnyObject>) -> Option<usize> {
        let key = item?.downcast_ref::<NSString>()?.to_string();
        self.index.by_id.get(key.as_str()).copied()
    }

    fn kids(&self, item: Option<&AnyObject>) -> &[usize] {
        match item {
            None => &self.index.roots,
            Some(_) => self.node_index(item).map_or(&[], |i| self.index.children[i].as_slice()),
        }
    }

    fn object(&self, i: usize) -> Retained<NSString> {
        self.objects[&self.nodes[i].id].clone()
    }

    fn set(&mut self, props: &SourceTreeProps) {
        self.nodes = props.nodes.clone();
        self.index = SourceIndex::new(&self.nodes);
        let mut objects = std::mem::take(&mut self.objects);
        self.objects = self
            .index
            .by_id
            .keys()
            .map(|id| (id.clone(), objects.remove(id).unwrap_or_else(|| NSString::from_str(id))))
            .collect();
        self.actions = props.actions.iter().map(|a| (a.id.clone(), a.clone())).collect();
        self.hover = props.action_visibility == ActionVisibility::Hover;
        self.drag_kind = props.drag_kind.clone();
    }

    /// The item row `i` drags as, if it drags.
    fn drag_item(&self, i: usize) -> Option<DragItem> {
        let node = &self.nodes[i];
        let kind = self.drag_kind.clone().filter(|_| !node.section && node.movable)?;
        Some(DragItem::new(kind, node.id.clone()))
    }
}

/// The pasteboard type an nd drag carries, with the dragged item's id as its string.
pub const DRAG_TYPE: &str = "dev.nativedesktop.drag";

/// AppKit's global coordinates (bottom-left origin) as nd's (top-left of the primary screen).
pub(crate) fn screen_point(mtm: MainThreadMarker, p: NSPoint) -> nd_core::Point {
    let primary = objc2_app_kit::NSScreen::screens(mtm).firstObject().map_or(0., |s| s.frame().size.height);
    nd_core::Point { x: p.x as f32, y: (primary - p.y) as f32 }
}

pub(crate) struct Shared {
    data: RefCell<Data>,
    emit: Emit,
    wake: Wake,
    /// Set while nd changes the outline (reload, expansion replay, selection), so the delegate
    /// does not report it back.
    programmatic: Cell<bool>,
}

impl Shared {
    fn report<E: 'static>(&self, event: E) {
        if !self.programmatic.get() {
            self.emit.emit(event);
            self.wake.wake();
        }
    }

    fn key_of(&self, item: Option<&AnyObject>) -> Option<SharedString> {
        let data = self.data.borrow();
        data.node_index(item).map(|i| data.nodes[i].id.clone())
    }
}

pub struct SourceIvars {
    shared: Rc<Shared>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements and this type does not implement Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = SourceIvars]
    struct NdSourceTree;

    unsafe impl NSObjectProtocol for NdSourceTree {}
    unsafe impl NSTableViewDataSource for NdSourceTree {}
    unsafe impl NSControlTextEditingDelegate for NdSourceTree {}
    unsafe impl NSTableViewDelegate for NdSourceTree {}

    unsafe impl NSOutlineViewDataSource for NdSourceTree {
        #[unsafe(method(outlineView:numberOfChildrenOfItem:))]
        fn number_of_children(&self, _outline: &NSOutlineView, item: Option<&AnyObject>) -> NSInteger {
            self.ivars().shared.data.borrow().kids(item).len() as NSInteger
        }

        #[unsafe(method_id(outlineView:child:ofItem:))]
        fn child(&self, _outline: &NSOutlineView, index: NSInteger, item: Option<&AnyObject>) -> Retained<AnyObject> {
            let data = self.ivars().shared.data.borrow();
            let i = data.kids(item)[index as usize];
            Retained::into_super(Retained::into_super(data.object(i)))
        }

        #[unsafe(method_id(outlineView:pasteboardWriterForItem:))]
        fn pasteboard_writer(
            &self,
            _outline: &NSOutlineView,
            item: &AnyObject,
        ) -> Option<Retained<ProtocolObject<dyn NSPasteboardWriting>>> {
            let data = self.ivars().shared.data.borrow();
            data.node_index(Some(item)).and_then(|i| data.drag_item(i)).map(|dragged| {
                let writer = NSPasteboardItem::new();
                writer.setString_forType(&NSString::from_str(&dragged.id), &NSString::from_str(DRAG_TYPE));
                ProtocolObject::from_retained(writer)
            })
        }

        #[unsafe(method(outlineView:draggingSession:willBeginAtPoint:forItems:))]
        fn drag_began(&self, outline: &NSOutlineView, session: &NSDraggingSession, point: NSPoint, items: &NSArray) {
            // A drop nowhere is the app's to place (a tab torn out to a new window), not a
            // failure to animate back from.
            session.setAnimatesToStartingPositionsOnCancelOrFail(false);
            let shared = &self.ivars().shared;
            let item = {
                let data = shared.data.borrow();
                items.firstObject().and_then(|o| data.node_index(Some(&o))).and_then(|i| data.drag_item(i))
            };
            if let Some(item) = item {
                let at = DragPoint::Screen(screen_point(MainThreadMarker::from(outline), point));
                shared.report(DragSource { item, phase: GesturePhase::Began, at, native: true });
            }
        }

        #[unsafe(method(outlineView:draggingSession:endedAtPoint:operation:))]
        fn drag_ended(&self, outline: &NSOutlineView, _session: &NSDraggingSession, point: NSPoint, _op: NSDragOperation) {
            let shared = &self.ivars().shared;
            let item = DragItem::new(shared.data.borrow().drag_kind.clone().unwrap_or_default(), "");
            let at = DragPoint::Screen(screen_point(MainThreadMarker::from(outline), point));
            shared.report(DragSource { item, phase: GesturePhase::Ended, at, native: true });
        }

        // A section is expandable too, or the outline would never show its rows; it just
        // draws no disclosure.
        #[unsafe(method(outlineView:isItemExpandable:))]
        fn expandable(&self, _outline: &NSOutlineView, item: &AnyObject) -> bool {
            let data = self.ivars().shared.data.borrow();
            data.node_index(Some(item)).is_some_and(|i| data.nodes[i].has_children || !data.index.children[i].is_empty())
        }
    }

    unsafe impl NSOutlineViewDelegate for NdSourceTree {
        #[unsafe(method(outlineView:isGroupItem:))]
        fn is_group(&self, _outline: &NSOutlineView, item: &AnyObject) -> bool {
            let data = self.ivars().shared.data.borrow();
            data.node_index(Some(item)).is_some_and(|i| data.nodes[i].section)
        }

        // Sections open and close from app state only: the source list's group look.
        #[unsafe(method(outlineView:shouldShowOutlineCellForItem:))]
        fn shows_disclosure(&self, _outline: &NSOutlineView, item: &AnyObject) -> bool {
            let data = self.ivars().shared.data.borrow();
            data.node_index(Some(item)).is_some_and(|i| !data.nodes[i].section)
        }

        #[unsafe(method(outlineView:shouldSelectItem:))]
        fn should_select(&self, _outline: &NSOutlineView, item: &AnyObject) -> bool {
            let data = self.ivars().shared.data.borrow();
            data.node_index(Some(item)).is_some_and(|i| data.nodes[i].selectable && !data.nodes[i].section)
        }

        #[unsafe(method(outlineView:heightOfRowByItem:))]
        fn height(&self, outline: &NSOutlineView, item: &AnyObject) -> f64 {
            let data = self.ivars().shared.data.borrow();
            let caption = data.node_index(Some(item)).is_some_and(|i| data.nodes[i].caption.is_some());
            // A caption is a second line in the small system font under the row's own height.
            if caption { outline.rowHeight() + caption_line(MainThreadMarker::from(outline)) } else { outline.rowHeight() }
        }

        #[unsafe(method_id(outlineView:rowViewForItem:))]
        fn row_view(&self, outline: &NSOutlineView, _item: &AnyObject) -> Option<Retained<NSTableRowView>> {
            let mtm = MainThreadMarker::from(outline);
            let row = NdSourceRowView::alloc(mtm).set_ivars(RowIvars { hover: Cell::new(self.ivars().shared.data.borrow().hover) });
            let row: Retained<NdSourceRowView> = unsafe { msg_send![super(row), init] };
            Some(Retained::into_super(row))
        }

        #[unsafe(method_id(outlineView:viewForTableColumn:item:))]
        fn view_for(
            &self,
            outline: &NSOutlineView,
            _column: Option<&NSTableColumn>,
            item: &AnyObject,
        ) -> Option<Retained<NSView>> {
            self.cell_for(outline, item)
        }

        #[unsafe(method(outlineViewSelectionDidChange:))]
        fn selection_changed(&self, notification: &NSNotification) {
            let Some(outline) = notification.object().and_then(|o| o.downcast::<NSOutlineView>().ok()) else { return };
            let shared = &self.ivars().shared;
            if let Some(id) = shared.key_of(outline.itemAtRow(outline.selectedRow()).as_deref()) {
                shared.report(SourceSelected { id });
            }
        }

        #[unsafe(method(outlineViewItemDidExpand:))]
        fn did_expand(&self, notification: &NSNotification) {
            let shared = &self.ivars().shared;
            if let Some(id) = shared.key_of(changed_item(notification).as_deref()) {
                shared.report(SourceExpanded { id });
            }
        }

        #[unsafe(method(outlineViewItemDidCollapse:))]
        fn did_collapse(&self, notification: &NSNotification) {
            let shared = &self.ivars().shared;
            if let Some(id) = shared.key_of(changed_item(notification).as_deref()) {
                shared.report(SourceCollapsed { id });
            }
        }
    }

    impl NdSourceTree {
        #[unsafe(method(rowDoubleClicked:))]
        fn row_double_clicked(&self, outline: &NSOutlineView) {
            let shared = &self.ivars().shared;
            if outline.clickedRow() >= 0
                && let Some(id) = shared.key_of(outline.itemAtRow(outline.clickedRow()).as_deref())
            {
                shared.report(SourceActivated { id });
            }
        }
    }
);

impl NdSourceTree {
    fn cell_for(&self, outline: &NSOutlineView, item: &AnyObject) -> Option<Retained<NSView>> {
        let shared = &self.ivars().shared;
        let data = shared.data.borrow();
        let i = data.node_index(Some(item))?;
        let node = &data.nodes[i];
        let id = NSString::from_str(if node.section { SECTION_CELL } else { ROW_CELL });
        let reused = unsafe { outline.makeViewWithIdentifier_owner(&id, Some(self)) }
            .and_then(|v| v.downcast::<NdSourceCell>().ok());
        let cell = reused.unwrap_or_else(|| {
            let cell = NdSourceCell::new(MainThreadMarker::from(outline), Rc::downgrade(shared));
            cell.setIdentifier(Some(&id));
            cell
        });
        let actions: Vec<SourceAction> = node.actions.iter().filter_map(|a| data.actions.get(a).cloned()).collect();
        cell.configure(node, &actions, data.hover);
        Some(Retained::into_super(Retained::into_super(cell)))
    }
}

/// The item an expand or collapse notification names (`userInfo["NSObject"]`).
fn changed_item(notification: &NSNotification) -> Option<Retained<AnyObject>> {
    let info = notification.userInfo()?;
    info.objectForKey(&NSString::from_str("NSObject"))
}

fn caption_line(mtm: MainThreadMarker) -> f64 {
    let probe = NSTextField::labelWithString(&NSString::from_str("Ag"), mtm);
    probe.setFont(Some(&NSFont::systemFontOfSize(NSFont::smallSystemFontSize())));
    probe.intrinsicContentSize().height.ceil()
}

const ROW_CELL: &str = "nd.source.row";
const SECTION_CELL: &str = "nd.source.section";

pub struct RowIvars {
    hover: Cell<bool>,
}

define_class!(
    /// Shows a row's trailing actions while the pointer is over it, when they show on hover.
    // SAFETY: NSTableRowView has no subclassing requirements beyond calling super; no Drop.
    #[unsafe(super(NSTableRowView, NSView, NSResponder))]
    #[thread_kind = MainThreadOnly]
    #[ivars = RowIvars]
    struct NdSourceRowView;

    impl NdSourceRowView {
        #[unsafe(method(updateTrackingAreas))]
        fn update_tracking_areas(&self) {
            let _: () = unsafe { msg_send![super(self), updateTrackingAreas] };
            for area in self.trackingAreas().iter() {
                if area.owner().is_some_and(|o| std::ptr::eq(&*o, (self as *const Self).cast())) {
                    self.removeTrackingArea(&area);
                }
            }
            if !self.ivars().hover.get() {
                return;
            }
            let options = NSTrackingAreaOptions::MouseEnteredAndExited | NSTrackingAreaOptions::ActiveInKeyWindow;
            let area = unsafe {
                NSTrackingArea::initWithRect_options_owner_userInfo(
                    NSTrackingArea::alloc(),
                    self.bounds(),
                    options,
                    Some(self),
                    None,
                )
            };
            self.addTrackingArea(&area);
        }

        #[unsafe(method(mouseEntered:))]
        fn mouse_entered(&self, _event: &NSEvent) {
            self.show_actions(true);
        }

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, _event: &NSEvent) {
            self.show_actions(false);
        }
    }
);

impl NdSourceRowView {
    fn show_actions(&self, visible: bool) {
        for sub in self.subviews().iter() {
            if let Some(cell) = sub.downcast_ref::<NdSourceCell>() {
                cell.set_actions_visible(visible);
            }
        }
    }
}

pub struct CellIvars {
    tree: Weak<Shared>,
    icon: Retained<NSImageView>,
    caption: Retained<NSTextField>,
    badge: Retained<NSTextField>,
    actions: Retained<NSView>,
    /// The node the cell shows now, and the action ids of its buttons in order.
    node: RefCell<SharedString>,
    buttons: RefCell<Vec<(Retained<NSButton>, SharedString)>>,
    constraints: RefCell<Vec<Retained<NSLayoutConstraint>>>,
}

define_class!(
    /// A source list row: the system's image view and text field in an `NSTableCellView`, an
    /// optional caption line under the title, the badge and trailing actions at the end.
    /// Spacing is AppKit's system spacing and sizes are the controls' own, so the row matches
    /// Finder's and Mail's.
    // SAFETY: as above.
    #[unsafe(super(NSTableCellView, NSView, NSResponder))]
    #[thread_kind = MainThreadOnly]
    #[ivars = CellIvars]
    struct NdSourceCell;

    impl NdSourceCell {
        #[unsafe(method(setBackgroundStyle:))]
        fn set_background_style(&self, style: NSBackgroundStyle) {
            let _: () = unsafe { msg_send![super(self), setBackgroundStyle: style] };
            self.recolour(style);
        }

        #[unsafe(method(actionClicked:))]
        fn action_clicked(&self, sender: &NSButton) {
            self.fire(sender);
        }
    }
);

impl NdSourceCell {
    fn fire(&self, sender: &NSButton) {
        let Some(tree) = self.ivars().tree.upgrade() else { return };
        let action = {
            let buttons = self.ivars().buttons.borrow();
            buttons.iter().find(|(b, _)| std::ptr::eq::<NSButton>(&**b, sender)).map(|(_, a)| a.clone())
        };
        let Some(action) = action else { return };
        let id = self.ivars().node.borrow().clone();
        tree.report(SourceActionClicked { id, action });
    }

    fn new(mtm: MainThreadMarker, tree: Weak<Shared>) -> Retained<Self> {
        let label = || {
            let f = NSTextField::labelWithString(&NSString::new(), mtm);
            f.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
            f.setTranslatesAutoresizingMaskIntoConstraints(false);
            f
        };
        let ivars = CellIvars {
            tree,
            icon: NSImageView::new(mtm),
            caption: label(),
            badge: label(),
            actions: NSView::new(mtm),
            node: RefCell::default(),
            buttons: RefCell::default(),
            constraints: RefCell::default(),
        };
        let this = Self::alloc(mtm).set_ivars(ivars);
        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] };
        let title = label();
        this.addSubview(&title);
        unsafe { this.setTextField(Some(&title)) };
        let iv = this.ivars();
        iv.icon.setTranslatesAutoresizingMaskIntoConstraints(false);
        iv.caption.setFont(Some(&NSFont::systemFontOfSize(NSFont::smallSystemFontSize())));
        iv.badge
            .setFont(Some(&NSFont::monospacedDigitSystemFontOfSize_weight(NSFont::smallSystemFontSize(), unsafe {
                NSFontWeightRegular
            })));
        iv.actions.setTranslatesAutoresizingMaskIntoConstraints(false);
        for v in [&*iv.icon as &NSView, &iv.caption, &iv.badge, &iv.actions] {
            this.addSubview(v);
        }
        unsafe { this.setImageView(Some(&iv.icon)) };
        this
    }

    fn title(&self) -> Retained<NSTextField> {
        unsafe { self.textField() }.expect("title field")
    }

    fn configure(&self, node: &SourceNode, actions: &[SourceAction], hover: bool) {
        let mtm = MainThreadMarker::from(self);
        let iv = self.ivars();
        *iv.node.borrow_mut() = node.id.clone();
        let title = self.title();
        title.setStringValue(&NSString::from_str(&node.title));
        let image = node.icon.filter(|_| !node.section).and_then(|i| crate::icons::symbol(i.lucide()));
        iv.icon.setImage(image.as_deref());
        iv.icon.setHidden(image.is_none());
        let caption = node.caption.as_deref().filter(|_| !node.section);
        iv.caption.setStringValue(&NSString::from_str(caption.unwrap_or("")));
        iv.caption.setHidden(caption.is_none());
        let badge = node.badge.as_deref().filter(|_| !node.section);
        iv.badge.setStringValue(&NSString::from_str(badge.unwrap_or("")));
        iv.badge.setHidden(badge.is_none());

        for (b, _) in iv.buttons.borrow_mut().drain(..) {
            b.removeFromSuperview();
        }
        let mut buttons = Vec::new();
        for action in actions.iter().filter(|_| !node.section) {
            let target: &AnyObject = self;
            let button = match &action.label {
                Some(label) => {
                    let b = unsafe {
                        NSButton::buttonWithTitle_target_action(
                            &NSString::from_str(label),
                            Some(target),
                            Some(sel!(actionClicked:)),
                            mtm,
                        )
                    };
                    b.setBezelStyle(NSBezelStyle::Push);
                    b.setControlSize(objc2_app_kit::NSControlSize::Small);
                    b
                }
                None => {
                    let image = crate::icons::symbol(action.icon.lucide());
                    let b = unsafe {
                        match image {
                            Some(image) => NSButton::buttonWithImage_target_action(
                                &image,
                                Some(target),
                                Some(sel!(actionClicked:)),
                                mtm,
                            ),
                            None => NSButton::buttonWithTitle_target_action(
                                &NSString::from_str(action.icon.lucide()),
                                Some(target),
                                Some(sel!(actionClicked:)),
                                mtm,
                            ),
                        }
                    };
                    b.setBordered(false);
                    b
                }
            };
            if action.destructive {
                button.setContentTintColor(Some(&NSColor::systemRedColor()));
            }
            if let Some(tip) = &action.tooltip {
                button.setToolTip(Some(&NSString::from_str(tip)));
            }
            button.setAccessibilityLabel(Some(&NSString::from_str(&action.accessible_label())));
            button.setTranslatesAutoresizingMaskIntoConstraints(false);
            iv.actions.addSubview(&button);
            buttons.push((button, action.id.clone()));
        }
        *iv.buttons.borrow_mut() = buttons;
        self.constrain(node.section);
        self.recolour(self.backgroundStyle());
        // Alpha, never hidden: a hidden button would hand its width to the title and the row
        // would shift under the pointer.
        self.set_actions_visible(!hover);
    }

    pub(crate) fn set_actions_visible(&self, visible: bool) {
        for (b, _) in self.ivars().buttons.borrow().iter() {
            b.setAlphaValue(if visible { 1. } else { 0. });
            b.setEnabled(visible);
        }
    }

    fn recolour(&self, style: NSBackgroundStyle) {
        let iv = self.ivars();
        let secondary = if style == NSBackgroundStyle::Emphasized {
            NSColor::alternateSelectedControlTextColor()
        } else {
            NSColor::secondaryLabelColor()
        };
        iv.caption.setTextColor(Some(&secondary));
        // secondaryLabelColor badges are 3.95:1 on a light sidebar and 2.3:1 on an unemphasized
        // selection, under WCAG AA for text this size.
        let badge = if style == NSBackgroundStyle::Emphasized { secondary } else { NSColor::labelColor() };
        iv.badge.setTextColor(Some(&badge));
    }

    /// The row's constraints for what it shows now. Hidden views still constrain, so every
    /// absent element's slot goes with it.
    fn constrain(&self, section: bool) {
        let iv = self.ivars();
        let title = self.title();
        NSLayoutConstraint::deactivateConstraints(&NSArray::from_retained_slice(&iv.constraints.borrow()));
        let mut c: Vec<Retained<NSLayoutConstraint>> = Vec::new();
        let has_icon = !iv.icon.isHidden();
        let has_caption = !iv.caption.isHidden();
        if has_icon {
            c.push(iv.icon.leadingAnchor().constraintEqualToAnchor(&self.leadingAnchor()));
            c.push(iv.icon.centerYAnchor().constraintEqualToAnchor(&self.centerYAnchor()));
            c.push(
                title
                    .leadingAnchor()
                    .constraintEqualToSystemSpacingAfterAnchor_multiplier(&iv.icon.trailingAnchor(), 1.),
            );
        } else {
            c.push(title.leadingAnchor().constraintEqualToAnchor(&self.leadingAnchor()));
        }
        if has_caption {
            c.push(title.topAnchor().constraintEqualToAnchor(&self.topAnchor()));
            c.push(iv.caption.topAnchor().constraintEqualToAnchor(&title.bottomAnchor()));
            c.push(iv.caption.leadingAnchor().constraintEqualToAnchor(&title.leadingAnchor()));
            c.push(iv.caption.trailingAnchor().constraintLessThanOrEqualToAnchor(&iv.actions.leadingAnchor()));
        } else {
            c.push(title.centerYAnchor().constraintEqualToAnchor(&self.centerYAnchor()));
        }
        c.push(iv.actions.trailingAnchor().constraintEqualToAnchor(&self.trailingAnchor()));
        c.push(iv.actions.centerYAnchor().constraintEqualToAnchor(&self.centerYAnchor()));
        let buttons = iv.buttons.borrow();
        let mut previous: Option<&NSButton> = None;
        for (b, _) in buttons.iter() {
            c.push(b.centerYAnchor().constraintEqualToAnchor(&iv.actions.centerYAnchor()));
            c.push(b.topAnchor().constraintGreaterThanOrEqualToAnchor(&iv.actions.topAnchor()));
            match previous {
                Some(p) => c.push(
                    b.leadingAnchor().constraintEqualToSystemSpacingAfterAnchor_multiplier(&p.trailingAnchor(), 1.),
                ),
                None => c.push(b.leadingAnchor().constraintEqualToAnchor(&iv.actions.leadingAnchor())),
            }
            previous = Some(b);
        }
        match previous {
            Some(p) => c.push(p.trailingAnchor().constraintEqualToAnchor(&iv.actions.trailingAnchor())),
            None => c.push(iv.actions.widthAnchor().constraintEqualToConstant(0.)),
        }
        let end = if iv.badge.isHidden() {
            &*iv.actions as &NSView
        } else {
            c.push(iv.badge.centerYAnchor().constraintEqualToAnchor(&title.centerYAnchor()));
            c.push(if buttons.is_empty() {
                iv.badge.trailingAnchor().constraintEqualToAnchor(&iv.actions.leadingAnchor())
            } else {
                iv.actions
                    .leadingAnchor()
                    .constraintEqualToSystemSpacingAfterAnchor_multiplier(&iv.badge.trailingAnchor(), 1.)
            });
            &iv.badge
        };
        c.push(
            end.leadingAnchor()
                .constraintGreaterThanOrEqualToSystemSpacingAfterAnchor_multiplier(&title.trailingAnchor(), 1.),
        );
        let _ = section;
        NSLayoutConstraint::activateConstraints(&NSArray::from_retained_slice(&c));
        *iv.constraints.borrow_mut() = c;
    }
}

define_class!(
    /// The empty state: icon, title and description centred in the list.
    // SAFETY: as above.
    #[unsafe(super(NSView, NSResponder))]
    #[thread_kind = MainThreadOnly]
    struct NdSourceEmpty;

    impl NdSourceEmpty {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        // Clicks reach the list below.
        #[unsafe(method_id(hitTest:))]
        fn hit_test(&self, _point: objc2_foundation::NSPoint) -> Option<Retained<NSView>> {
            None
        }
    }
);

pub(crate) struct SourceTreeNative {
    pub scroll: Retained<NSScrollView>,
    outline: Retained<NSOutlineView>,
    source: Retained<NdSourceTree>,
    empty: Retained<NSView>,
    empty_title: Retained<NSTextField>,
    empty_description: Retained<NSTextField>,
    empty_icon: Retained<NSImageView>,
    /// Marks where a drag over the list would land.
    slot_line: Retained<NSView>,
    props: SourceTreeProps,
}

impl SourceTreeNative {
    pub fn new(mtm: MainThreadMarker, emit: Emit, wake: Wake, props: &SourceTreeProps) -> Self {
        let outline = NSOutlineView::new(mtm);
        outline.setStyle(NSTableViewStyle::SourceList);
        outline.setHeaderView(None);
        let column = NSTableColumn::initWithIdentifier(NSTableColumn::alloc(mtm), &NSString::from_str("nd.source"));
        outline.addTableColumn(&column);
        unsafe { outline.setOutlineTableColumn(Some(&column)) };
        outline.setFloatsGroupRows(false);
        let shared = Rc::new(Shared { data: RefCell::default(), emit, wake, programmatic: Cell::new(false) });
        let source = NdSourceTree::alloc(mtm).set_ivars(SourceIvars { shared });
        let source: Retained<NdSourceTree> = unsafe { msg_send![super(source), init] };
        unsafe {
            outline.setDataSource(Some(ProtocolObject::from_ref(&*source)));
            outline.setDelegate(Some(ProtocolObject::from_ref(&*source)));
            outline.setTarget(Some(&source));
            outline.setDoubleAction(Some(sel!(rowDoubleClicked:)));
        }
        let scroll = NSScrollView::new(mtm);
        scroll.setDocumentView(Some(&outline));
        scroll.setHasVerticalScroller(true);
        scroll.setDrawsBackground(false);

        let empty: Retained<NdSourceEmpty> =
            unsafe { msg_send![NdSourceEmpty::alloc(mtm), initWithFrame: NSRect::ZERO] };
        let empty: Retained<NSView> = Retained::into_super(empty);
        let empty_icon = NSImageView::new(mtm);
        empty_icon.setContentTintColor(Some(&NSColor::tertiaryLabelColor()));
        let empty_title = NSTextField::labelWithString(&NSString::new(), mtm);
        empty_title.setTextColor(Some(&NSColor::secondaryLabelColor()));
        empty_title.setAlignment(objc2_app_kit::NSTextAlignment::Center);
        let empty_description = NSTextField::wrappingLabelWithString(&NSString::new(), mtm);
        empty_description.setTextColor(Some(&NSColor::tertiaryLabelColor()));
        empty_description.setFont(Some(&NSFont::systemFontOfSize(NSFont::smallSystemFontSize())));
        empty_description.setAlignment(objc2_app_kit::NSTextAlignment::Center);
        for v in [&*empty_icon as &NSView, &empty_title, &empty_description] {
            v.setTranslatesAutoresizingMaskIntoConstraints(false);
            empty.addSubview(v);
        }
        NSLayoutConstraint::activateConstraints(&NSArray::from_retained_slice(&[
            empty_title.centerXAnchor().constraintEqualToAnchor(&empty.centerXAnchor()),
            empty_title.centerYAnchor().constraintEqualToAnchor(&empty.centerYAnchor()),
            empty_title.widthAnchor().constraintLessThanOrEqualToAnchor(&empty.widthAnchor()),
            empty_icon.centerXAnchor().constraintEqualToAnchor(&empty.centerXAnchor()),
            empty_title
                .topAnchor()
                .constraintEqualToSystemSpacingBelowAnchor_multiplier(&empty_icon.bottomAnchor(), 1.),
            empty_description.centerXAnchor().constraintEqualToAnchor(&empty.centerXAnchor()),
            empty_description
                .topAnchor()
                .constraintEqualToSystemSpacingBelowAnchor_multiplier(&empty_title.bottomAnchor(), 1.),
            empty_description.widthAnchor().constraintLessThanOrEqualToAnchor(&empty.widthAnchor()),
        ]));
        empty.setAutoresizingMask(
            objc2_app_kit::NSAutoresizingMaskOptions::ViewWidthSizable
                | objc2_app_kit::NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        empty.setHidden(true);
        scroll.addSubview(&empty);

        let slot_line = NSView::new(mtm);
        slot_line.setWantsLayer(true);
        if let Some(layer) = slot_line.layer() {
            layer.setBackgroundColor(Some(&NSColor::controlAccentColor().CGColor()));
        }
        slot_line.setHidden(true);
        outline.addSubview(&slot_line);
        outline.setDraggingSourceOperationMask_forLocal(NSDragOperation::Move | NSDragOperation::Generic, true);
        outline.setDraggingSourceOperationMask_forLocal(NSDragOperation::None, false);

        let mut this = Self {
            scroll,
            outline,
            source,
            empty,
            empty_title,
            empty_description,
            empty_icon,
            slot_line,
            props: props.clone(),
        };
        this.reload(props);
        this
    }

    pub fn view(&self) -> &NSView {
        &self.scroll
    }

    pub fn outline(&self) -> &NSOutlineView {
        &self.outline
    }

    /// The object the outline tracks `id` by, for tests of identity across updates.
    pub fn item(&self, id: &str) -> Option<Retained<NSString>> {
        self.shared().data.borrow().objects.get(id).cloned()
    }

    fn shared(&self) -> &Rc<Shared> {
        &self.source.ivars().shared
    }

    pub fn update(&mut self, props: &SourceTreeProps) {
        if props.nodes != self.props.nodes
            || props.actions != self.props.actions
            || props.action_visibility != self.props.action_visibility
        {
            self.reload(props);
        } else if props.selected != self.props.selected {
            self.props.selected = props.selected.clone();
            self.select();
        }
        if props.empty != self.props.empty {
            self.props.empty = props.empty.clone();
            self.update_empty();
        }
        if props.drag_kind != self.props.drag_kind || props.accepts != self.props.accepts {
            self.props.drag_kind = props.drag_kind.clone();
            self.props.accepts = props.accepts.clone();
            self.shared().data.borrow_mut().drag_kind = props.drag_kind.clone();
        }
    }

    /// Where a drag at `at` (window points, top-left origin) would land, with the slot line
    /// shown there; `None` hides the line.
    pub fn drop_slot(&self, at: Option<nd_core::Point>) -> Option<DropSlot> {
        let Some(at) = at else {
            self.slot_line.setHidden(true);
            return None;
        };
        let window = self.outline.window()?;
        let height = window.contentView().map_or(0., |v| v.frame().size.height);
        let p = self.outline.convertPoint_fromView(NSPoint::new(f64::from(at.x), height - f64::from(at.y)), None);
        let rows = self.outline.numberOfRows();
        let row = if p.y < 0. { 0 } else { self.outline.rowAtPoint(p) };
        let data = self.shared().data.borrow();
        let visible = data.index.visible(&data.nodes);
        let (slot, line_y, line_x) = if row < 0 || row >= rows {
            let last = rows - 1;
            let y = if last >= 0 {
                self.outline.rectOfRow(last).origin.y + self.outline.rectOfRow(last).size.height
            } else {
                0.
            };
            (data.index.drop_slot(&data.nodes, &visible, None, false), y, 0.)
        } else {
            let rect = self.outline.rectOfRow(row);
            let upper = p.y < rect.origin.y + rect.size.height / 2.;
            let node = data.node_index(self.outline.itemAtRow(row).as_deref());
            let at = node.and_then(|i| visible.iter().position(|(v, _)| *v == i));
            let slot = data.index.drop_slot(&data.nodes, &visible, at, upper);
            let below = !upper || node.is_some_and(|i| data.nodes[i].section);
            let y = if below { rect.origin.y + rect.size.height } else { rect.origin.y };
            let x = self.outline.frameOfCellAtColumn_row(0, row).origin.x;
            (slot, y, x)
        };
        let width = self.outline.bounds().size.width;
        let thickness = 2.; // nd-lint: allow, matches NSTableView's own drop indicator
        self.slot_line.setFrame(NSRect::new(
            NSPoint::new(line_x, line_y - thickness / 2.),
            NSSize::new((width - line_x).max(0.), thickness),
        ));
        if let Some(layer) = self.slot_line.layer() {
            layer.setCornerRadius(thickness / 2.);
        }
        // Above the row views the outline adds as it scrolls.
        self.outline.addSubview(&self.slot_line);
        self.slot_line.setHidden(false);
        Some(slot)
    }

    fn reload(&mut self, props: &SourceTreeProps) {
        self.props = props.clone();
        let shared = self.shared().clone();
        shared.data.borrow_mut().set(props);
        shared.programmatic.set(true);
        self.outline.reloadData();
        // Parent first, in list order: a child expands only once its parent's rows exist.
        let order: Vec<(Retained<NSString>, bool)> = {
            let data = shared.data.borrow();
            data.index.visible_all().into_iter().map(|i| (data.object(i), data.nodes[i].expanded)).collect()
        };
        for (object, expanded) in order {
            let any: &AnyObject = &object;
            if expanded {
                unsafe { self.outline.expandItem(Some(any)) };
            } else {
                unsafe { self.outline.collapseItem(Some(any)) };
            }
        }
        shared.programmatic.set(false);
        self.select_now(true);
        self.update_empty();
    }

    fn update_empty(&self) {
        let empty = self.shared().data.borrow().nodes.is_empty();
        let state = self.props.empty.as_ref().filter(|_| empty);
        self.empty.setHidden(state.is_none());
        if let Some(state) = state {
            self.empty.setFrame(self.scroll.bounds());
            self.empty_title.setStringValue(&NSString::from_str(&state.title));
            let description = state.description.as_deref().unwrap_or("");
            self.empty_description.setStringValue(&NSString::from_str(description));
            self.empty_description.setHidden(description.is_empty());
            let image = state.icon.and_then(|i| crate::icons::symbol(i.lucide()));
            self.empty_icon.setHidden(image.is_none());
            self.empty_icon.setImage(image.as_deref());
        }
    }

    fn select(&self) {
        self.select_now(false);
    }

    fn select_now(&self, force: bool) {
        let shared = self.shared();
        let current = shared.key_of(self.outline.itemAtRow(self.outline.selectedRow()).as_deref());
        let wanted = self.props.selected.clone();
        if current == wanted && !force {
            return;
        }
        shared.programmatic.set(true);
        let object = wanted.as_ref().and_then(|k| shared.data.borrow().objects.get(k).cloned());
        let row = object.map(|o| unsafe { self.outline.rowForItem(Some(&o)) }).filter(|r| *r >= 0);
        match row {
            Some(row) => {
                self.outline.selectRowIndexes_byExtendingSelection(&NSIndexSet::indexSetWithIndex(row as usize), false)
            }
            None => unsafe { self.outline.deselectAll(None) },
        }
        shared.programmatic.set(false);
    }

    /// The empty state's title while it shows.
    pub fn empty_title(&self) -> Option<String> {
        (!self.empty.isHidden()).then(|| self.empty_title.stringValue().to_string())
    }

    /// Clicks action `action` on row `id`'s cell as its button would, for automation.
    pub fn click_action(&self, id: &str, action: &str) -> bool {
        let Some(object) = self.item(id) else { return false };
        let row = unsafe { self.outline.rowForItem(Some(&object)) };
        if row < 0 {
            return false;
        }
        let Some(cell) =
            self.outline.viewAtColumn_row_makeIfNecessary(0, row, true).and_then(|v| v.downcast::<NdSourceCell>().ok())
        else {
            return false;
        };
        let button = cell.ivars().buttons.borrow().iter().find(|(_, a)| a == action).map(|(b, _)| b.clone());
        let Some(button) = button else { return false };
        cell.fire(&button);
        true
    }

    /// Alpha of row `id`'s action buttons: 0 while hidden for hover.
    pub fn action_alpha(&self, id: &str) -> Vec<f64> {
        let Some(object) = self.item(id) else { return Vec::new() };
        let row = unsafe { self.outline.rowForItem(Some(&object)) };
        if row < 0 {
            return Vec::new();
        }
        self.outline
            .viewAtColumn_row_makeIfNecessary(0, row, false)
            .and_then(|v| v.downcast::<NdSourceCell>().ok())
            .map(|c| c.ivars().buttons.borrow().iter().map(|(b, _)| b.alphaValue()).collect())
            .unwrap_or_default()
    }

    /// Shows row `id`'s actions as a pointer entering it would.
    pub fn hover(&self, id: &str, inside: bool) {
        let Some(object) = self.item(id) else { return };
        let row = unsafe { self.outline.rowForItem(Some(&object)) };
        if row < 0 {
            return;
        }
        if let Some(view) = self.outline.rowViewAtRow_makeIfNecessary(row, false)
            && let Some(row_view) = view.downcast_ref::<NdSourceRowView>()
        {
            row_view.show_actions(inside);
        }
    }
}

trait VisibleAll {
    fn visible_all(&self) -> Vec<usize>;
}

impl VisibleAll for SourceIndex {
    /// Every node, parents before children, siblings in list order.
    fn visible_all(&self) -> Vec<usize> {
        fn walk(index: &SourceIndex, i: usize, out: &mut Vec<usize>) {
            out.push(i);
            for &c in &index.children[i] {
                walk(index, c, out);
            }
        }
        let mut out = Vec::new();
        for &r in &self.roots {
            walk(self, r, &mut out);
        }
        out
    }
}
