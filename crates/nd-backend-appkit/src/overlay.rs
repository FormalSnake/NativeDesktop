//! Overlays on AppKit (ARCHITECTURE.md 6.3). Every overlay is its own window above the owner, so
//! the window server, not hit-testing inside one window, decides where input goes: a press,
//! scroll or hover over an overlay can never reach a WKWebView, CEF or GPU surface view under it.
//! In-window views would not be enough: tracking areas (hover, cursor) fire for covered views
//! too.
//!
//! - Popovers are `NSPopover`s anchored to their parent's view: the system's own arrow, material
//!   and placement, which keep the owner's key chrome.
//! - Panels and tooltips are borderless child `NSPanel`s that never become key, so the owner keeps
//!   its key chrome and keys keep reaching nd dispatch (Esc included). Tooltips ignore the mouse.
//!   A panel fades in when it opens and out when it closes, unless Reduce Motion is on.
//! - One local event monitor serves every open overlay: a press outside the innermost overlays
//!   closes each light-dismiss one above the pressed window and is consumed when it landed in an
//!   nd window; Esc pressed while an overlay window is key closes the innermost one. Presses in
//!   windows nd does not own (the menu bar, a system panel) still close light-dismiss overlays
//!   but are not consumed.
//!
//! The overlay's content is an `NdLayerView` laid out by nd (Taffy), sized by the overlay node's
//! frame; the backend shows or moves it in the transaction's commit, after frames are set, so it
//! opens over its laid-out content.

use std::cell::{Cell, RefCell};

use block2::RcBlock;
use nd_core::{NodeId, Rect};
use nd_ui::events::Emit;
use nd_ui::{DismissCause, Dismissed, Modality, OverlayKind, OverlayProps, Placement};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{DefinedClass, MainThreadOnly, Message, define_class, msg_send};
use objc2_app_kit::{
    NSAnimatablePropertyContainer, NSAnimationContext, NSAppearanceCustomization, NSBackingStoreType, NSColor, NSEvent,
    NSEventMask, NSEventModifierFlags, NSEventType, NSPanel, NSPopover, NSPopoverBehavior, NSResponder, NSView,
    NSViewController, NSWindow, NSWindowOrderingMode, NSWindowStyleMask, NSWorkspace,
};
use objc2_foundation::{MainThreadMarker, NSPoint, NSRect, NSRectEdge, NSSize};

use crate::NdWindow;
use crate::events::Wake;
use crate::look::ViewLook;
use crate::views::NdLayerView;

/// The gap between an anchor and a tooltip.
const TOOLTIP_GAP: f64 = 6.; // nd-lint: allow, AppKit has no tooltip offset to read
/// A panel's top edge sits this far down the owner's content, as a fraction of its height.
const PANEL_TOP: f64 = 0.18;
/// How long a panel takes to fade in or out, in seconds.
const PANEL_FADE: f64 = 0.18;

fn fades() -> bool {
    !NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion()
}

/// Animates `panel`'s alpha to `to`, then runs `done`.
fn fade(panel: &NSPanel, to: f64, done: impl Fn() + 'static) {
    let fading = panel.retain();
    let changes = RcBlock::new(move |context: std::ptr::NonNull<NSAnimationContext>| {
        unsafe { context.as_ref() }.setDuration(PANEL_FADE);
        fading.animator().setAlphaValue(to);
    });
    let done = RcBlock::new(done);
    NSAnimationContext::runAnimationGroup_completionHandler(&changes, Some(&done));
}

enum Presenter {
    Popover(Retained<NSPopover>),
    Panel(Retained<NSPanel>),
}

pub(crate) struct OverlayNative {
    pub content: Retained<NdLayerView>,
    node: NodeId,
    props: OverlayProps,
    presenter: Presenter,
    anchor: RefCell<Option<Retained<NSView>>>,
    owner: RefCell<Option<Retained<NdWindow>>>,
    size: Cell<NSSize>,
    /// Shown, or waiting for the commit that shows it.
    attached: Cell<bool>,
    dirty: Cell<bool>,
    emit: Emit,
    wake: Wake,
}

