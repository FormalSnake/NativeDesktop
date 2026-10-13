//! `source_list()` as AppKit's source list: an `NSOutlineView` in `NSTableViewStyle::SourceList`
//! inside an `NSScrollView`, so selection, group rows, row height and the sidebar material are
//! the system's. The data source answers from the props it was last given (3.13: no app code in
//! a data source); a user selection is reported once, a programmatic one never.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use nd_components::{SourceItem, SourceListProps, SourceSelected};
use nd_ui::events::Emit;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::NSLayoutConstraint;
use objc2_app_kit::{
    NSColor, NSControlTextEditingDelegate, NSImageView, NSOutlineView, NSOutlineViewDataSource, NSOutlineViewDelegate,
    NSScrollView, NSTableCellView, NSTableColumn, NSTableViewDataSource, NSTableViewDelegate, NSTableViewStyle,
    NSTextField, NSView,
};
use objc2_foundation::{
    MainThreadMarker, NSArray, NSIndexSet, NSInteger, NSNotification, NSObject, NSObjectProtocol, NSString,
};

use crate::events::Wake;

/// The outline's model: rows by key, children by parent key (`None` is the root).
#[derive(Default)]
struct Model {
    rows: HashMap<String, Row>,
    children: HashMap<Option<String>, Vec<String>>,
    /// One object per key: the outline tracks items by identity.
    objects: HashMap<String, Retained<NSString>>,
}

struct Row {
    title: String,
    icon: Option<&'static str>,
    badge: Option<String>,
    group: bool,
}

const GROUP: &str = "\u{1}group:";

impl Model {
    fn from_props(props: &SourceListProps) -> Self {
        let mut m = Model::default();
        for (i, section) in props.sections.iter().enumerate() {
            match &section.title {
                Some(title) => {
                    let key = format!("{GROUP}{i}");
                    m.add(None, key.clone(), Row { title: title.to_string(), icon: None, badge: None, group: true });
                    m.items(Some(key), &section.items);
                }
                None => m.items(None, &section.items),
            }
        }
        m
    }

    fn items(&mut self, parent: Option<String>, items: &[SourceItem]) {
        for item in items {
            let key = item.id.to_string();
            let row = Row {
                title: item.title.to_string(),
                icon: item.icon.map(|i| i.lucide()),
                badge: item.badge.as_ref().map(|b| b.to_string()),
                group: false,
            };
            self.add(parent.clone(), key.clone(), row);
            self.items(Some(key), &item.children);
        }
    }

    fn add(&mut self, parent: Option<String>, key: String, row: Row) {
        self.children.entry(parent).or_default().push(key.clone());
        self.objects.insert(key.clone(), NSString::from_str(&key));
        self.rows.insert(key, row);
    }

    fn key(item: Option<&AnyObject>) -> Option<String> {
        item.and_then(|i| i.downcast_ref::<NSString>()).map(|s| s.to_string())
    }

    fn kids(&self, item: Option<&AnyObject>) -> &[String] {
        self.children.get(&Self::key(item)).map_or(&[], Vec::as_slice)
    }
}

