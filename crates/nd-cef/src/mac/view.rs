//! The page's view: an ordinary layer-backed `NSView` among the window's views. Chromium's
//! frames become its layer contents; every input event it receives is forwarded to Chromium;
//! it is the text input client, the drag source and destination, and the accessibility parent
//! of the page. Its display link paces Chromium's frames.

use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::rc::Rc;

use cef::*;
use objc2::rc::{Retained, Weak};
use objc2::runtime::{AnyObject, NSObjectProtocol, ProtocolObject, Sel};
use objc2::{AnyThread, DefinedClass, MainThreadOnly, Message, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAccessibilityElement, NSApplication, NSCursor, NSDragOperation, NSDraggingContext, NSDraggingDestination,
    NSDraggingInfo, NSDraggingItem, NSDraggingSession, NSDraggingSource, NSEvent, NSEventModifierFlags, NSEventType,
    NSImage, NSPasteboardItem, NSPasteboardTypeFileURL, NSPasteboardTypeHTML, NSPasteboardTypeString,
    NSPasteboardTypeURL, NSResponder, NSScreen, NSTextInputClient, NSTrackingArea, NSTrackingAreaOptions, NSView,
    NSViewLayerContentsRedrawPolicy,
};
use objc2_foundation::{
    MainThreadMarker, NSArray, NSAttributedString, NSAttributedStringKey, NSData, NSNotFound, NSNull, NSPoint, NSRange,
    NSRangePointer, NSRect, NSSize, NSString, NSTimer, NSUInteger, NSURL,
};
use objc2_quartz_core::{CADisplayLink, CALayer, CATransaction, kCAGravityTopLeft};

use super::keys;
use super::paint::Pool;
use crate::a11y;
use crate::keys::{Edit, edit_command, flag};
use crate::page::{self, CefPage};

/// A page's view, as the shared page code holds it.
pub(crate) type View = Retained<OsrView>;

pub(crate) fn new(id: u64) -> View {
    let mtm = MainThreadMarker::new().expect("pages are created on the main thread");
    OsrView::new(mtm, id)
}

/// Display ticks without a paint before the link stops and a slow probe takes over.
const IDLE_TICKS: u32 = 30;
/// While idle, how often Chromium still gets a begin frame, so a page that changes on its own
/// (a timer, a network response) is drawn within this long.
const PROBE_INTERVAL: f64 = 0.2;

#[derive(Default)]
struct Ime {
    marked: String,
    in_key_down: bool,
    inserted: Option<String>,
    bounds: Vec<Rect>,
    selection: String,
}

pub struct ViewIvars {
    id: Cell<u64>,
    pool: RefCell<Pool>,
    popup_pool: RefCell<Pool>,
    popup: RefCell<Option<Retained<CALayer>>>,
    popup_visible: Cell<bool>,
    cursor: RefCell<Option<Retained<NSCursor>>>,
    ime: RefCell<Ime>,
    tracking: RefCell<Option<Retained<NSTrackingArea>>>,
    link: RefCell<Option<Retained<CADisplayLink>>>,
    probe: RefCell<Option<Retained<NSTimer>>>,
    idle: Cell<u32>,
    drag_op: Cell<u32>,
    drag_source_ops: Cell<u32>,
    last_drag_event: RefCell<Option<Retained<NSEvent>>>,
    tree: RefCell<a11y::Tree>,
    accessibility_on: Cell<bool>,
    elements: RefCell<Option<Retained<NSArray<AnyObject>>>>,
    weak: RefCell<Weak<OsrView>>,
}