impl OverlayNative {
    pub fn new(mtm: MainThreadMarker, node: NodeId, props: &OverlayProps, emit: Emit, wake: Wake) -> Self {
        let content = NdLayerView::new(mtm, &ViewLook::from_style(&props.style, false));
        let presenter = match props.kind {
            OverlayKind::Popover => {
                let popover = NSPopover::new(mtm);
                // nd decides when it closes (the monitor below), so outside presses are consumed
                // the same way on every overlay instead of AppKit's transient pass-through.
                popover.setBehavior(NSPopoverBehavior::ApplicationDefined);
                let controller = NSViewController::new(mtm);
                controller.setView(&content);
                popover.setContentViewController(Some(&controller));
                Presenter::Popover(popover)
            }
            OverlayKind::Panel | OverlayKind::Tooltip => {
                let panel = new_panel(mtm);
                panel.setContentView(Some(&content));
                if props.kind == OverlayKind::Tooltip {
                    panel.setIgnoresMouseEvents(true);
                }
                content.set_props(&panel_look(props));
                Presenter::Panel(panel)
            }
        };
        Self {
            content,
            node,
            props: props.clone(),
            presenter,
            anchor: RefCell::new(None),
            owner: RefCell::new(None),
            size: Cell::new(NSSize::ZERO),
            attached: Cell::new(false),
            dirty: Cell::new(false),
            emit,
            wake,
        }
    }

    pub fn update(&mut self, props: &OverlayProps) {
        if props.style != self.props.style {
            let look = match props.kind {
                OverlayKind::Popover => ViewLook::from_style(&props.style, false),
                OverlayKind::Panel | OverlayKind::Tooltip => panel_look(props),
            };
            self.content.set_props(&look);
        }
        let moved = props.placement != self.props.placement;
        self.props = props.clone();
        if moved {
            self.dirty.set(true);
        }
    }

    /// Attached under `anchor` (its parent's view) in `owner`; shown at the commit.
    pub fn attach(&self, anchor: Option<Retained<NSView>>, owner: Retained<NdWindow>) {
        *self.anchor.borrow_mut() = anchor;
        *self.owner.borrow_mut() = Some(owner);
        self.attached.set(true);
        self.dirty.set(true);
    }

    pub fn detach(&self) {
        self.attached.set(false);
        self.hide();
    }

    pub fn set_size(&self, frame: Rect) {
        let size = NSSize::new(frame.size.width.into(), frame.size.height.into());
        if self.size.replace(size) != size {
            self.dirty.set(true);
        }
    }

    /// Shows or moves the overlay to match its anchor, size and placement. Runs at commit.
    pub fn present(&self) {
        if !self.attached.get() || !self.dirty.replace(false) {
            return;
        }
        let Some(owner) = self.owner.borrow().clone() else { return };
        let size = self.size.get();
        match &self.presenter {
            Presenter::Popover(popover) => {
                let Some(anchor) = self.anchor.borrow().clone() else { return };
                if anchor.window().is_none() {
                    // An unmapped anchor: try again on the next commit that touches us.
                    self.dirty.set(true);
                    return;
                }
                popover.setContentSize(size);
                let flipped = anchor.isFlipped();
                let edge = match self.props.placement {
                    Placement::Below if flipped => NSRectEdge::MaxY,
                    Placement::Below => NSRectEdge::MinY,
                    Placement::Above if flipped => NSRectEdge::MinY,
                    Placement::Above => NSRectEdge::MaxY,
                    Placement::Leading => NSRectEdge::MinX,
                    Placement::Trailing => NSRectEdge::MaxX,
                };
                popover.showRelativeToRect_ofView_preferredEdge(anchor.bounds(), &anchor, edge);
            }
            Presenter::Panel(panel) => {
                let frame = match self.props.kind {
                    OverlayKind::Tooltip => {
                        let Some(anchor) = self.anchor.borrow().clone() else { return };
                        let Some(window) = anchor.window() else {
                            self.dirty.set(true);
                            return;
                        };
                        let rect = window.convertRectToScreen(anchor.convertRect_toView(anchor.bounds(), None));
                        let x = rect.origin.x + (rect.size.width - size.width) / 2.;
                        let y = match self.props.placement {
                            Placement::Below => rect.origin.y - TOOLTIP_GAP - size.height,
                            _ => rect.origin.y + rect.size.height + TOOLTIP_GAP,
                        };
                        NSRect::new(NSPoint::new(x, y), size)
                    }
                    _ => {
                        let content = owner.contentLayoutRect();
                        let screen = owner.convertRectToScreen(content);
                        let x = screen.origin.x + (screen.size.width - size.width) / 2.;
                        let top = screen.origin.y + screen.size.height * (1. - PANEL_TOP);
                        NSRect::new(NSPoint::new(x, top - size.height), size)
                    }
                };
                panel.setFrame_display(frame, true);
                panel.setAppearance(owner.appearance().as_deref());
                let opening = panel.parentWindow().is_none();
                if opening {
                    unsafe { owner.addChildWindow_ordered(panel, NSWindowOrderingMode::Above) };
                }
                let fade_in = opening && self.props.kind == OverlayKind::Panel && fades();
                if fade_in {
                    panel.setAlphaValue(0.);
                }
                panel.orderFront(None);
                if fade_in {
                    fade(panel, 1., || {});
                } else if opening {
                    panel.setAlphaValue(1.);
                }
            }
        }
        register(self);
    }

