//! `scaffold()` on AppKit: an `NSSplitViewController` whose sidebar is
//! `NSSplitViewItem(sidebarWithViewController:)` and whose inspector is
//! `NSSplitViewItem(inspectorWithViewController:)` (ARCHITECTURE.md 3.10), so the OS draws the
//! sidebar's material, the full-height layout under the title bar and the collapse animation,
//! and restyles them with each release. Panes are layout boundaries: the split view sizes
//! them, [`NdPaneView`] places the pane's nd content inside its safe area, and every size
//! change asks the window for a layout pass so Taffy lays the pane out at its new size in the
//! same display cycle.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use nd_components::{PaneCollapsed, PaneProps, PaneRole};
use nd_core::Size;
use objc2::rc::Retained;
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSResponder, NSSplitViewController, NSSplitViewItem, NSView, NSViewController,
};
use objc2_foundation::{MainThreadMarker, NSRect, NSSize};

use crate::look::ViewLook;
use crate::views::NdLayerView;

pub(crate) type Hook = Rc<dyn Fn()>;

#[derive(Default)]
pub struct PaneIvars {
    child: RefCell<Option<Retained<NSView>>>,
    last: Cell<NSSize>,
    on_resize: RefCell<Option<Hook>>,
}

define_class!(
    // SAFETY: NSView has no subclassing requirements beyond calling super in the overrides
    // below, and this type does not implement Drop.
    #[unsafe(super(NSView, NSResponder))]
    #[thread_kind = MainThreadOnly]
    #[ivars = PaneIvars]
    pub struct NdPaneView;

    impl NdPaneView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(layout))]
        fn layout(&self) {
            let _: () = unsafe { msg_send![super(self), layout] };
            let safe = self.safe_rect();
            if let Some(child) = self.ivars().child.borrow().as_ref()
                && child.frame() != safe
            {
                child.setFrame(safe);
            }
            if self.ivars().last.replace(safe.size) != safe.size {
                let hook = self.ivars().on_resize.borrow().clone();
                if let Some(hook) = hook {
                    hook();
                }
            }
        }

        #[unsafe(method(safeAreaInsetsDidChange))]
        fn safe_area_changed(&self) {
            self.setNeedsLayout(true);
        }
    }
);

impl NdPaneView {
    fn new(mtm: MainThreadMarker, child: &NSView) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(PaneIvars::default());
        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] };
        this.addSubview(child);
        *this.ivars().child.borrow_mut() = Some(child.retain());
        this
    }

    /// The safe area the pane's content gets: below the toolbar, beside the sidebar.
    pub fn safe_rect(&self) -> NSRect {
        self.safeAreaRect()
    }

    fn set_on_resize(&self, hook: Option<Hook>) {
        *self.ivars().on_resize.borrow_mut() = hook;
    }
}

use objc2::Message;

/// One pane: a view controller whose view is an [`NdPaneView`] holding the node's content.
pub(crate) struct PaneNative {
    pub role: PaneRole,
    _controller: Retained<NSViewController>,
    pub host: Retained<NdPaneView>,
    pub content: Retained<NdLayerView>,
    pub item: Option<Retained<NSSplitViewItem>>,
    pub props: PaneProps,
    /// The collapse state last applied or reported, so neither echoes (3.13 rule 2).
    pub collapsed: Cell<bool>,
}

impl PaneNative {
    pub fn new(mtm: MainThreadMarker, props: &PaneProps) -> Self {
        let content = NdLayerView::new(mtm, &ViewLook::from_style(&props.style, false));
        let host = NdPaneView::new(mtm, &content);
        let controller = NSViewController::new(mtm);
        controller.setView(&host);
        let item = match props.role {
            PaneRole::Sidebar => Some(NSSplitViewItem::sidebarWithViewController(&controller)),
            PaneRole::Inspector => Some(NSSplitViewItem::inspectorWithViewController(&controller)),
            PaneRole::Content => Some(NSSplitViewItem::splitViewItemWithViewController(&controller)),
            PaneRole::Overlay => None,
        };
        let this = Self {
            role: props.role,
            _controller: controller,
            host,
            content,
            item,
            props: props.clone(),
            collapsed: Cell::new(false),
        };
        this.apply(props, true);
        this
    }