define_class!(
    // SAFETY: NSView has no subclassing requirements; overrides call super where AppKit expects
    // it. No Drop impl.
    #[unsafe(super(NSView, NSResponder))]
    #[thread_kind = MainThreadOnly]
    #[ivars = ViewIvars]
    pub struct OsrView;

    unsafe impl NSObjectProtocol for OsrView {}

    impl OsrView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool {
            true
        }

        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            true
        }

        #[unsafe(method(becomeFirstResponder))]
        fn become_first_responder(&self) -> bool {
            if let Some(host) = self.host() {
                host.set_focus(1);
            }
            if let Some(p) = self.page() {
                p.page.report_focus(true);
            }
            true
        }

        #[unsafe(method(resignFirstResponder))]
        fn resign_first_responder(&self) -> bool {
            if let Some(host) = self.host() {
                host.set_focus(0);
            }
            if let Some(p) = self.page() {
                p.page.report_focus(false);
            }
            true
        }

        #[unsafe(method(setFrameSize:))]
        fn set_frame_size(&self, size: NSSize) {
            let _: () = unsafe { msg_send![super(self), setFrameSize: size] };
            if let Some(host) = self.host() {
                host.was_resized();
                self.wake();
            }
        }

        #[unsafe(method(viewDidChangeBackingProperties))]
        fn did_change_backing(&self) {
            let _: () = unsafe { msg_send![super(self), viewDidChangeBackingProperties] };
            self.sync_scale();
            if let Some(host) = self.host() {
                host.notify_screen_info_changed();
                host.was_resized();
                self.wake();
            }
        }

        #[unsafe(method(viewDidMoveToWindow))]
        fn did_move_to_window(&self) {
            let _: () = unsafe { msg_send![super(self), viewDidMoveToWindow] };
            let in_window = self.window().is_some();
            if in_window {
                self.start_link();
            } else {
                self.stop_link();
            }
            self.sync_scale();
            if let Some(host) = self.host() {
                host.was_hidden(i32::from(!in_window));
                host.notify_screen_info_changed();
                host.was_resized();
            }
        }

        #[unsafe(method(updateTrackingAreas))]
        fn update_tracking_areas(&self) {
            if let Some(old) = self.ivars().tracking.borrow_mut().take() {
                self.removeTrackingArea(&old);
            }
            let options = NSTrackingAreaOptions::MouseEnteredAndExited
                | NSTrackingAreaOptions::MouseMoved
                | NSTrackingAreaOptions::CursorUpdate
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
            *self.ivars().tracking.borrow_mut() = Some(area);
            let _: () = unsafe { msg_send![super(self), updateTrackingAreas] };
        }

        #[unsafe(method(cursorUpdate:))]
        fn cursor_update(&self, _event: &NSEvent) {
            let cursor = self.ivars().cursor.borrow().clone().unwrap_or_else(NSCursor::arrowCursor);
            cursor.set();
        }

        #[unsafe(method(tick:))]
        fn tick(&self, _link: &CADisplayLink) {
            self.begin_frame();
            let idle = self.ivars().idle.get() + 1;
            self.ivars().idle.set(idle);
            if idle > IDLE_TICKS {
                self.go_idle();
            }
        }

        #[unsafe(method(probe:))]
        fn probe(&self, _timer: &NSTimer) {
            self.begin_frame();
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            if let Some(window) = self.window()
                && !window.firstResponder().is_some_and(|r| std::ptr::eq(&*r, self.as_ref()))
            {
                window.makeFirstResponder(Some(self));
            }
            self.click(event, MouseButtonType::LEFT, false);
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, event: &NSEvent) {
            self.click(event, MouseButtonType::LEFT, true);
        }

        #[unsafe(method(rightMouseDown:))]
        fn right_mouse_down(&self, event: &NSEvent) {
            self.click(event, MouseButtonType::RIGHT, false);
        }

        #[unsafe(method(rightMouseUp:))]
        fn right_mouse_up(&self, event: &NSEvent) {
            self.click(event, MouseButtonType::RIGHT, true);
        }

        #[unsafe(method(otherMouseDown:))]
        fn other_mouse_down(&self, event: &NSEvent) {
            if event.buttonNumber() == 2 {
                self.click(event, MouseButtonType::MIDDLE, false);
            }
        }

        #[unsafe(method(otherMouseUp:))]
        fn other_mouse_up(&self, event: &NSEvent) {
            if event.buttonNumber() == 2 {
                self.click(event, MouseButtonType::MIDDLE, true);
            }
        }

        #[unsafe(method(mouseMoved:))]
        fn mouse_moved(&self, event: &NSEvent) {
            self.move_mouse(event, false);
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, event: &NSEvent) {
            *self.ivars().last_drag_event.borrow_mut() = Some(event.retain());
            self.move_mouse(event, false);
        }

        #[unsafe(method(rightMouseDragged:))]
        fn right_mouse_dragged(&self, event: &NSEvent) {
            self.move_mouse(event, false);
        }

        #[unsafe(method(otherMouseDragged:))]
        fn other_mouse_dragged(&self, event: &NSEvent) {
            self.move_mouse(event, false);
        }

        #[unsafe(method(mouseEntered:))]
        fn mouse_entered(&self, event: &NSEvent) {
            self.move_mouse(event, false);
        }

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, event: &NSEvent) {
            self.move_mouse(event, true);
        }

        #[unsafe(method(scrollWheel:))]
        fn scroll_wheel(&self, event: &NSEvent) {
            let Some(host) = self.host() else { return };
            let precise = event.hasPreciseScrollingDeltas();
            let (dx, dy) = keys::wheel_delta(precise, event.scrollingDeltaX(), event.scrollingDeltaY());
            let mut mouse = self.mouse_event(event);
            if precise {
                mouse.modifiers |= flag::PRECISION_SCROLLING_DELTA;
            }
            host.send_mouse_wheel_event(Some(&mouse), dx, dy);
            self.wake();
        }

        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            self.key(event);
        }

        #[unsafe(method(keyUp:))]
        fn key_up(&self, event: &NSEvent) {
            if let Some(host) = self.host() {
                host.send_key_event(Some(&key_event(event, KeyEventType::KEYUP)));
                self.wake();
            }
        }

        #[unsafe(method(flagsChanged:))]
        fn flags_changed(&self, event: &NSEvent) {
            let Some(host) = self.host() else { return };
            let flag = match event.keyCode() {
                56 | 60 => NSEventModifierFlags::Shift,
                59 | 62 => NSEventModifierFlags::Control,
                58 | 61 => NSEventModifierFlags::Option,
                55 | 54 => NSEventModifierFlags::Command,
                57 => NSEventModifierFlags::CapsLock,
                _ => return,
            };
            let kind = if event.modifierFlags().contains(flag) { KeyEventType::RAWKEYDOWN } else { KeyEventType::KEYUP };
            let mut key = key_event(event, kind);
            key.character = 0;
            key.unmodified_character = 0;
            host.send_key_event(Some(&key));
        }

        /// Cmd shortcuts reach the view here when no menu item took them: the edit commands
        /// run in the focused frame, the rest go to the page as keys.
        #[unsafe(method(performKeyEquivalent:))]
        fn perform_key_equivalent(&self, event: &NSEvent) -> bool {
            self.key_equivalent(event)
        }

        #[unsafe(method(copy:))]
        fn copy(&self, _sender: Option<&AnyObject>) {
            self.edit(Edit::Copy);
        }

        #[unsafe(method(cut:))]
        fn cut(&self, _sender: Option<&AnyObject>) {
            self.edit(Edit::Cut);
        }

        #[unsafe(method(paste:))]
        fn paste(&self, _sender: Option<&AnyObject>) {
            self.edit(Edit::Paste);
        }

        #[unsafe(method(pasteAsPlainText:))]
        fn paste_plain(&self, _sender: Option<&AnyObject>) {
            self.edit(Edit::PasteAndMatchStyle);
        }

        #[unsafe(method(selectAll:))]
        fn select_all(&self, _sender: Option<&AnyObject>) {
            self.edit(Edit::SelectAll);
        }

        #[unsafe(method(undo:))]
        fn undo(&self, _sender: Option<&AnyObject>) {
            self.edit(Edit::Undo);
        }

        #[unsafe(method(redo:))]
        fn redo(&self, _sender: Option<&AnyObject>) {
            self.edit(Edit::Redo);
        }

        #[unsafe(method(isAccessibilityElement))]
        fn is_accessibility_element(&self) -> bool {
            false
        }

        #[unsafe(method_id(accessibilityChildren))]
        fn accessibility_children(&self) -> Option<Retained<NSArray<AnyObject>>> {
            self.enable_accessibility();
            self.ivars().elements.borrow().clone()
        }
    }

    unsafe impl NSTextInputClient for OsrView {
        #[unsafe(method(insertText:replacementRange:))]
        fn insert_text(&self, string: &AnyObject, _range: NSRange) {
            let text = string_of(string);
            let composing = {
                let mut ime = self.ivars().ime.borrow_mut();
                let composing = !ime.marked.is_empty();
                ime.marked.clear();
                if ime.in_key_down && !composing {
                    ime.inserted.get_or_insert_with(String::new).push_str(&text);
                    return;
                }
                composing
            };
            let _ = composing;
            if let Some(host) = self.host() {
                host.ime_commit_text(Some(&CefString::from(text.as_str())), None, 0);
                self.wake();
            }
        }

        #[unsafe(method(doCommandBySelector:))]
        fn do_command_by_selector(&self, _selector: Sel) {
            // Keys without text (arrows, Delete): `keyDown:` sends them as raw keys.
        }

        #[unsafe(method(setMarkedText:selectedRange:replacementRange:))]
        fn set_marked_text(&self, string: &AnyObject, selected: NSRange, _replacement: NSRange) {
            let text = string_of(string);
            self.ivars().ime.borrow_mut().marked = text.clone();
            let Some(host) = self.host() else { return };
            if text.is_empty() {
                host.ime_cancel_composition();
                return;
            }
            let len = text.encode_utf16().count() as u32;
            let underline = CompositionUnderline {
                range: Range { from: 0, to: len },
                color: 0xff00_0000,
                background_color: 0,
                thick: 0,
                style: CompositionUnderlineStyle::SOLID,
                ..CompositionUnderline::default()
            };
            let from = selected.location as u32;
            let selection = Range { from, to: from + selected.length as u32 };
            host.ime_set_composition(Some(&CefString::from(text.as_str())), Some(&[underline]), None, Some(&selection));
            self.wake();
        }

        #[unsafe(method(unmarkText))]
        fn unmark_text(&self) {
            let had = !std::mem::take(&mut self.ivars().ime.borrow_mut().marked).is_empty();
            if let (true, Some(host)) = (had, self.host()) {
                host.ime_finish_composing_text(0);
            }
        }

        #[unsafe(method(selectedRange))]
        fn selected_range(&self) -> NSRange {
            NSRange::new(NSNotFound as NSUInteger, 0)
        }

        #[unsafe(method(markedRange))]
        fn marked_range(&self) -> NSRange {
            match self.ivars().ime.borrow().marked.encode_utf16().count() {
                0 => NSRange::new(NSNotFound as NSUInteger, 0),
                len => NSRange::new(0, len),
            }
        }

        #[unsafe(method(hasMarkedText))]
        fn has_marked_text(&self) -> bool {
            !self.ivars().ime.borrow().marked.is_empty()
        }

        #[unsafe(method_id(attributedSubstringForProposedRange:actualRange:))]
        fn attributed_substring(&self, _range: NSRange, _actual: NSRangePointer) -> Option<Retained<NSAttributedString>> {
            None
        }

        #[unsafe(method_id(validAttributesForMarkedText))]
        fn valid_attributes(&self) -> Retained<NSArray<NSAttributedStringKey>> {
            NSArray::new()
        }

        /// Where the candidate window opens: under the composition's characters, which
        /// Chromium reports in view coordinates.
        #[unsafe(method(firstRectForCharacterRange:actualRange:))]
        fn first_rect(&self, range: NSRange, _actual: NSRangePointer) -> NSRect {
            let rect = {
                let ime = self.ivars().ime.borrow();
                let i = range.location.min(ime.bounds.len().saturating_sub(1));
                ime.bounds.get(i).cloned()
            };
            let local = rect.map_or(NSRect::new(NSPoint::new(0., 0.), NSSize::new(0., 16.)), |r| {
                NSRect::new(NSPoint::new(f64::from(r.x), f64::from(r.y)), NSSize::new(f64::from(r.width), f64::from(r.height)))
            });
            let in_window = self.convertRect_toView(local, None);
            self.window().map_or(NSRect::ZERO, |w| w.convertRectToScreen(in_window))
        }

        #[unsafe(method(characterIndexForPoint:))]
        fn character_index_for_point(&self, _point: NSPoint) -> NSUInteger {
            NSNotFound as NSUInteger
        }
    }

    unsafe impl NSDraggingDestination for OsrView {
        #[unsafe(method(draggingEntered:))]
        fn dragging_entered(&self, sender: &ProtocolObject<dyn NSDraggingInfo>) -> NSDragOperation {
            let Some(host) = self.host() else { return NSDragOperation::None };
            let Some(mut data) = drag_data_from(sender) else { return NSDragOperation::None };
            let event = self.drag_event(sender);
            let allowed = ops(sender.draggingSourceOperationMask().0 as u32);
            self.ivars().drag_op.set(sender.draggingSourceOperationMask().0 as u32 & 1);
            host.drag_target_drag_enter(Some(&mut data), Some(&event), allowed);
            self.wake();
            NSDragOperation(self.ivars().drag_op.get() as usize)
        }

        #[unsafe(method(draggingUpdated:))]
        fn dragging_updated(&self, sender: &ProtocolObject<dyn NSDraggingInfo>) -> NSDragOperation {
            if let Some(host) = self.host() {
                let event = self.drag_event(sender);
                host.drag_target_drag_over(Some(&event), ops(sender.draggingSourceOperationMask().0 as u32));
                self.wake();
            }
            NSDragOperation(self.ivars().drag_op.get() as usize)
        }

        #[unsafe(method(draggingExited:))]
        fn dragging_exited(&self, _sender: Option<&ProtocolObject<dyn NSDraggingInfo>>) {
            if let Some(host) = self.host() {
                host.drag_target_drag_leave();
            }
        }

        #[unsafe(method(performDragOperation:))]
        fn perform_drag_operation(&self, sender: &ProtocolObject<dyn NSDraggingInfo>) -> bool {
            self.drop_on_page(sender)
        }
    }

    unsafe impl NSDraggingSource for OsrView {
        #[unsafe(method(draggingSession:sourceOperationMaskForDraggingContext:))]
        fn source_mask(&self, _session: &NSDraggingSession, _context: NSDraggingContext) -> NSDragOperation {
            NSDragOperation(self.ivars().drag_source_ops.get() as usize)
        }

        #[unsafe(method(draggingSession:endedAtPoint:operation:))]
        fn session_ended(&self, _session: &NSDraggingSession, screen_point: NSPoint, operation: NSDragOperation) {
            let Some(host) = self.host() else { return };
            let (x, y) = self.view_point_from_screen(screen_point);
            host.drag_source_ended_at(x, y, ops(operation.0 as u32));
            host.drag_source_system_drag_ended();
            self.wake();
        }
    }
);