    fn hide(&self) {
        unregister(self.node);
        match &self.presenter {
            Presenter::Popover(popover) => {
                if popover.isShown() {
                    popover.close();
                }
            }
            Presenter::Panel(panel) => {
                let parent = panel.parentWindow();
                if let Some(parent) = &parent {
                    parent.removeChildWindow(panel);
                }
                if parent.is_some() && self.props.kind == OverlayKind::Panel && panel.isVisible() && fades() {
                    // The panel window outlives the node for the fade: the block holds it.
                    let closing = panel.retain();
                    // Reopened while it faded: it is a child window again and stays up.
                    fade(panel, 0., move || {
                        if closing.parentWindow().is_none() {
                            closing.orderOut(None);
                        }
                    });
                } else {
                    panel.orderOut(None);
                }
            }
        }
    }
}

impl Drop for OverlayNative {
    fn drop(&mut self) {
        self.hide();
    }
}

/// A panel draws what the app styled, on the window background when the app set none: unlike a
/// popover it has no system material of its own.
fn panel_look(props: &OverlayProps) -> ViewLook {
    let mut look = ViewLook::from_style(&props.style, false);
    look.background.get_or_insert(crate::look::Color::WindowBackground);
    look
}

fn new_panel(mtm: MainThreadMarker) -> Retained<NSPanel> {
    let style = NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel;
    let panel = NSPanel::initWithContentRect_styleMask_backing_defer(
        NSPanel::alloc(mtm),
        NSRect::new(NSPoint::ZERO, NSSize::new(1., 1.)), // nd-lint: allow, placeholder until the first layout
        style,
        NSBackingStoreType::Buffered,
        false,
    );
    unsafe { panel.setReleasedWhenClosed(false) };
    panel.setOpaque(false);
    panel.setBackgroundColor(Some(&NSColor::clearColor()));
    panel.setHasShadow(true);
    panel.setHidesOnDeactivate(false);
    panel
}

struct Entry {
    node: NodeId,
    content: Retained<NdLayerView>,
    modality: Modality,
    emit: Emit,
    wake: Wake,
}

#[derive(Default)]
struct Open {
    entries: Vec<Entry>,
    monitor: Option<Retained<AnyObject>>,
    /// A press was consumed: its release is consumed too, even after the overlay closed under
    /// it, so nothing below sees half a click. Keeps the monitor alive until then.
    swallow_up: bool,
}

thread_local! {
    static OPEN: RefCell<Open> = RefCell::default();
}

fn register(overlay: &OverlayNative) {
    OPEN.with(|open| {
        let mut open = open.borrow_mut();
        if open.entries.iter().any(|e| e.node == overlay.node) {
            return;
        }
        open.entries.push(Entry {
            node: overlay.node,
            content: overlay.content.clone(),
            modality: overlay.props.modality,
            emit: overlay.emit.clone(),
            wake: overlay.wake.clone(),
        });
        if open.monitor.is_none() {
            let mask = NSEventMask::LeftMouseDown
                | NSEventMask::RightMouseDown
                | NSEventMask::OtherMouseDown
                | NSEventMask::LeftMouseUp
                | NSEventMask::RightMouseUp
                | NSEventMask::OtherMouseUp
                | NSEventMask::KeyDown;
            let block = RcBlock::new(|event: std::ptr::NonNull<NSEvent>| -> *mut NSEvent {
                let event = unsafe { event.as_ref() };
                if consume(event) { std::ptr::null_mut() } else { Retained::as_ptr(&event.retain()).cast_mut() }
            });
            open.monitor = unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(mask, &block) };
        }
    });
}

fn unregister(node: NodeId) {
    OPEN.with(|open| open.borrow_mut().entries.retain(|e| e.node != node));
    remove_idle_monitor();
}