pub struct SourceIvars {
    model: RefCell<Model>,
    emit: Emit,
    wake: Wake,
    /// Set while nd changes the selection, so the delegate does not report it back.
    programmatic: Cell<bool>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements and this type does not implement Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = SourceIvars]
    struct NdSourceList;

    unsafe impl NSObjectProtocol for NdSourceList {}

    unsafe impl NSTableViewDataSource for NdSourceList {}
    unsafe impl NSControlTextEditingDelegate for NdSourceList {}
    unsafe impl NSTableViewDelegate for NdSourceList {}

    unsafe impl NSOutlineViewDataSource for NdSourceList {
        #[unsafe(method(outlineView:numberOfChildrenOfItem:))]
        fn number_of_children(&self, _outline: &NSOutlineView, item: Option<&AnyObject>) -> NSInteger {
            self.ivars().model.borrow().kids(item).len() as NSInteger
        }

        #[unsafe(method_id(outlineView:child:ofItem:))]
        fn child(&self, _outline: &NSOutlineView, index: NSInteger, item: Option<&AnyObject>) -> Retained<AnyObject> {
            let model = self.ivars().model.borrow();
            let key = &model.kids(item)[index as usize];
            Retained::into_super(Retained::into_super(model.objects[key].clone()))
        }

        #[unsafe(method(outlineView:isItemExpandable:))]
        fn expandable(&self, _outline: &NSOutlineView, item: &AnyObject) -> bool {
            !self.ivars().model.borrow().kids(Some(item)).is_empty()
        }
    }

    unsafe impl NSOutlineViewDelegate for NdSourceList {
        #[unsafe(method(outlineView:isGroupItem:))]
        fn is_group(&self, _outline: &NSOutlineView, item: &AnyObject) -> bool {
            Model::key(Some(item)).is_some_and(|k| k.starts_with(GROUP))
        }

        #[unsafe(method(outlineView:shouldSelectItem:))]
        fn should_select(&self, _outline: &NSOutlineView, item: &AnyObject) -> bool {
            Model::key(Some(item)).is_some_and(|k| !k.starts_with(GROUP))
        }

        #[unsafe(method_id(outlineView:viewForTableColumn:item:))]
        fn view_for(
            &self,
            outline: &NSOutlineView,
            _column: Option<&NSTableColumn>,
            item: &AnyObject,
        ) -> Option<Retained<NSView>> {
            let model = self.ivars().model.borrow();
            let row = Model::key(Some(item)).and_then(|k| model.rows.get(&k));
            row.map(|row| cell(MainThreadMarker::from(outline), row))
        }

        #[unsafe(method(outlineViewSelectionDidChange:))]
        fn selection_changed(&self, notification: &NSNotification) {
            if self.ivars().programmatic.get() {
                return;
            }
            let Some(outline) = notification.object().and_then(|o| o.downcast::<NSOutlineView>().ok()) else { return };
            let Some(key) = outline.itemAtRow(outline.selectedRow()).and_then(|i| Model::key(Some(&i))) else { return };
            self.ivars().emit.emit(SourceSelected { id: key.into() });
            self.ivars().wake.wake();
        }
    }
);

/// A source list cell: the system's text field and image view in an `NSTableCellView`, the
/// badge a label at the trailing edge. Spacing is AppKit's system spacing and sizes
/// are the controls' own, so the row matches Finder's and Mail's.
fn cell(mtm: MainThreadMarker, row: &Row) -> Retained<NSView> {
    let cell = NSTableCellView::new(mtm);
    let text = NSTextField::labelWithString(&NSString::from_str(&row.title), mtm);
    text.setLineBreakMode(objc2_app_kit::NSLineBreakMode::ByTruncatingTail);
    text.setTranslatesAutoresizingMaskIntoConstraints(false);
    cell.addSubview(&text);
    unsafe { cell.setTextField(Some(&text)) };
    let mut constraints = vec![text.centerYAnchor().constraintEqualToAnchor(&cell.centerYAnchor())];
    // A group row is the title alone; the outline styles it as a section header.
    match row.icon.filter(|_| !row.group).and_then(crate::icons::symbol) {
        Some(image) => {
            let view = NSImageView::imageViewWithImage(&image, mtm);
            view.setTranslatesAutoresizingMaskIntoConstraints(false);
            cell.addSubview(&view);
            unsafe { cell.setImageView(Some(&view)) };
            constraints.push(view.leadingAnchor().constraintEqualToAnchor(&cell.leadingAnchor()));
            constraints.push(view.centerYAnchor().constraintEqualToAnchor(&cell.centerYAnchor()));
            constraints.push(
                text.leadingAnchor().constraintEqualToSystemSpacingAfterAnchor_multiplier(&view.trailingAnchor(), 1.),
            );
        }
        None => constraints.push(text.leadingAnchor().constraintEqualToAnchor(&cell.leadingAnchor())),
    }
    match row.badge.as_ref().filter(|_| !row.group) {
        Some(badge) => {
            let label = NSTextField::labelWithString(&NSString::from_str(badge), mtm);
            // secondaryLabelColor is 3.95:1 on a light sidebar, under WCAG AA for text this size.
            label.setTextColor(Some(&NSColor::labelColor()));
            label.setTranslatesAutoresizingMaskIntoConstraints(false);
            cell.addSubview(&label);
            constraints.push(label.trailingAnchor().constraintEqualToAnchor(&cell.trailingAnchor()));
            constraints.push(label.centerYAnchor().constraintEqualToAnchor(&cell.centerYAnchor()));
            constraints.push(
                label
                    .leadingAnchor()
                    .constraintGreaterThanOrEqualToSystemSpacingAfterAnchor_multiplier(&text.trailingAnchor(), 1.),
            );
        }
        None => constraints.push(text.trailingAnchor().constraintLessThanOrEqualToAnchor(&cell.trailingAnchor())),
    }
    NSLayoutConstraint::activateConstraints(&NSArray::from_retained_slice(&constraints));
    Retained::into_super(cell)
}