fn string_of(object: &AnyObject) -> String {
    if let Some(attributed) = object.downcast_ref::<NSAttributedString>() {
        return attributed.string().to_string();
    }
    object.downcast_ref::<NSString>().map(NSString::to_string).unwrap_or_default()
}

fn ops(bits: u32) -> DragOperationsMask {
    DragOperationsMask::from(sys::cef_drag_operations_mask_t(bits))
}

fn key_event(event: &NSEvent, kind: KeyEventType) -> KeyEvent {
    let chars = event.characters().map(|c| c.to_string()).unwrap_or_default();
    let unmodified = event.charactersIgnoringModifiers().map(|c| c.to_string()).unwrap_or_default();
    let mut modifiers = keys::modifiers(event.modifierFlags());
    if matches!(event.r#type(), NSEventType::KeyDown) && event.isARepeat() {
        modifiers |= flag::IS_REPEAT;
    }
    KeyEvent {
        type_: kind,
        modifiers,
        windows_key_code: 0,
        native_key_code: i32::from(event.keyCode()),
        is_system_key: 0,
        character: keys::first_unit(&chars),
        unmodified_character: keys::first_unit(&unmodified),
        focus_on_editable_field: 0,
        ..KeyEvent::default()
    }
}

/// The drag's pasteboard as Chromium drag data: files, a URL, text and HTML.
fn drag_data_from(sender: &ProtocolObject<dyn NSDraggingInfo>) -> Option<DragData> {
    let data = drag_data_create()?;
    let board = sender.draggingPasteboard();
    let items = board.pasteboardItems().unwrap_or_default();
    for item in items.iter() {
        if let Some(file) =
            item.stringForType(unsafe { NSPasteboardTypeFileURL }).and_then(|s| NSURL::URLWithString(&s))
            && let Some(path) = file.path()
        {
            let name = file.lastPathComponent().map(|n| n.to_string()).unwrap_or_default();
            data.add_file(Some(&CefString::from(path.to_string().as_str())), Some(&CefString::from(name.as_str())));
            continue;
        }
        if let Some(url) = item.stringForType(unsafe { NSPasteboardTypeURL }) {
            data.set_link_url(Some(&CefString::from(url.to_string().as_str())));
        }
        if let Some(text) = item.stringForType(unsafe { NSPasteboardTypeString }) {
            data.set_fragment_text(Some(&CefString::from(text.to_string().as_str())));
        }
        if let Some(html) = item.stringForType(unsafe { NSPasteboardTypeHTML }) {
            data.set_fragment_html(Some(&CefString::from(html.to_string().as_str())));
        }
    }
    Some(data)
}