/// Removes the monitor once no overlay is open and no release is owed, on a later turn: never
/// from inside the monitor's own handler.
fn remove_idle_monitor() {
    crate::events::defer(|| {
        let monitor = OPEN.with(|open| {
            let mut open = open.borrow_mut();
            if open.entries.is_empty() && !open.swallow_up { open.monitor.take() } else { None }
        });
        if let Some(monitor) = monitor {
            unsafe { NSEvent::removeMonitor(&monitor) };
        }
    });
}

fn is_nd_window(window: &NSWindow) -> bool {
    let any: &AnyObject = window.as_ref();
    any.downcast_ref::<NdWindow>().is_some()
}

/// The monitor's decision for one event: true consumes it.
fn consume(event: &NSEvent) -> bool {
    let Some(mtm) = MainThreadMarker::new() else { return false };
    let target = event.window(mtm);
    let kind = event.r#type();
    if matches!(kind, NSEventType::LeftMouseUp | NSEventType::RightMouseUp | NSEventType::OtherMouseUp) {
        let owed = OPEN.with(|open| std::mem::take(&mut open.borrow_mut().swallow_up));
        if owed {
            remove_idle_monitor();
        }
        return owed;
    }
    let consumed = OPEN.with(|open| {
        let open = open.borrow();
        let level = target.as_ref().and_then(|t| {
            open.entries.iter().rposition(|e| e.content.window().is_some_and(|w| std::ptr::eq(&*w, &**t)))
        });
        if kind == NSEventType::KeyDown {
            // Esc while an overlay window is key; in an nd window, nd dispatch has the same rule.
            if event.keyCode() != 53 || level.is_none() {
                return false;
            }
            let Some(entry) = open.entries.iter().rev().find(|e| e.modality != Modality::Passive) else {
                return false;
            };
            entry.emit.emit(Dismissed { reason: DismissCause::Escape });
            entry.wake.wake();
            return true;
        }
        let above = level.map_or(0, |i| i + 1);
        let mut blocked = false;
        for entry in &open.entries[above..] {
            match entry.modality {
                Modality::LightDismiss => {
                    entry.emit.emit(Dismissed { reason: DismissCause::OutsidePress });
                    entry.wake.wake();
                    blocked = true;
                }
                Modality::Modal => blocked = true,
                Modality::Passive => {}
            }
        }
        blocked && (level.is_some() || target.as_deref().is_some_and(is_nd_window))
    });
    if consumed && kind != NSEventType::KeyDown {
        OPEN.with(|open| open.borrow_mut().swallow_up = true);
    }
    consumed
}

/// Shown overlays' windows, for automation: bounds of nodes inside them convert through the
/// screen into their owner's coordinates.
pub(crate) fn screen_rect(view: &NSView) -> Option<NSRect> {
    let window = view.window()?;
    Some(window.convertRectToScreen(view.convertRect_toView(view.bounds(), None)))
}

#[derive(Default)]
pub struct LayerPanelIvars {
    /// Takes key status (a panel with a text field); otherwise the owner stays key.
    key: Cell<bool>,
    owner: RefCell<Option<objc2::rc::Weak<NdWindow>>>,
}

define_class!(
    /// The panel a [`WindowLayer`] draws in. A key one hands the shortcuts its own views did not
    /// take to its owner's nd dispatch, so menu accelerators and app bindings keep working while
    /// a field in it has focus; the main menu still gets what nd leaves.
    // SAFETY: NSPanel has no subclassing requirements beyond calling super; no Drop.
    #[unsafe(super(NSPanel, NSWindow, NSResponder))]
    #[thread_kind = MainThreadOnly]
    #[ivars = LayerPanelIvars]
    pub struct NdLayerPanel;

    impl NdLayerPanel {
        #[unsafe(method(canBecomeKeyWindow))]
        fn can_become_key(&self) -> bool {
            self.ivars().key.get()
        }

        #[unsafe(method(canBecomeMainWindow))]
        fn can_become_main(&self) -> bool {
            false
        }

        #[unsafe(method(performKeyEquivalent:))]
        fn perform_key_equivalent(&self, event: &NSEvent) -> bool {
            let mine: bool = unsafe { msg_send![super(self), performKeyEquivalent: event] };
            // Only shortcuts: plain keys, and secondary-Return, belong to the panel's own field.
            let flags = event.modifierFlags();
            let shortcut = (flags.contains(NSEventModifierFlags::Command) || flags.contains(NSEventModifierFlags::Control))
                && !matches!(event.keyCode(), 36 | 76);
            let owner = self.ivars().owner.borrow().as_ref().and_then(objc2::rc::Weak::load);
            mine || (shortcut && owner.is_some_and(|o| o.route_key_from_overlay(event)))
        }
    }
);

