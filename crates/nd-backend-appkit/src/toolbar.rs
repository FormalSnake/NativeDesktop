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
use objc2_app_kit::NSUserInterfaceItemIdentification;
use objc2_app_kit::{
    NSResponder, NSSplitView, NSToolbar, NSToolbarDelegate, NSToolbarDisplayMode, NSToolbarFlexibleSpaceItemIdentifier,
    NSToolbarInspectorTrackingSeparatorItemIdentifier, NSToolbarItem, NSToolbarItemIdentifier,
    NSToolbarSidebarTrackingSeparatorItemIdentifier, NSToolbarSpaceItemIdentifier,
    NSToolbarToggleInspectorItemIdentifier, NSToolbarToggleSidebarItemIdentifier, NSTrackingSeparatorToolbarItem,
    NSView, NSWindow, NSWindowToolbarStyle,
};
use objc2_foundation::{
    MainThreadMarker, NSArray, NSCopying, NSObject, NSObjectProtocol, NSRect, NSSet, NSSize, NSString,
};

#[derive(Default)]
pub struct ToolbarIvars {
    items: RefCell<Vec<Retained<NSToolbarItem>>>,
    identifiers: RefCell<Vec<Retained<NSString>>>,
    selectable: RefCell<Vec<Retained<NSString>>>,
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