impl OsrView {
    fn key_equivalent(&self, event: &NSEvent) -> bool {
        let first = self.window().and_then(|w| w.firstResponder()).is_some_and(|r| std::ptr::eq(&*r, self.as_ref()));
        if !first || event.r#type() != NSEventType::KeyDown {
            return false;
        }
        let flags = event.modifierFlags();
        if !flags.contains(NSEventModifierFlags::Command) {
            return false;
        }
        let key = event.charactersIgnoringModifiers().map(|c| c.to_string().to_lowercase()).unwrap_or_default();
        match edit_command(
            &key,
            flags.contains(NSEventModifierFlags::Shift),
            flags.contains(NSEventModifierFlags::Option),
        ) {
            Some(edit) => {
                self.edit(edit);
                true
            }
            None => false,
        }
    }

    fn drop_on_page(&self, sender: &ProtocolObject<dyn NSDraggingInfo>) -> bool {
        let Some(host) = self.host() else { return false };
        let event = self.drag_event(sender);
        host.drag_target_drop(Some(&event));
        self.wake();
        true
    }

    pub(crate) fn new(mtm: MainThreadMarker, id: u64) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ViewIvars {
            id: Cell::new(id),
            pool: RefCell::default(),
            popup_pool: RefCell::default(),
            popup: RefCell::default(),
            popup_visible: Cell::new(false),
            cursor: RefCell::default(),
            ime: RefCell::default(),
            tracking: RefCell::default(),
            link: RefCell::default(),
            probe: RefCell::default(),
            idle: Cell::new(0),
            drag_op: Cell::new(0),
            drag_source_ops: Cell::new(0),
            last_drag_event: RefCell::default(),
            tree: RefCell::default(),
            accessibility_on: Cell::new(false),
            elements: RefCell::default(),
            weak: RefCell::default(),
        });
        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] };
        *this.ivars().weak.borrow_mut() = Weak::from_retained(&this);
        this.setWantsLayer(true);
        this.setLayerContentsRedrawPolicy(NSViewLayerContentsRedrawPolicy::Never);
        if let Some(layer) = this.layer() {
            unsafe { layer.setContentsGravity(kCAGravityTopLeft) };
            layer.setBackgroundColor(Some(&objc2_core_graphics::CGColor::new_srgb(1., 1., 1., 1.)));
            layer.setGeometryFlipped(true);
            let none = NSNull::null();
            let actions = objc2_foundation::NSDictionary::from_slices(
                &[&*NSString::from_str("contents"), &*NSString::from_str("bounds"), &*NSString::from_str("position")],
                &[&*none as &AnyObject, &*none, &*none],
            );
            // SAFETY: NSNull answers CAAction (no animation), as Apple documents for this map.
            let actions: &objc2_foundation::NSDictionary<NSString, ProtocolObject<dyn objc2_quartz_core::CAAction>> =
                unsafe { &*Retained::as_ptr(&actions).cast() };
            layer.setActions(Some(actions));
        }
        let kinds =
            unsafe { [NSPasteboardTypeFileURL, NSPasteboardTypeURL, NSPasteboardTypeString, NSPasteboardTypeHTML] };
        let types = NSArray::from_slice(&kinds);
        this.registerForDraggedTypes(&types);
        this
    }

    /// How Chromium creates this view's browser: off-screen, frames as IOSurfaces, paced by
    /// begin frames the display link sends.
    pub(crate) fn window_info(&self) -> WindowInfo {
        WindowInfo {
            windowless_rendering_enabled: 1,
            shared_texture_enabled: 1,
            external_begin_frame_enabled: 1,
            parent_view: (self as *const Self).cast_mut().cast(),
            runtime_style: RuntimeStyle::ALLOY,
            ..WindowInfo::default()
        }
    }

    /// Pops the context menu up at `at` (view coordinates) and blocks until it closes.
    pub(crate) fn show_menu(
        &self,
        at: (f64, f64),
        model: &MenuModel,
        entries: &[nd_webview::menu::Entry<'_>],
        done: impl FnOnce(Option<crate::menu::Choice>),
    ) {
        let built = super::menu::build(self.mtm(), Some(model), entries);
        built.menu.popUpMenuPositioningItem_atLocation_inView(None, NSPoint::new(at.0, at.1), Some(self));
        let chosen = built.chosen.borrow_mut().take();
        done(chosen);
    }

    pub(crate) fn take_focus(&self, next: bool) {
        if let Some(window) = self.window() {
            if next {
                window.selectNextKeyView(None);
            } else {
                window.selectPreviousKeyView(None);
            }
        }
    }

    pub(crate) fn focus(&self) {
        if let Some(window) = self.window() {
            window.makeFirstResponder(Some(self));
        }
    }

    /// Delivers the key event AppKit is dispatching to the page.
    pub(crate) fn forward_current_key(&self) {
        if let Some(event) = NSApplication::sharedApplication(self.mtm()).currentEvent() {
            self.forward_key(&event);
        }
    }

    /// The page closed: out of the window, frames stopped.
    pub(crate) fn detach(&self) {
        self.stop_link();
        self.removeFromSuperview();
    }

    pub(crate) fn page(&self) -> Option<Rc<CefPage>> {
        page::get(self.ivars().id.get())
    }

    fn host(&self) -> Option<BrowserHost> {
        self.page()?.host()
    }

    fn scale(&self) -> f64 {
        self.window().map_or_else(
            || NSScreen::mainScreen(self.mtm()).map_or(2., |s| s.backingScaleFactor()),
            |w| w.backingScaleFactor(),
        )
    }

    fn sync_scale(&self) {
        let scale = self.scale();
        if let Some(layer) = self.layer() {
            layer.setContentsScale(scale);
        }
        if let Some(popup) = self.ivars().popup.borrow().as_ref() {
            popup.setContentsScale(scale);
        }
    }

    /// The view in CEF's view coordinates: DIPs, top-left origin.
    pub(crate) fn view_rect(&self) -> Rect {
        let b = self.bounds();
        Rect { x: 0, y: 0, width: (b.size.width.round() as i32).max(1), height: (b.size.height.round() as i32).max(1) }
    }

    /// Points from the top-left of the primary screen, which CEF calls screen coordinates.
    fn to_screen(&self, local: NSPoint) -> Option<(f64, f64)> {
        let window = self.window()?;
        let p = window.convertPointToScreen(self.convertPoint_toView(local, None));
        let primary = NSScreen::screens(self.mtm()).firstObject()?.frame().size.height;
        Some((p.x, primary - p.y))
    }

    fn view_point_from_screen(&self, screen: NSPoint) -> (i32, i32) {
        let Some(window) = self.window() else { return (0, 0) };
        let in_window = window.convertPointFromScreen(screen);
        let p = self.convertPoint_fromView(in_window, None);
        (p.x.round() as i32, p.y.round() as i32)
    }

    /// The view's bounds in screen coordinates. Reported as the whole screen, so Chromium keeps
    /// `<select>` popups and other widgets inside the view, where they can be drawn.
    pub(crate) fn screen_rect(&self) -> Option<Rect> {
        let (x, y) = self.to_screen(NSPoint::new(0., 0.))?;
        let r = self.view_rect();
        Some(Rect { x: x.round() as i32, y: y.round() as i32, width: r.width, height: r.height })
    }

    pub(crate) fn screen_point(&self, x: i32, y: i32) -> Option<(i32, i32)> {
        let (sx, sy) = self.to_screen(NSPoint::new(f64::from(x), f64::from(y)))?;
        Some((sx.round() as i32, sy.round() as i32))
    }

    pub(crate) fn screen_info(&self) -> Option<(f32, Rect)> {
        let rect = self.screen_rect().unwrap_or_else(|| self.view_rect());
        Some((self.scale() as f32, rect))
    }

    /// Shows a frame: the page, or the open `<select>` popup.
    pub(crate) fn present(&self, popup: bool, surface: *mut c_void) {
        let pool = if popup { &self.ivars().popup_pool } else { &self.ivars().pool };
        let Some(contents) = pool.borrow_mut().copy(surface, false) else { return };
        CATransaction::begin();
        CATransaction::setDisableActions(true);
        if popup {
            if let Some(layer) = self.ivars().popup.borrow().as_ref() {
                unsafe { layer.setContents(Some(&contents)) };
            }
        } else if let Some(layer) = self.layer() {
            unsafe { layer.setContents(Some(&contents)) };
        }
        CATransaction::commit();
        self.ivars().idle.set(0);
        self.wake();
    }

    pub(crate) fn popup_show(&self, show: bool) {
        self.ivars().popup_visible.set(show);
        if !show {
            if let Some(layer) = self.ivars().popup.borrow_mut().take() {
                layer.removeFromSuperlayer();
            }
            self.ivars().popup_pool.borrow_mut().clear();
            return;
        }
        if self.ivars().popup.borrow().is_none()
            && let Some(parent) = self.layer()
        {
            let layer = CALayer::new();
            unsafe { layer.setContentsGravity(kCAGravityTopLeft) };
            layer.setContentsScale(self.scale());
            layer.setGeometryFlipped(true);
            parent.addSublayer(&layer);
            *self.ivars().popup.borrow_mut() = Some(layer);
        }
    }

    pub(crate) fn popup_size(&self, rect: Rect) {
        if let Some(layer) = self.ivars().popup.borrow().as_ref() {
            CATransaction::begin();
            CATransaction::setDisableActions(true);
            layer.setFrame(NSRect::new(
                NSPoint::new(f64::from(rect.x), f64::from(rect.y)),
                NSSize::new(f64::from(rect.width), f64::from(rect.height)),
            ));
            CATransaction::commit();
        }
    }

    pub(crate) fn set_cursor(&self, cursor: *mut NSCursor) {
        // SAFETY: CEF hands the NSCursor it wants shown; it stays alive while retained here.
        let Some(cursor) = (unsafe { cursor.as_ref() }).map(|c| c.retain()) else { return };
        let inside = self.window().is_some_and(|w| {
            let p = self.convertPoint_fromView(w.mouseLocationOutsideOfEventStream(), None);
            objc2_foundation::NSPointInRect(p, self.bounds())
        });
        if inside {
            cursor.set();
        }
        *self.ivars().cursor.borrow_mut() = Some(cursor);
    }

    pub(crate) fn set_composition_bounds(&self, bounds: Vec<Rect>) {
        self.ivars().ime.borrow_mut().bounds = bounds;
        if let Some(context) = self.inputContext() {
            context.invalidateCharacterCoordinates();
        }
    }

    pub(crate) fn set_selection(&self, text: String) {
        self.ivars().ime.borrow_mut().selection = text;
    }

    fn mouse_event(&self, event: &NSEvent) -> MouseEvent {
        let p = self.convertPoint_fromView(event.locationInWindow(), None);
        MouseEvent {
            x: p.x.round() as i32,
            y: p.y.round() as i32,
            modifiers: keys::modifiers(event.modifierFlags()) | keys::buttons(NSEvent::pressedMouseButtons()),
        }
    }

    fn drag_event(&self, sender: &ProtocolObject<dyn NSDraggingInfo>) -> MouseEvent {
        let p = self.convertPoint_fromView(sender.draggingLocation(), None);
        let flags = NSEvent::modifierFlags_class();
        MouseEvent { x: p.x.round() as i32, y: p.y.round() as i32, modifiers: keys::modifiers(flags) }
    }

    fn click(&self, event: &NSEvent, button: MouseButtonType, up: bool) {
        let Some(host) = self.host() else { return };
        let mouse = self.mouse_event(event);
        host.send_mouse_click_event(Some(&mouse), button, i32::from(up), event.clickCount().max(1) as i32);
        self.wake();
    }

    fn move_mouse(&self, event: &NSEvent, leave: bool) {
        let Some(host) = self.host() else { return };
        host.send_mouse_move_event(Some(&self.mouse_event(event)), i32::from(leave));
        self.wake();
    }

    fn key(&self, event: &NSEvent) {
        let Some(host) = self.host() else { return };
        let had_marked = {
            let mut ime = self.ivars().ime.borrow_mut();
            ime.in_key_down = true;
            ime.inserted = None;
            !ime.marked.is_empty()
        };
        let handled = self.inputContext().is_some_and(|c| c.handleEvent(event));
        let (inserted, has_marked) = {
            let mut ime = self.ivars().ime.borrow_mut();
            ime.in_key_down = false;
            (ime.inserted.take(), !ime.marked.is_empty())
        };
        self.wake();
        if has_marked || (had_marked && handled && inserted.is_none()) {
            return;
        }
        host.send_key_event(Some(&key_event(event, KeyEventType::RAWKEYDOWN)));
        let command = event.modifierFlags().contains(NSEventModifierFlags::Command);
        let chars = event.characters().map(|c| c.to_string()).unwrap_or_default();
        let typed = match inserted {
            Some(text) if text.encode_utf16().count() == 1 => text.encode_utf16().next(),
            Some(text) => {
                host.ime_commit_text(Some(&CefString::from(text.as_str())), None, 0);
                None
            }
            None if !command => keys::command_character(&chars),
            None => None,
        };
        if let Some(c) = typed {
            let mut key = key_event(event, KeyEventType::CHAR);
            key.character = c;
            key.unmodified_character = c;
            host.send_key_event(Some(&key));
        }
    }

    /// Delivers a key event nd is dispatching (Tab) to the page, as if it came from AppKit.
    fn forward_key(&self, event: &NSEvent) {
        match event.r#type() {
            NSEventType::KeyDown => self.key(event),
            NSEventType::KeyUp => {
                let _: () = unsafe { msg_send![self, keyUp: event] };
            }
            _ => {}
        }
    }

    fn edit(&self, edit: Edit) {
        let Some(frame) = self.page().and_then(|p| p.focused_frame()) else { return };
        match edit {
            Edit::Undo => frame.undo(),
            Edit::Redo => frame.redo(),
            Edit::Cut => frame.cut(),
            Edit::Copy => frame.copy(),
            Edit::Paste => frame.paste(),
            Edit::PasteAndMatchStyle => frame.paste_and_match_style(),
            Edit::SelectAll => frame.select_all(),
        }
        self.wake();
    }

    /// A key the page did not handle goes to the main menu, so app accelerators work while a
    /// page has the keyboard.
    pub(crate) fn unhandled_key(&self, event: *mut NSEvent) -> bool {
        // SAFETY: CEF hands back the NSEvent it was given for this key.
        let Some(event) = (unsafe { event.as_ref() }) else { return false };
        let app = NSApplication::sharedApplication(self.mtm());
        app.mainMenu().is_some_and(|m| m.performKeyEquivalent(event))
    }

    pub(crate) fn start_dragging(&self, data: &DragData, allowed: DragOperationsMask, x: i32, y: i32) -> bool {
        let mtm = self.mtm();
        let item = NSPasteboardItem::new();
        let text = |s: &CefStringUserfree| crate::text(s);
        let link = text(&data.link_url());
        if !link.is_empty() {
            item.setString_forType(&NSString::from_str(&link), unsafe { NSPasteboardTypeURL });
            item.setString_forType(&NSString::from_str(&link), unsafe { NSPasteboardTypeString });
        }
        let fragment = text(&data.fragment_text());
        if !fragment.is_empty() && link.is_empty() {
            item.setString_forType(&NSString::from_str(&fragment), unsafe { NSPasteboardTypeString });
        }
        let html = text(&data.fragment_html());
        if !html.is_empty() {
            item.setString_forType(&NSString::from_str(&html), unsafe { NSPasteboardTypeHTML });
        }
        let image = data.image().and_then(|img| {
            let png = img.as_png(self.scale() as f32, 1, None, None)?;
            let mut bytes = vec![0u8; png.size()];
            png.data(Some(&mut bytes), 0);
            NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(&bytes))
        });
        let dragging =
            NSDraggingItem::initWithPasteboardWriter(NSDraggingItem::alloc(), ProtocolObject::from_ref(&*item));
        let size = image.as_ref().map_or(NSSize::new(64., 20.), |i| i.size());
        let hotspot = data.image_hotspot();
        let frame = NSRect::new(NSPoint::new(f64::from(x - hotspot.x), f64::from(y - hotspot.y)), size);
        unsafe { dragging.setDraggingFrame_contents(frame, image.as_deref().map(|i| i as &AnyObject)) };
        let event = self
            .ivars()
            .last_drag_event
            .borrow()
            .clone()
            .or_else(|| NSApplication::sharedApplication(mtm).currentEvent());
        let Some(event) = event else { return false };
        self.ivars().drag_source_ops.set(allowed.as_ref().0);
        let items = NSArray::from_retained_slice(&[dragging]);
        self.beginDraggingSessionWithItems_event_source(&items, &event, ProtocolObject::from_ref(self));
        true
    }

    pub(crate) fn set_drag_operation(&self, op: DragOperationsMask) {
        self.ivars().drag_op.set(op.as_ref().0);
    }

    fn enable_accessibility(&self) {
        if self.ivars().accessibility_on.replace(true) {
            return;
        }
        if let Some(host) = self.host() {
            host.set_accessibility_state(State::ENABLED);
        }
    }

    pub(crate) fn accessibility_update(&self, change: &serde_json::Value) {
        self.ivars().tree.borrow_mut().apply(change);
        self.rebuild_elements();
    }

    pub(crate) fn accessibility_locations(&self, change: &serde_json::Value) {
        self.ivars().tree.borrow_mut().locations(change);
        self.rebuild_elements();
    }

    /// Mirrors the tree as `NSAccessibilityElement`s, frames in screen coordinates.
    fn rebuild_elements(&self) {
        let tree = self.ivars().tree.borrow();
        let Some(root) = tree.root else { return };
        let Some(window) = self.window() else { return };
        let frame_of = |id: i64| {
            let (x, y, w, h) = tree.absolute(id);
            let local = NSRect::new(NSPoint::new(x, y), NSSize::new(w, h));
            window.convertRectToScreen(self.convertRect_toView(local, None))
        };
        fn build(
            tree: &a11y::Tree,
            id: i64,
            parent: &AnyObject,
            frame_of: &dyn Fn(i64) -> NSRect,
            out: &mut Vec<Retained<AnyObject>>,
        ) {
            let Some(node) = tree.nodes.get(&id) else { return };
            let Some(role) = a11y::ns_role(&node.role) else { return };
            let element = unsafe {
                NSAccessibilityElement::accessibilityElementWithRole_frame_label_parent(
                    &NSString::from_str(role),
                    frame_of(id),
                    Some(&NSString::from_str(&node.name)),
                    Some(parent),
                )
            };
            if !node.value.is_empty() {
                let value = NSString::from_str(&node.value);
                let _: () = unsafe { msg_send![&*element, setAccessibilityValue: &*value] };
            }
            let mut children = Vec::new();
            for child in &node.children {
                build(tree, *child, &element, frame_of, &mut children);
            }
            if !children.is_empty() {
                let array = NSArray::from_retained_slice(&children);
                let _: () = unsafe { msg_send![&*element, setAccessibilityChildren: &*array] };
            }
            out.push(element);
        }
        let mut top = Vec::new();
        build(&tree, root, self, &frame_of, &mut top);
        *self.ivars().elements.borrow_mut() = Some(NSArray::from_retained_slice(&top));
    }

    fn begin_frame(&self) {
        if let Some(host) = self.host() {
            host.send_external_begin_frame();
        }
    }

    /// Something may change what the page draws: frames run at the display's rate again.
    pub(crate) fn wake(&self) {
        self.ivars().idle.set(0);
        if let Some(timer) = self.ivars().probe.borrow_mut().take() {
            timer.invalidate();
        }
        if let Some(link) = self.ivars().link.borrow().as_ref() {
            link.setPaused(false);
        }
    }

    fn go_idle(&self) {
        if let Some(link) = self.ivars().link.borrow().as_ref() {
            link.setPaused(true);
        }
        if self.ivars().probe.borrow().is_some() || self.window().is_none() {
            return;
        }
        let timer = unsafe {
            NSTimer::timerWithTimeInterval_target_selector_userInfo_repeats(
                PROBE_INTERVAL,
                self,
                sel!(probe:),
                None,
                true,
            )
        };
        unsafe {
            objc2_foundation::NSRunLoop::mainRunLoop().addTimer_forMode(&timer, objc2_foundation::NSRunLoopCommonModes)
        };
        *self.ivars().probe.borrow_mut() = Some(timer);
    }

    fn start_link(&self) {
        if self.ivars().link.borrow().is_some() {
            self.wake();
            return;
        }
        // The link retains its target (this view) until `stop_link` invalidates it.
        let link = unsafe { self.displayLinkWithTarget_selector(self, sel!(tick:)) };
        unsafe {
            link.addToRunLoop_forMode(
                &objc2_foundation::NSRunLoop::mainRunLoop(),
                objc2_foundation::NSRunLoopCommonModes,
            )
        };
        *self.ivars().link.borrow_mut() = Some(link);
        self.wake();
    }

    pub(crate) fn stop_link(&self) {
        if let Some(link) = self.ivars().link.borrow_mut().take() {
            link.invalidate();
        }
        if let Some(timer) = self.ivars().probe.borrow_mut().take() {
            timer.invalidate();
        }
    }
}