pub(crate) struct SourceListNative {
    pub scroll: Retained<NSScrollView>,
    outline: Retained<NSOutlineView>,
    source: Retained<NdSourceList>,
    props: SourceListProps,
}

impl SourceListNative {
    pub fn new(mtm: MainThreadMarker, emit: Emit, wake: Wake, props: &SourceListProps) -> Self {
        let outline = NSOutlineView::new(mtm);
        outline.setStyle(NSTableViewStyle::SourceList);
        outline.setHeaderView(None);
        let column = NSTableColumn::initWithIdentifier(NSTableColumn::alloc(mtm), &NSString::from_str("nd.source"));
        outline.addTableColumn(&column);
        unsafe { outline.setOutlineTableColumn(Some(&column)) };
        outline.setFloatsGroupRows(false);
        let source = NdSourceList::alloc(mtm).set_ivars(SourceIvars {
            model: RefCell::new(Model::from_props(props)),
            emit,
            wake,
            programmatic: Cell::new(false),
        });
        let source: Retained<NdSourceList> = unsafe { msg_send![super(source), init] };
        unsafe {
            outline.setDataSource(Some(ProtocolObject::from_ref(&*source)));
            outline.setDelegate(Some(ProtocolObject::from_ref(&*source)));
        }
        let scroll = NSScrollView::new(mtm);
        scroll.setDocumentView(Some(&outline));
        scroll.setHasVerticalScroller(true);
        scroll.setDrawsBackground(false);
        let this = Self { scroll, outline, source, props: props.clone() };
        this.reload();
        this
    }

    pub fn view(&self) -> &NSView {
        &self.scroll
    }

    pub fn update(&mut self, props: &SourceListProps) {
        if props.sections != self.props.sections {
            *self.source.ivars().model.borrow_mut() = Model::from_props(props);
            self.props = props.clone();
            self.reload();
        } else if props.selected != self.props.selected {
            self.props.selected = props.selected.clone();
            self.select();
        }
    }

    fn reload(&self) {
        self.source.ivars().programmatic.set(true);
        self.outline.reloadData();
        unsafe { self.outline.expandItem_expandChildren(None, true) };
        self.source.ivars().programmatic.set(false);
        self.select();
    }

    fn select(&self) {
        let ivars = self.source.ivars();
        let current = self.outline.itemAtRow(self.outline.selectedRow()).and_then(|i| Model::key(Some(&i)));
        let wanted = self.props.selected.as_ref().map(|s| s.to_string());
        if current == wanted {
            return;
        }
        ivars.programmatic.set(true);
        let object = wanted.as_ref().and_then(|k| ivars.model.borrow().objects.get(k).cloned());
        match object {
            Some(object) => {
                let row = unsafe { self.outline.rowForItem(Some(&object)) };
                if row >= 0 {
                    self.outline
                        .selectRowIndexes_byExtendingSelection(&NSIndexSet::indexSetWithIndex(row as usize), false);
                }
            }
            None => unsafe { self.outline.deselectAll(None) },
        }
        ivars.programmatic.set(false);
    }
}