        #[unsafe(method_id(toolbarSelectableItemIdentifiers:))]
        fn selectable_identifiers(&self, _toolbar: &NSToolbar) -> Retained<NSArray<NSToolbarItemIdentifier>> {
            NSArray::from_retained_slice(&self.ivars().selectable.borrow())
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

    fn set_selectable(&self, identifiers: Vec<Retained<NSString>>) {
        *self.ivars().selectable.borrow_mut() = identifiers;
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
    constraints: RefCell<Vec<Retained<objc2_app_kit::NSLayoutConstraint>>>,
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

        // The toolbar puts the item in the window after nd sized it: grow the titlebar then.
        #[unsafe(method(viewDidMoveToWindow))]
        fn did_move_to_window(&self) {
            let _: () = unsafe { msg_send![super(self), viewDidMoveToWindow] };
            if let Some(window) = self.window() {
                titlebar_room(&window, self.ivars().size.get().height);
            }
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
            // Constraints, not only the intrinsic size: a toolbar grows to an item taller than
            // its standard row when the item's view asks for it with Auto Layout.
            self.setTranslatesAutoresizingMaskIntoConstraints(false);
            for old in self.ivars().constraints.borrow_mut().drain(..) {
                old.setActive(false);
            }
            let w = self.widthAnchor().constraintEqualToConstant(size.width);
            let h = self.heightAnchor().constraintEqualToConstant(size.height);
            w.setActive(true);
            h.setActive(true);
            self.ivars().constraints.borrow_mut().extend([w, h]);
            if let Some(window) = self.window() {
                titlebar_room(&window, size.height);
            }
        }
    }
}

/// The tallest item the unified toolbar's row holds; an item taller than this (a conversation's
/// avatar over its name, as Messages shows it) grows the titlebar by the rest.
const TOOLBAR_ROW: f64 = 38.;

/// Grows the titlebar so an item `height` tall fits: an empty bottom titlebar accessory as tall as
/// the overflow, the system's own way to make the band taller. One per window, resized in place;
/// it only ever grows, so items of different heights do not make the band jump.
pub fn titlebar_room(window: &objc2_app_kit::NSWindow, height: f64) {
    let extra = (height - TOOLBAR_ROW).max(0.).ceil();
    let id = NSString::from_str("nd.toolbar.room");
    let existing =
        window.titlebarAccessoryViewControllers().iter().find(|c| c.view().identifier().is_some_and(|i| *i == *id));
    match existing {
        Some(controller) => {
            let view = controller.view();
            if view.frame().size.height < extra {
                set_room_height(&view, extra);
            }
        }
        None if extra > 0. => {
            let Some(mtm) = MainThreadMarker::new() else { return };
            let controller = objc2_app_kit::NSTitlebarAccessoryViewController::new(mtm);
            let view = objc2_app_kit::NSView::initWithFrame(
                objc2_app_kit::NSView::alloc(mtm),
                NSRect::new(objc2_foundation::NSPoint::ZERO, NSSize::new(1., extra)),
            );
            view.setIdentifier(Some(&id));
            set_room_height(&view, extra);
            controller.setView(&view);
            controller.setLayoutAttribute(objc2_app_kit::NSLayoutAttribute::Bottom);
            // Its own height, not a standard accessory bar's.
            controller.setAutomaticallyAdjustsSize(false);
            window.addTitlebarAccessoryViewController(&controller);
        }
        None => {}
    }
}

/// The accessory's height as a constraint: AppKit sizes titlebar accessories with Auto Layout. It
/// still gives a bottom accessory at least a standard bar's height (36 pt).
fn set_room_height(view: &objc2_app_kit::NSView, height: f64) {
    for c in view.constraints().iter() {
        c.setActive(false);
    }
    view.setFrameSize(NSSize::new(view.frame().size.width, height));
    view.setTranslatesAutoresizingMaskIntoConstraints(false);
    view.heightAnchor().constraintEqualToConstant(height).setActive(true);
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
        // A centred item is the window's subject (a conversation), not a control: no bezel or
        // glass around it.
        self.item.setBordered(!props.centered);
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

/// A system tracking separator (`id` names the sidebar's or the inspector's), bound to
/// `split`'s divider at `divider`.
fn tracking_separator(
    id: &NSToolbarItemIdentifier,
    split: &NSSplitView,
    divider: isize,
    items: &mut Vec<Retained<NSToolbarItem>>,
    ids: &mut Vec<Retained<NSString>>,
) {
    let separator = NSTrackingSeparatorToolbarItem::trackingSeparatorToolbarItemWithIdentifier_splitView_dividerIndex(
        id, split, divider,
    );
    items.push(Retained::into_super(separator));
    ids.push(id.copy());
}

/// An entry of the toolbar as the backend rebuilds it.
pub(crate) enum Entry {
    /// An app item, whether it leads, whether it is centred.
    Item(Retained<NSToolbarItem>, bool, bool),
    System(SystemItem),
    Tab(Retained<NSToolbarItem>),
}

pub struct TabIvars {
    id: nd_ui::prelude::SharedString,
    emit: RefCell<Option<(nd_ui::events::Emit, crate::events::Wake)>>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements and this type does not implement Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = TabIvars]
    pub struct NdTabTarget;

    unsafe impl NSObjectProtocol for NdTabTarget {}

    impl NdTabTarget {
        #[unsafe(method(pick:))]
        fn pick(&self, _sender: &NSObject) {
            if let Some((emit, wake)) = self.ivars().emit.borrow().as_ref() {
                emit.emit(nd_components::TabSelected { id: self.ivars().id.clone() });
                wake.wake();
            }
        }
    }
);

/// A settings-style tab: the toolbar's own selectable image item.
pub(crate) struct ToolbarTabNative {
    pub item: Retained<NSToolbarItem>,
    target: Retained<NdTabTarget>,
}

impl ToolbarTabNative {
    pub fn new(mtm: MainThreadMarker, props: &nd_components::ToolbarTabProps) -> Self {
        let id = NSString::from_str(&format!("nd.tab.{}", props.id));
        let item = NSToolbarItem::initWithItemIdentifier(NSToolbarItem::alloc(mtm), &id);
        let target = NdTabTarget::alloc(mtm).set_ivars(TabIvars { id: props.id.clone(), emit: RefCell::new(None) });
        let target: Retained<NdTabTarget> = unsafe { msg_send![super(target), init] };
        unsafe {
            item.setTarget(Some(&target));
            item.setAction(Some(objc2::sel!(pick:)));
        }
        let this = Self { item, target };
        this.apply(props);
        this
    }