    /// The node's view: where the split view (or, for an overlay, the scaffold) holds it.
    pub fn outer(&self) -> &NSView {
        match self.role {
            PaneRole::Overlay => &self.content,
            _ => &self.host,
        }
    }

    pub fn apply(&self, props: &PaneProps, first: bool) {
        self.content.set_props(&ViewLook::from_style(&props.style, false));
        let Some(item) = &self.item else { return };
        if (first || props.min_width != self.props.min_width)
            && let Some(min) = props.min_width
        {
            item.setMinimumThickness(min.into());
        }
        if (first || props.max_width != self.props.max_width)
            && let Some(max) = props.max_width
        {
            item.setMaximumThickness(max.into());
        }
        if (first || props.collapsed != self.props.collapsed) && props.collapsed != self.collapsed.get() {
            self.collapsed.set(props.collapsed);
            item.setCollapsed(props.collapsed);
        }
    }

    pub fn update(&mut self, props: &PaneProps) {
        self.apply(props, false);
        self.props = props.clone();
    }

    /// The size the split view gives the pane's content.
    pub fn boundary(&self) -> Size {
        let r = match self.role {
            PaneRole::Overlay => return Size::default(),
            _ => self.host.safe_rect(),
        };
        Size { width: r.size.width.max(0.) as f32, height: r.size.height.max(0.) as f32 }
    }

    /// Reports a collapse the user made (toolbar button, divider drag). `None` when nothing
    /// changed since the last apply or report.
    pub fn take_collapse(&self) -> Option<PaneCollapsed> {
        let collapsed = self.item.as_ref()?.isCollapsed();
        (self.collapsed.replace(collapsed) != collapsed).then_some(PaneCollapsed { collapsed })
    }
}

/// The scaffold: a host view (the node's view, framed by nd) filled by the split view.
pub(crate) struct ScaffoldNative {
    pub host: Retained<NdLayerView>,
    pub controller: Retained<NSSplitViewController>,
    /// Pane nodes in split order, with their role.
    pub panes: Vec<(nd_core::NodeId, PaneRole)>,
}

fn order(role: PaneRole) -> u8 {
    match role {
        PaneRole::Sidebar => 0,
        PaneRole::Content => 1,
        PaneRole::Inspector => 2,
        PaneRole::Overlay => 3,
    }
}

impl ScaffoldNative {
    pub fn new(mtm: MainThreadMarker) -> Self {
        let host = NdLayerView::new(mtm, &ViewLook::default());
        let controller = NSSplitViewController::new(mtm);
        let split = controller.view();
        split.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        host.addSubview(&split);
        Self { host, controller, panes: Vec::new() }
    }

    /// Adds a pane where its role puts it. Overlays sit above the split view.
    pub fn insert(&mut self, node: nd_core::NodeId, pane: &PaneNative, on_resize: Hook) {
        pane.host.set_on_resize(Some(on_resize));
        self.panes.retain(|(n, _)| *n != node);
        if pane.role == PaneRole::Overlay {
            self.host.addSubview(&pane.content);
            pane.content.setFrame(self.host.bounds());
            pane.content.setAutoresizingMask(
                NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
            );
            self.panes.push((node, pane.role));
            return;
        }
        let Some(item) = &pane.item else { return };
        let index = self.panes.iter().filter(|(_, r)| *r != PaneRole::Overlay && order(*r) < order(pane.role)).count();
        self.controller.insertSplitViewItem_atIndex(item, index as isize);
        self.panes.push((node, pane.role));
        self.panes.sort_by_key(|(_, r)| order(*r));
        self.controller.splitView().adjustSubviews();
    }

    pub fn remove(&mut self, node: nd_core::NodeId, pane: &PaneNative) {
        self.panes.retain(|(n, _)| *n != node);
        pane.host.set_on_resize(None);
        match &pane.item {
            Some(item) => self.controller.removeSplitViewItem(item),
            None => pane.content.removeFromSuperview(),
        }
    }

    pub fn has_sidebar(&self) -> bool {
        self.panes.iter().any(|(_, r)| *r == PaneRole::Sidebar)
    }

    /// Lays the split view out now, so the panes have their sizes before nd measures them.
    pub fn settle(&self) {
        let split = self.controller.view();
        if split.frame() != self.host.bounds() {
            split.setFrame(self.host.bounds());
        }
        split.layoutSubtreeIfNeeded();
    }
}
