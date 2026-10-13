//! `NSToolbar`: the delegate that serves nd's items, [`ToolbarHost`] for a fixed item set, and
//! the `toolbar()` intent's natives. Every structural item is the system's own
//! (`NSToolbarFlexibleSpaceItemIdentifier`, `toggleSidebar`, the tracking separators); an app
//! item is an `NSToolbarItem` showing the item's native view, or, for an icon-only button, the
//! toolbar's own image item wired to that button's action.

use std::cell::{Cell, RefCell};

use nd_components::{SystemItem, ToolbarItemProps, ToolbarProps};
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSResponder, NSSplitView, NSToolbar, NSToolbarDelegate, NSToolbarDisplayMode, NSToolbarFlexibleSpaceItemIdentifier,
    NSToolbarInspectorTrackingSeparatorItemIdentifier, NSToolbarItem, NSToolbarItemIdentifier,
    NSToolbarSidebarTrackingSeparatorItemIdentifier, NSToolbarSpaceItemIdentifier,
    NSToolbarToggleInspectorItemIdentifier, NSToolbarToggleSidebarItemIdentifier, NSTrackingSeparatorToolbarItem,
    NSView, NSWindow, NSWindowToolbarStyle,
};
use objc2_foundation::{MainThreadMarker, NSArray, NSCopying, NSObject, NSObjectProtocol, NSRect, NSSize, NSString};

#[derive(Default)]
pub struct ToolbarIvars {
    items: RefCell<Vec<Retained<NSToolbarItem>>>,
    identifiers: RefCell<Vec<Retained<NSString>>>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements and this type does not implement Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = ToolbarIvars]
    struct NdToolbarDelegate;

    unsafe impl NSObjectProtocol for NdToolbarDelegate {}

    unsafe impl NSToolbarDelegate for NdToolbarDelegate {
        #[unsafe(method_id(toolbar:itemForItemIdentifier:willBeInsertedIntoToolbar:))]
        fn item_for_identifier(
            &self,
            _toolbar: &NSToolbar,
            identifier: &NSToolbarItemIdentifier,
            _will_insert: bool,
        ) -> Option<Retained<NSToolbarItem>> {
            self.ivars().items.borrow().iter().find(|item| &*item.itemIdentifier() == identifier).cloned()
        }

        #[unsafe(method_id(toolbarDefaultItemIdentifiers:))]
        fn default_identifiers(&self, _toolbar: &NSToolbar) -> Retained<NSArray<NSToolbarItemIdentifier>> {
            NSArray::from_retained_slice(&self.ivars().identifiers.borrow())
        }

        #[unsafe(method_id(toolbarAllowedItemIdentifiers:))]
        fn allowed_identifiers(&self, _toolbar: &NSToolbar) -> Retained<NSArray<NSToolbarItemIdentifier>> {
            NSArray::from_retained_slice(&self.ivars().identifiers.borrow())
        }
    }
);

impl NdToolbarDelegate {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        unsafe { msg_send![super(Self::alloc(mtm).set_ivars(ToolbarIvars::default())), init] }
    }

    /// The custom items it serves and the identifiers, system ones included, in order.
    fn set(&self, items: Vec<Retained<NSToolbarItem>>, identifiers: Vec<Retained<NSString>>) {
        *self.ivars().items.borrow_mut() = items;
        *self.ivars().identifiers.borrow_mut() = identifiers;
    }
}

/// An `NSToolbar` serving a fixed set of items. Keep it alive as long as the window: the
/// toolbar holds its delegate weakly.
pub struct ToolbarHost {
    pub toolbar: Retained<NSToolbar>,
    _delegate: Retained<NdToolbarDelegate>,
}

impl ToolbarHost {
    pub fn attach(
        mtm: MainThreadMarker,
        window: &NSWindow,
        identifier: &str,
        items: Vec<Retained<NSToolbarItem>>,
    ) -> Self {
        let delegate = NdToolbarDelegate::new(mtm);
        let ids = items.iter().map(|i| i.itemIdentifier()).collect();
        delegate.set(items, ids);
        let toolbar = NSToolbar::initWithIdentifier(NSToolbar::alloc(mtm), &NSString::from_str(identifier));
        toolbar.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        window.setToolbar(Some(&toolbar));
        Self { toolbar, _delegate: delegate }
    }
}

#[derive(Default)]
pub struct ItemHostIvars {
    size: Cell<NSSize>,
}

define_class!(
    // SAFETY: NSView has no subclassing requirements beyond calling super, and this type does
    // not implement Drop.
    #[unsafe(super(NSView, NSResponder))]
    #[thread_kind = MainThreadOnly]
    #[ivars = ItemHostIvars]
    pub struct NdItemHostView;

    impl NdItemHostView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        // NSToolbar sizes a custom view item from this: the size nd laid the item out at.
        #[unsafe(method(intrinsicContentSize))]
        fn intrinsic_content_size(&self) -> NSSize {
            self.ivars().size.get()
        }
    }
);

impl NdItemHostView {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ItemHostIvars::default());
        unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] }
    }

    pub fn set_size(&self, size: NSSize) {
        if self.ivars().size.replace(size) != size {
            self.invalidateIntrinsicContentSize();
            self.setFrameSize(size);
        }
    }
}

/// One app item: the toolbar item, the view it shows, and what the backend learned about the
/// child it wraps.
pub(crate) struct ToolbarItemNative {
    pub item: Retained<NSToolbarItem>,
    pub host: Retained<NdItemHostView>,
    pub props: ToolbarItemProps,
    /// The child is a window controls slot: the toolbar already shows the traffic lights
    /// there, so the item is left out.
    pub skip: Cell<bool>,
}