    pub fn apply(&self, props: &nd_components::ToolbarTabProps) {
        self.item.setLabel(&NSString::from_str(&props.label));
        self.item.setImage(props.icon.and_then(|i| crate::icons::symbol(i.lucide())).as_deref());
    }

    /// Clicks report [`nd_components::TabSelected`] on the toolbar node.
    pub fn report_to(&self, emit: nd_ui::events::Emit, wake: crate::events::Wake) {
        *self.target.ivars().emit.borrow_mut() = Some((emit, wake));
    }
}

/// The `toolbar()` intent: the window's `NSToolbar` and its delegate.
pub(crate) struct ToolbarNative {
    pub toolbar: Retained<NSToolbar>,
    delegate: Retained<NdToolbarDelegate>,
    pub props: ToolbarProps,
    pub children: Vec<nd_core::NodeId>,
    pub dirty: Cell<bool>,
    /// The window it is attached to, so a later title change reaches it.
    window: std::cell::RefCell<Option<objc2::rc::Weak<NSWindow>>>,
}

impl ToolbarNative {
    pub fn new(mtm: MainThreadMarker, props: &ToolbarProps, identifier: &str) -> Self {
        let delegate = NdToolbarDelegate::new(mtm);
        let toolbar = NSToolbar::initWithIdentifier(NSToolbar::alloc(mtm), &NSString::from_str(identifier));
        toolbar.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        toolbar.setAllowsUserCustomization(false);
        let this = Self {
            toolbar,
            delegate,
            props: props.clone(),
            children: Vec::new(),
            dirty: Cell::new(true),
            window: std::cell::RefCell::new(None),
        };
        this.apply_display();
        this
    }

    fn apply_display(&self) {
        // A settings window's tabs show their labels, as System Settings' do.
        let tabs = self.props.selected_tab.is_some();
        self.toolbar.setDisplayMode(match self.props.display {
            nd_components::ToolbarDisplay::Automatic if tabs => NSToolbarDisplayMode::IconAndLabel,
            // The unified toolbar shows icons; labels are for the overflow menu.
            nd_components::ToolbarDisplay::Automatic | nd_components::ToolbarDisplay::IconOnly => {
                NSToolbarDisplayMode::IconOnly
            }
            nd_components::ToolbarDisplay::IconAndLabel => NSToolbarDisplayMode::IconAndLabel,
        });
    }

    pub fn update(&mut self, props: &ToolbarProps) {
        let retitled = self.props.title != props.title;
        self.props = props.clone();
        self.apply_display();
        if retitled
            && let Some(title) = &self.props.title
            && let Some(window) = self.window.borrow().as_ref().and_then(|w| w.load())
        {
            window.setTitle(&NSString::from_str(title));
        }
        if let Some(window) = self.window.borrow().as_ref().and_then(|w| w.load()) {
            self.apply_title_visibility(&window);
        }
    }

    fn apply_title_visibility(&self, window: &NSWindow) {
        window.setTitleVisibility(if self.props.hide_title {
            objc2_app_kit::NSWindowTitleVisibility::Hidden
        } else {
            objc2_app_kit::NSWindowTitleVisibility::Visible
        });
    }

    pub fn attach_to(&self, window: &NSWindow) {
        // A settings window keeps its preference style.
        if window.toolbarStyle() != NSWindowToolbarStyle::Preference {
            window.setToolbarStyle(NSWindowToolbarStyle::Unified);
        }
        window.setToolbar(Some(&self.toolbar));
        self.apply_title_visibility(window);
        *self.window.borrow_mut() = Some(objc2::rc::Weak::new(window));
        if let Some(title) = &self.props.title {
            window.setTitle(&NSString::from_str(title));
        }
    }