/// A borderless child panel over the whole of a window, title bar included, kept on the owner's
/// frame as it moves and resizes: where framework panels (the command palette, toasts) draw, so
/// they sit above every view in the window, native surfaces included, and take their input from
/// the window server. Clicks on its transparent areas pass through to the owner.
pub(crate) struct WindowLayer {
    panel: Retained<NdLayerPanel>,
    owner: Retained<NSWindow>,
    observers: Vec<Retained<objc2::runtime::ProtocolObject<dyn objc2_foundation::NSObjectProtocol>>>,
}

impl WindowLayer {
    pub fn new(mtm: MainThreadMarker, owner: &NSWindow, key: bool) -> Self {
        let style = NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel;
        let panel = NdLayerPanel::alloc(mtm).set_ivars(LayerPanelIvars::default());
        let panel: Retained<NdLayerPanel> = unsafe {
            msg_send![
                super(panel),
                initWithContentRect: owner.frame(),
                styleMask: style,
                backing: NSBackingStoreType::Buffered,
                defer: false
            ]
        };
        panel.ivars().key.set(key);
        let nd_owner = {
            let any: &AnyObject = owner.as_ref();
            any.downcast_ref::<NdWindow>().map(objc2::rc::Weak::from)
        };
        *panel.ivars().owner.borrow_mut() = nd_owner;
        unsafe { panel.setReleasedWhenClosed(false) };
        panel.setOpaque(false);
        panel.setBackgroundColor(Some(&NSColor::clearColor()));
        panel.setHasShadow(false);
        panel.setHidesOnDeactivate(false);
        panel.setFrame_display(owner.frame(), false);
        panel.setAppearance(owner.appearance().as_deref());
        unsafe { owner.addChildWindow_ordered(&panel, NSWindowOrderingMode::Above) };
        let center = objc2_foundation::NSNotificationCenter::defaultCenter();
        let mut observers = Vec::new();
        for name in
            unsafe { [objc2_app_kit::NSWindowDidResizeNotification, objc2_app_kit::NSWindowDidMoveNotification] }
        {
            let (weak_panel, weak_owner) =
                (objc2::rc::Weak::from_retained(&panel), objc2::rc::Weak::from_retained(&owner.retain()));
            let block = RcBlock::new(move |_n: std::ptr::NonNull<objc2_foundation::NSNotification>| {
                if let (Some(panel), Some(owner)) = (weak_panel.load(), weak_owner.load()) {
                    panel.setFrame_display(owner.frame(), true);
                }
            });
            let any: &AnyObject = owner.as_ref();
            observers.push(unsafe {
                center.addObserverForName_object_queue_usingBlock(Some(name), Some(any), None, &block)
            });
        }
        Self { panel, owner: owner.retain(), observers }
    }

    /// The flipped-or-not view to add content to, the size of the owner's frame.
    pub fn host(&self) -> Retained<NSView> {
        self.panel.contentView().expect("panel content view")
    }

    pub fn owner(&self) -> &NSWindow {
        &self.owner
    }

    pub fn window(&self) -> &NSWindow {
        &self.panel
    }

    /// Shows the panel; a key one takes key status, so its field gets the keyboard. It takes the
    /// owner's appearance (a window-level theme override), so materials and labels in it
    /// resolve as they would in the window.
    pub fn show(&self) {
        self.panel.setAppearance(self.owner.appearance().as_deref());
        if self.panel.ivars().key.get() {
            self.panel.makeKeyAndOrderFront(None);
        } else {
            self.panel.orderFront(None);
        }
    }
}

impl Drop for WindowLayer {
    fn drop(&mut self) {
        let center = objc2_foundation::NSNotificationCenter::defaultCenter();
        for observer in &self.observers {
            let any: &AnyObject = observer.as_ref();
            unsafe { center.removeObserver(any) };
        }
        let was_key = self.panel.isKeyWindow();
        self.owner.removeChildWindow(&self.panel);
        self.panel.orderOut(None);
        if was_key {
            self.owner.makeKeyWindow();
        }
    }
}