impl ToolbarItemNative {
    pub fn new(mtm: MainThreadMarker, identifier: &str, props: &ToolbarItemProps) -> Self {
        let item = NSToolbarItem::initWithItemIdentifier(NSToolbarItem::alloc(mtm), &NSString::from_str(identifier));
        let host = NdItemHostView::new(mtm);
        item.setView(Some(&host));
        let this = Self { item, host, props: props.clone(), skip: Cell::new(false) };
        this.apply(props);
        this
    }

    pub fn apply(&self, props: &ToolbarItemProps) {
        if let Some(label) = &props.label {
            self.item.setLabel(&NSString::from_str(label));
        }
        self.item.setToolTip(props.tooltip.as_deref().map(NSString::from_str).as_deref());
    }

    /// Shows the child's own view.
    pub fn show_view(&self, view: &NSView) {
        self.host.addSubview(view);
        self.item.setView(Some(&self.host));
    }
}

fn system_identifier(item: SystemItem) -> &'static NSString {
    unsafe {
        match item {
            SystemItem::FlexibleSpace => NSToolbarFlexibleSpaceItemIdentifier,
            SystemItem::Space => NSToolbarSpaceItemIdentifier,
            SystemItem::ToggleSidebar => NSToolbarToggleSidebarItemIdentifier,
            SystemItem::ToggleInspector => NSToolbarToggleInspectorItemIdentifier,
            SystemItem::SidebarTrackingSeparator => NSToolbarSidebarTrackingSeparatorItemIdentifier,
            SystemItem::InspectorTrackingSeparator => NSToolbarInspectorTrackingSeparatorItemIdentifier,
        }
    }
}

/// The system's sidebar tracking separator, bound to `split`'s first divider.
fn tracking_separator(
    split: &NSSplitView,
    items: &mut Vec<Retained<NSToolbarItem>>,
    ids: &mut Vec<Retained<NSString>>,
) {
    let id = unsafe { NSToolbarSidebarTrackingSeparatorItemIdentifier };
    let separator =
        NSTrackingSeparatorToolbarItem::trackingSeparatorToolbarItemWithIdentifier_splitView_dividerIndex(id, split, 0);
    items.push(Retained::into_super(separator));
    ids.push(id.copy());
}

/// An entry of the toolbar as the backend rebuilds it.
pub(crate) enum Entry {
    Item(Retained<NSToolbarItem>, bool),
    System(SystemItem),
}

/// The `toolbar()` intent: the window's `NSToolbar` and its delegate.
pub(crate) struct ToolbarNative {
    pub toolbar: Retained<NSToolbar>,
    delegate: Retained<NdToolbarDelegate>,
    pub props: ToolbarProps,
    pub children: Vec<nd_core::NodeId>,
    pub dirty: Cell<bool>,
}

impl ToolbarNative {
    pub fn new(mtm: MainThreadMarker, props: &ToolbarProps, identifier: &str) -> Self {
        let delegate = NdToolbarDelegate::new(mtm);
        let toolbar = NSToolbar::initWithIdentifier(NSToolbar::alloc(mtm), &NSString::from_str(identifier));
        toolbar.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        toolbar.setAllowsUserCustomization(false);
        let this = Self { toolbar, delegate, props: props.clone(), children: Vec::new(), dirty: Cell::new(true) };
        this.apply_display();
        this
    }

    fn apply_display(&self) {
        self.toolbar.setDisplayMode(match self.props.display {
            // The unified toolbar shows icons; labels are for the overflow menu.
            nd_components::ToolbarDisplay::Automatic | nd_components::ToolbarDisplay::IconOnly => {
                NSToolbarDisplayMode::IconOnly
            }
            nd_components::ToolbarDisplay::IconAndLabel => NSToolbarDisplayMode::IconAndLabel,
        });
    }

    pub fn update(&mut self, props: &ToolbarProps) {
        self.props = props.clone();
        self.apply_display();
    }

    pub fn attach_to(&self, window: &NSWindow) {
        window.setToolbarStyle(NSWindowToolbarStyle::Unified);
        window.setToolbar(Some(&self.toolbar));
        if let Some(title) = &self.props.title {
            window.setTitle(&NSString::from_str(title));
        }
    }

    /// Re-lists the toolbar's items: `entries` in order, the sidebar tracking separator after
    /// the leading ones when `split` has a sidebar.
    pub fn rebuild(&self, entries: Vec<Entry>, split: Option<&NSSplitView>) {
        self.dirty.set(false);
        let mut items = Vec::new();
        let mut identifiers: Vec<Retained<NSString>> = Vec::new();
        // The sidebar's own section: leading items and the sidebar toggle, as in Apple's apps.
        let leading = entries
            .iter()
            .take_while(|e| matches!(e, Entry::Item(_, true) | Entry::System(SystemItem::ToggleSidebar)))
            .count();
        for (i, entry) in entries.into_iter().enumerate() {
            if i == leading
                && let Some(split) = split
            {
                tracking_separator(split, &mut items, &mut identifiers);
            }
            match entry {
                Entry::Item(item, _) => {
                    identifiers.push(item.itemIdentifier());
                    items.push(item);
                }
                Entry::System(system) => identifiers.push(system_identifier(system).copy()),
            }
        }
        if leading == identifiers.len()
            && let Some(split) = split
        {
            tracking_separator(split, &mut items, &mut identifiers);
        }
        let current: Vec<Retained<NSString>> = self.toolbar.items().iter().map(|i| i.itemIdentifier()).collect();
        self.delegate.set(items, identifiers.clone());
        if current == identifiers {
            return;
        }
        while self.toolbar.items().count() > 0 {
            self.toolbar.removeItemAtIndex(0);
        }
        for (i, id) in identifiers.iter().enumerate() {
            self.toolbar.insertItemWithItemIdentifier_atIndex(id, i as isize);
        }
    }
}