    /// Re-lists the toolbar's items: `entries` in order, the sidebar tracking separator after
    /// the leading ones when `split` has a sidebar, and the inspector tracking separator ahead
    /// of the inspector toggle and what follows it (at the end without one) when the window
    /// has an inspector, whose leading divider `inspector` names.
    pub fn rebuild(&self, entries: Vec<Entry>, split: Option<&NSSplitView>, inspector: Option<(&NSSplitView, isize)>) {
        self.dirty.set(false);
        let mut items = Vec::new();
        let mut identifiers: Vec<Retained<NSString>> = Vec::new();
        let mut tabs: Vec<Retained<NSString>> = Vec::new();
        let mut centered: Vec<Retained<NSString>> = Vec::new();
        // The sidebar's own section: leading items and the sidebar toggle, as in Apple's apps.
        let leading = entries
            .iter()
            .take_while(|e| matches!(e, Entry::Item(_, true, _) | Entry::System(SystemItem::ToggleSidebar)))
            .count();
        let explicit = entries.iter().any(|e| matches!(e, Entry::System(SystemItem::InspectorTrackingSeparator)));
        let inspector = inspector.filter(|_| !explicit);
        let trailing = entries
            .iter()
            .position(|e| matches!(e, Entry::System(SystemItem::ToggleInspector)))
            .unwrap_or(entries.len())
            .max(leading);
        let sidebar_id = unsafe { NSToolbarSidebarTrackingSeparatorItemIdentifier };
        let inspector_id = unsafe { NSToolbarInspectorTrackingSeparatorItemIdentifier };
        for (i, entry) in entries.into_iter().enumerate() {
            if i == leading
                && let Some(split) = split
            {
                tracking_separator(sidebar_id, split, 0, &mut items, &mut identifiers);
            }
            if i == trailing
                && let Some((split, divider)) = inspector
            {
                tracking_separator(inspector_id, split, divider, &mut items, &mut identifiers);
                // The inspector toggle sits at the window's trailing edge, as in Apple's apps.
                if matches!(entry, Entry::System(SystemItem::ToggleInspector)) {
                    identifiers.push(system_identifier(SystemItem::FlexibleSpace).copy());
                }
            }
            match entry {
                Entry::Item(item, _, center) => {
                    if center {
                        centered.push(item.itemIdentifier());
                    }
                    identifiers.push(item.itemIdentifier());
                    items.push(item);
                }
                Entry::System(system) => identifiers.push(system_identifier(system).copy()),
                Entry::Tab(item) => {
                    tabs.push(item.itemIdentifier());
                    identifiers.push(item.itemIdentifier());
                    items.push(item);
                }
            }
        }
        if leading == identifiers.len()
            && let Some(split) = split
        {
            tracking_separator(sidebar_id, split, 0, &mut items, &mut identifiers);
        }
        if let Some((split, divider)) = inspector
            && !identifiers.iter().any(|i| &**i == inspector_id)
        {
            tracking_separator(inspector_id, split, divider, &mut items, &mut identifiers);
        }
        let current: Vec<Retained<NSString>> = self.toolbar.items().iter().map(|i| i.itemIdentifier()).collect();
        self.delegate.set(items, identifiers.clone());
        self.delegate.set_selectable(tabs);
        if current != identifiers {
            while self.toolbar.items().count() > 0 {
                self.toolbar.removeItemAtIndex(0);
            }
            for (i, id) in identifiers.iter().enumerate() {
                self.toolbar.insertItemWithItemIdentifier_atIndex(id, i as isize);
            }
        }
        let centered = NSSet::from_retained_slice(&centered);
        self.toolbar.setCenteredItemIdentifiers(&centered);
        self.select_tab();
    }

    /// Shows the app's selected tab as the toolbar's selection.
    pub fn select_tab(&self) {
        let id = self.props.selected_tab.as_ref().map(|t| NSString::from_str(&format!("nd.tab.{t}")));
        self.toolbar.setSelectedItemIdentifier(id.as_deref());
    }
}
