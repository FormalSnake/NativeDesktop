//! The AppKit presenter: an `NSView` backed by a `CAMetalLayer`, drawn by `metal::Renderer`.
//!
//! The architecture puts GPU-only content on macOS in an embedded GPUI surface (3.8), but that
//! surface is a fork patch that has not landed; the terminal is a single GPU leaf, so it draws
//! with Metal directly and keeps every other rule for that case: it is an ordinary NSView among
//! its siblings, AppKit hit-testing and first responder reach it, it implements
//! `NSTextInputClient` for input methods, its frame comes with its siblings' and it presents with
//! the Core Animation transaction, so a live resize never shows a stale grid.

mod metal;

use std::cell::{Cell as StdCell, RefCell};
use std::collections::HashMap;

use dispatch2::DispatchQueue;
use nd_backend_appkit::custom::{CustomCx, CustomNative, CustomPaint, CustomWidget, Waker};
use nd_core::NodeId;
use nd_theme::Theme;
use objc2::rc::{Retained, Weak};
use objc2::runtime::{AnyObject, NSObjectProtocol, ProtocolObject, Sel};
use objc2::{AnyThread, DefinedClass, MainThreadOnly, Message, define_class, msg_send};
use objc2_app_kit::{
    NSCursor, NSEvent, NSEventModifierFlags, NSPasteboard, NSPasteboardTypeString, NSResponder, NSTextInputClient,
    NSTrackingArea, NSTrackingAreaOptions, NSView, NSViewLayerContentsRedrawPolicy,
};
use objc2_foundation::{
    MainThreadMarker, NSArray, NSAttributedString, NSAttributedStringKey, NSNotFound, NSPoint, NSRange, NSRangePointer,
    NSRect, NSSize, NSString, NSUInteger,
};
use objc2_metal::{MTLCommandBuffer, MTLCommandQueue, MTLDrawable, MTLPixelFormat, MTLTexture};
use objc2_quartz_core::{CALayer, CAMetalDrawable, CAMetalLayer};

use crate::element::{FocusChanged, LinkActivated, TerminalProps, TerminalWidget, emit_session_events};
use crate::input::{Key, KeyAction, KeyInput, Mods, MouseButton, key_for_mac_keycode, unshifted_us};
use crate::links::Link;
use crate::session::{GridSize, TerminalSession, WakerGuard};
use crate::theme::TerminalTheme;
use metal::{Fonts, Overlay, Renderer};

/// Point size when neither the element nor the theme names one, as on the GPUI presenter.
pub const DEFAULT_FONT_SIZE: f32 = 13.;

pub(crate) fn register() {
    nd_backend_appkit::custom::register::<TerminalWidget>(AppKitTerminal);
}

struct AppKitTerminal;

impl CustomWidget for AppKitTerminal {
    fn create(&self, cx: &CustomCx<'_>) -> Box<dyn CustomNative> {
        let props = cx.props().downcast_ref::<TerminalProps>().expect("terminal node needs TerminalProps").clone();
        let view = NdTerminalView::new(cx.mtm(), props, cx.theme(), cx.emitter(), cx.waker());
        Box::new(TerminalNative { view, node: cx.node() })
    }
}

struct TerminalNative {
    view: Retained<NdTerminalView>,
    #[allow(dead_code)]
    node: NodeId,
}

impl CustomNative for TerminalNative {
    fn view(&self) -> &NSView {
        &self.view
    }

    fn update(&mut self, cx: &CustomCx<'_>) {
        let props = cx.props().downcast_ref::<TerminalProps>().expect("terminal node needs TerminalProps").clone();
        self.view.set_props(props, cx.theme());
    }

    fn set_theme(&mut self, theme: &Theme) {
        self.view.set_theme(theme);
    }

    fn paints(&self) -> Vec<CustomPaint> {
        self.view.paints()
    }
}

thread_local! {
    /// Live terminal views by id, for the I/O threads' wakes, which arrive on the main queue.
    static VIEWS: RefCell<HashMap<u64, Weak<NdTerminalView>>> = RefCell::default();
    static NEXT_ID: StdCell<u64> = const { StdCell::new(1) };
}

/// What a view keeps; `None` only between alloc and init.
struct Term {
    session: TerminalSession,
    props: TerminalProps,
    tokens: Theme,
    theme: TerminalTheme,
    emit: nd_ui::events::Emit,
    wake: Waker,
    renderer: Option<Renderer>,
    fonts: Option<(FontKey, Fonts)>,
    guard: Option<WakerGuard>,
    marked: Option<String>,
    hover: Option<Link>,
    held: Option<MouseButton>,
    /// Text `insertText:` produced during the current `keyDown:`.
    typed: Option<String>,
    in_key_down: bool,
    focused: bool,
}

#[derive(Clone, Debug, PartialEq)]
struct FontKey {
    family: Option<String>,
    points: f32,
    scale: f64,
}

pub struct TermIvars {
    id: u64,
    term: RefCell<Option<Term>>,
    /// A draw is queued for the next main queue turn.
    scheduled: StdCell<bool>,
    presented: StdCell<u64>,
}

define_class!(
    // SAFETY: NSView has no subclassing requirements beyond calling super in the overrides
    // below, and this type does not implement Drop.
    #[unsafe(super(NSView, NSResponder))]
    #[thread_kind = MainThreadOnly]
    #[ivars = TermIvars]
    pub struct NdTerminalView;

    impl NdTerminalView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method_id(makeBackingLayer))]
        fn make_backing_layer(&self) -> Retained<CALayer> {
            let layer = CAMetalLayer::new();
            layer.setPixelFormat(MTLPixelFormat::BGRA8Unorm);
            layer.setFramebufferOnly(true);
            layer.setPresentsWithTransaction(true);
            if let Some(term) = self.ivars().term.borrow().as_ref()
                && let Some(renderer) = &term.renderer
            {
                layer.setDevice(Some(renderer.device()));
            }
            Retained::into_super(layer)
        }

        #[unsafe(method(wantsUpdateLayer))]
        fn wants_update_layer(&self) -> bool {
            true
        }

        #[unsafe(method(updateLayer))]
        fn update_layer(&self) {
            self.draw();
        }

        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool {
            true
        }

        #[unsafe(method(becomeFirstResponder))]
        fn become_first_responder(&self) -> bool {
            let ok: bool = unsafe { msg_send![super(self), becomeFirstResponder] };
            if ok {
                self.focus_changed(true);
            }
            ok
        }

        #[unsafe(method(resignFirstResponder))]
        fn resign_first_responder(&self) -> bool {
            let ok: bool = unsafe { msg_send![super(self), resignFirstResponder] };
            if ok {
                self.focus_changed(false);
            }
            ok
        }

        #[unsafe(method(mouseDownCanMoveWindow))]
        fn mouse_down_can_move_window(&self) -> bool {
            false
        }

        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            true
        }

        #[unsafe(method(setFrameSize:))]
        fn set_frame_size(&self, size: NSSize) {
            let _: () = unsafe { msg_send![super(self), setFrameSize: size] };
            self.layout_grid();
        }

        #[unsafe(method(viewDidChangeBackingProperties))]
        fn view_did_change_backing_properties(&self) {
            let _: () = unsafe { msg_send![super(self), viewDidChangeBackingProperties] };
            self.layout_grid();
        }

        #[unsafe(method(viewDidMoveToWindow))]
        fn view_did_move_to_window(&self) {
            let _: () = unsafe { msg_send![super(self), viewDidMoveToWindow] };
            self.layout_grid();
        }

        #[unsafe(method(updateTrackingAreas))]
        fn update_tracking_areas(&self) {
            let _: () = unsafe { msg_send![super(self), updateTrackingAreas] };
            for area in self.trackingAreas().iter() {
                self.removeTrackingArea(&area);
            }
            let options = NSTrackingAreaOptions::MouseMoved
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
        }

        #[unsafe(method(resetCursorRects))]
        fn reset_cursor_rects(&self) {
            self.addCursorRect_cursor(self.bounds(), &NSCursor::IBeamCursor());
        }

        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            if !self.key(event) {
                let _: () = unsafe { msg_send![super(self), keyDown: event] };
            }
        }

        #[unsafe(method(keyUp:))]
        fn key_up(&self, event: &NSEvent) {
            let Some(session) = self.session() else { return };
            let mods = mods_of(event.modifierFlags());
            let key = key_for_mac_keycode(event.keyCode());
            session.key(&KeyInput { action: KeyAction::Release, ..KeyInput::press(key, mods) });
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            if let Some(window) = self.window() {
                window.makeFirstResponder(Some(self));
            }
            self.mouse_press(event, MouseButton::Left);
        }

        #[unsafe(method(rightMouseDown:))]
        fn right_mouse_down(&self, event: &NSEvent) {
            self.mouse_press(event, MouseButton::Right);
        }

        #[unsafe(method(otherMouseDown:))]
        fn other_mouse_down(&self, event: &NSEvent) {
            self.mouse_press(event, MouseButton::Middle);
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, event: &NSEvent) {
            self.mouse_release(event, MouseButton::Left);
        }

        #[unsafe(method(rightMouseUp:))]
        fn right_mouse_up(&self, event: &NSEvent) {
            self.mouse_release(event, MouseButton::Right);
        }

        #[unsafe(method(otherMouseUp:))]
        fn other_mouse_up(&self, event: &NSEvent) {
            self.mouse_release(event, MouseButton::Middle);
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, event: &NSEvent) {
            self.mouse_motion(event);
        }

        #[unsafe(method(rightMouseDragged:))]
        fn right_mouse_dragged(&self, event: &NSEvent) {
            self.mouse_motion(event);
        }

        #[unsafe(method(otherMouseDragged:))]
        fn other_mouse_dragged(&self, event: &NSEvent) {
            self.mouse_motion(event);
        }

        #[unsafe(method(mouseMoved:))]
        fn mouse_moved(&self, event: &NSEvent) {
            self.mouse_motion(event);
        }

        #[unsafe(method(flagsChanged:))]
        fn flags_changed(&self, event: &NSEvent) {
            // Holding or releasing Cmd shows or hides the link under the pointer.
            if let Some(window) = self.window() {
                let at = self.convertPoint_fromView(window.mouseLocationOutsideOfEventStream(), None);
                self.update_hover(at, event.modifierFlags());
            }
        }

        #[unsafe(method(scrollWheel:))]
        fn scroll_wheel(&self, event: &NSEvent) {
            let Some((session, scale, cell_h)) = self.with_term(|t| {
                let (_, fonts) = t.fonts.as_ref()?;
                Some((t.session.clone(), self.scale(), fonts.cell_height as f32))
            }).flatten() else { return };
            let dy = event.scrollingDeltaY() as f32;
            let dy = if event.hasPreciseScrollingDeltas() { dy * scale as f32 } else { dy * cell_h };
            let (x, y) = self.local(event);
            session.scroll_pixels(dy, x, y, mods_of(event.modifierFlags()));
            self.after_input();
        }

        #[unsafe(method(copy:))]
        fn copy(&self, _sender: Option<&AnyObject>) {
            let Some(text) = self.session().and_then(|s| s.selection_text()) else { return };
            let pasteboard = NSPasteboard::generalPasteboard();
            pasteboard.clearContents();
            pasteboard.setString_forType(&NSString::from_str(&text), unsafe { NSPasteboardTypeString });
        }

        #[unsafe(method(paste:))]
        fn paste(&self, _sender: Option<&AnyObject>) {
            let pasteboard = NSPasteboard::generalPasteboard();
            let Some(text) = pasteboard.stringForType(unsafe { NSPasteboardTypeString }) else { return };
            if let Some(session) = self.session() {
                session.paste(&text.to_string());
                self.after_input();
            }
        }

        #[unsafe(method(selectAll:))]
        fn select_all(&self, _sender: Option<&AnyObject>) {
            if let Some(session) = self.session() {
                session.select_all();
                self.after_input();
            }
        }
    }

    unsafe impl NSObjectProtocol for NdTerminalView {}

    unsafe impl NSTextInputClient for NdTerminalView {
        #[unsafe(method(insertText:replacementRange:))]
        fn insert_text(&self, string: &AnyObject, _range: NSRange) {
            let text = string_of(string);
            let in_key_down = self.with_term(|t| {
                t.marked = None;
                if t.in_key_down {
                    t.typed.get_or_insert_with(String::new).push_str(&text);
                }
                t.in_key_down
            });
            if in_key_down == Some(false) && let Some(session) = self.session() {
                // Committed outside a key press: the emoji picker, dictation.
                session.commit_text(&text);
                self.after_input();
            }
            self.redraw();
        }

        #[unsafe(method(doCommandBySelector:))]
        fn do_command_by_selector(&self, _selector: Sel) {
            // Keys with no text (arrows, Return, Delete): `keyDown:` encodes them itself.
        }

        #[unsafe(method(setMarkedText:selectedRange:replacementRange:))]
        fn set_marked_text(&self, string: &AnyObject, _selected: NSRange, _replacement: NSRange) {
            let text = string_of(string);
            self.with_term(|t| t.marked = (!text.is_empty()).then_some(text));
            self.redraw();
        }

        #[unsafe(method(unmarkText))]
        fn unmark_text(&self) {
            self.with_term(|t| t.marked = None);
            self.redraw();
        }

        #[unsafe(method(selectedRange))]
        fn selected_range(&self) -> NSRange {
            let len = self.marked_len();
            NSRange::new(len, 0)
        }

        #[unsafe(method(markedRange))]
        fn marked_range(&self) -> NSRange {
            match self.marked_len() {
                0 => NSRange::new(NSNotFound as NSUInteger, 0),
                len => NSRange::new(0, len),
            }
        }

        #[unsafe(method(hasMarkedText))]
        fn has_marked_text(&self) -> bool {
            self.marked_len() > 0
        }

        #[unsafe(method_id(attributedSubstringForProposedRange:actualRange:))]
        fn attributed_substring(&self, _range: NSRange, _actual: NSRangePointer) -> Option<Retained<NSAttributedString>> {
            None
        }

        #[unsafe(method_id(validAttributesForMarkedText))]
        fn valid_attributes(&self) -> Retained<NSArray<NSAttributedStringKey>> {
            NSArray::new()
        }

        #[unsafe(method(firstRectForCharacterRange:actualRange:))]
        fn first_rect(&self, _range: NSRange, _actual: NSRangePointer) -> NSRect {
            // The candidate window opens under the cursor.
            let local = self.cursor_rect().unwrap_or(NSRect::ZERO);
            let in_window = self.convertRect_toView(local, None);
            self.window().map_or(NSRect::ZERO, |w| w.convertRectToScreen(in_window))
        }

        #[unsafe(method(characterIndexForPoint:))]
        fn character_index_for_point(&self, _point: NSPoint) -> NSUInteger {
            NSNotFound as NSUInteger
        }
    }
);

fn string_of(object: &AnyObject) -> String {
    if let Some(attributed) = object.downcast_ref::<NSAttributedString>() {
        return attributed.string().to_string();
    }
    object.downcast_ref::<NSString>().map(NSString::to_string).unwrap_or_default()
}

pub(crate) fn mods_of(flags: NSEventModifierFlags) -> Mods {
    let mut mods = Mods::empty();
    if flags.contains(NSEventModifierFlags::Shift) {
        mods |= Mods::SHIFT;
    }
    if flags.contains(NSEventModifierFlags::Control) {
        mods |= Mods::CTRL;
    }
    if flags.contains(NSEventModifierFlags::Option) {
        mods |= Mods::ALT;
    }
    if flags.contains(NSEventModifierFlags::Command) {
        mods |= Mods::SUPER;
    }
    if flags.contains(NSEventModifierFlags::CapsLock) {
        mods |= Mods::CAPS_LOCK;
    }
    mods
}

impl NdTerminalView {
    fn new(
        mtm: MainThreadMarker,
        props: TerminalProps,
        tokens: &Theme,
        emit: nd_ui::events::Emit,
        wake: Waker,
    ) -> Retained<Self> {
        let id = NEXT_ID.with(|n| {
            let id = n.get();
            n.set(id + 1);
            id
        });
        let theme = TerminalTheme::from_theme(tokens, &props.colors);
        props.session.set_theme(&theme);
        let term = Term {
            session: props.session.clone(),
            props,
            tokens: tokens.clone(),
            theme,
            emit,
            wake,
            renderer: Renderer::new(),
            fonts: None,
            guard: None,
            marked: None,
            hover: None,
            held: None,
            typed: None,
            in_key_down: false,
            focused: false,
        };
        let this = Self::alloc(mtm).set_ivars(TermIvars {
            id,
            term: RefCell::new(Some(term)),
            scheduled: StdCell::new(false),
            presented: StdCell::new(0),
        });
        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] };
        this.setWantsLayer(true);
        this.setLayerContentsRedrawPolicy(NSViewLayerContentsRedrawPolicy::DuringViewResize);
        VIEWS.with(|v| v.borrow_mut().insert(id, Weak::from_retained(&this)));
        this.watch_output();
        this
    }

    fn with_term<R>(&self, f: impl FnOnce(&mut Term) -> R) -> Option<R> {
        self.ivars().term.borrow_mut().as_mut().map(f)
    }

    fn session(&self) -> Option<TerminalSession> {
        self.with_term(|t| t.session.clone())
    }

    fn scale(&self) -> f64 {
        self.window().map_or(2., |w| w.backingScaleFactor())
    }

    fn marked_len(&self) -> NSUInteger {
        self.with_term(|t| t.marked.as_deref().map_or(0, |m| m.encode_utf16().count())).unwrap_or(0)
    }

    /// Redraws when the session has output: the I/O thread wakes the main queue, which parses
    /// it and schedules a display.
    fn watch_output(&self) {
        let id = self.ivars().id;
        let Some(session) = self.session() else { return };
        let guard = session.on_output(move || {
            DispatchQueue::main().exec_async(move || {
                let view = VIEWS.with(|v| v.borrow().get(&id).and_then(Weak::load));
                if let Some(view) = view {
                    view.output_ready();
                }
            });
        });
        self.with_term(|t| t.guard = Some(guard));
    }

    /// Frames this view has presented, for tests.
    pub fn frames_presented(&self) -> u64 {
        self.ivars().presented.get()
    }

    /// Draws on the next main queue turn, once however many requests arrive before it. The
    /// layer is drawn directly rather than through `setNeedsDisplay`: Core Animation does not
    /// ask a `CAMetalLayer` to display, and a grid that changes faster than the display stalls
    /// on `nextDrawable` instead of queueing frames.
    fn redraw(&self) {
        if self.ivars().scheduled.replace(true) {
            return;
        }
        let id = self.ivars().id;
        DispatchQueue::main().exec_async(move || {
            let view = VIEWS.with(|v| v.borrow().get(&id).and_then(Weak::load));
            if let Some(view) = view {
                view.ivars().scheduled.set(false);
                view.draw();
            }
        });
    }

    fn output_ready(&self) {
        self.pump();
        self.redraw();
    }

    /// Parses waiting output and delivers the session's events. True when more is waiting.
    fn pump(&self) -> bool {
        let Some((session, emit, wake)) = self.with_term(|t| (t.session.clone(), t.emit.clone(), t.wake.clone()))
        else {
            return false;
        };
        let more = session.pump();
        if emit_session_events(&session, &emit) {
            wake.wake();
        }
        more
    }

    fn after_input(&self) {
        if let Some((session, emit, wake)) = self.with_term(|t| (t.session.clone(), t.emit.clone(), t.wake.clone()))
            && emit_session_events(&session, &emit)
        {
            wake.wake();
        }
        self.redraw();
    }

    fn set_props(&self, props: TerminalProps, tokens: &Theme) {
        let session_changed = self.with_term(|t| {
            let changed = t.session != props.session;
            t.session = props.session.clone();
            t.props = props;
            t.tokens = tokens.clone();
            t.theme = TerminalTheme::from_theme(tokens, &t.props.colors);
            t.session.set_theme(&t.theme);
            changed
        });
        if session_changed == Some(true) {
            self.with_term(|t| t.guard = None);
            self.watch_output();
        }
        self.layout_grid();
        self.redraw();
    }

    fn set_theme(&self, tokens: &Theme) {
        self.with_term(|t| {
            t.tokens = tokens.clone();
            t.theme = TerminalTheme::from_theme(tokens, &t.props.colors);
            t.session.set_theme(&t.theme);
        });
        self.redraw();
    }

    fn paints(&self) -> Vec<CustomPaint> {
        self.with_term(|t| {
            let size = t.props.font_size.unwrap_or(DEFAULT_FONT_SIZE);
            t.session
                .frame(|f| crate::paint::marks(f, &t.theme, size))
                .into_iter()
                .map(|m| CustomPaint { label: m.label, color: m.color, under: m.under, text: m.text })
                .collect()
        })
        .unwrap_or_default()
    }

    /// The faces for the current props and backing scale, rebuilt when either changed.
    fn ensure_fonts(&self, t: &mut Term) {
        let key = FontKey {
            family: t.props.font_family.as_ref().map(|f| f.to_string()),
            points: t.props.font_size.unwrap_or(DEFAULT_FONT_SIZE),
            scale: self.scale(),
        };
        if t.fonts.as_ref().is_some_and(|(k, _)| *k == key) {
            return;
        }
        let family = key.family.clone().or_else(|| Some(t.tokens.fonts.mono.family.to_string()));
        let fonts = Fonts::new(family.as_deref(), f64::from(key.points), key.scale);
        t.fonts = Some((key, fonts));
    }

    /// Fits the grid to the view's size and tells the session.
    fn layout_grid(&self) {
        let bounds = self.bounds();
        let scale = self.scale();
        self.with_term(|t| {
            self.ensure_fonts(t);
            let Some((_, fonts)) = &t.fonts else { return };
            let pad = f64::from(t.props.padding) * scale;
            let w = (bounds.size.width * scale - 2. * pad).max(0.);
            let h = (bounds.size.height * scale - 2. * pad).max(0.);
            let cols = (w / f64::from(fonts.cell_width)).floor().max(1.) as u16;
            let rows = (h / f64::from(fonts.cell_height)).floor().max(1.) as u16;
            t.session.resize(GridSize { cols, rows, cell_width: fonts.cell_width, cell_height: fonts.cell_height });
        });
        if let Some(layer) = self.layer().and_then(|l| l.downcast::<CAMetalLayer>().ok()) {
            layer.setContentsScale(scale);
            layer.setDrawableSize(objc2_core_foundation::CGSize::new(
                bounds.size.width * scale,
                bounds.size.height * scale,
            ));
        }
        // Inside the resize's layout pass: this frame presents with the new size.
        if self.window().is_some() {
            self.draw();
        }
    }

    fn draw(&self) {
        let more = self.pump();
        let scale = self.scale();
        let bounds = self.bounds();
        if bounds.size.width < 1. || bounds.size.height < 1. {
            return;
        }
        let Some(layer) = self.layer().and_then(|l| l.downcast::<CAMetalLayer>().ok()) else { return };
        let mut term = self.ivars().term.borrow_mut();
        let Some(t) = term.as_mut() else { return };
        self.ensure_fonts(t);
        let (Some(renderer), Some((_, fonts))) = (t.renderer.as_mut(), t.fonts.as_ref()) else { return };
        if layer.device().is_none() {
            layer.setDevice(Some(renderer.device()));
        }
        let pad = t.props.padding * scale as f32;
        let overlay = Overlay { marked: t.marked.as_deref(), hover: t.hover.as_ref() };
        let theme = t.theme.clone();
        t.session.frame(|frame| renderer.build(frame, &theme, fonts, (pad, pad), &overlay));
        layer.setOpaque(theme.background.a >= 1.);
        let Some(drawable) = layer.nextDrawable() else { return };
        let Some(command_buffer) = renderer.queue().commandBuffer() else { return };
        renderer.encode(&command_buffer, &drawable.texture(), theme.background);
        command_buffer.commit();
        // The layer presents with the Core Animation transaction this display pass is in.
        command_buffer.waitUntilScheduled();
        drawable.present();
        self.ivars().presented.set(self.ivars().presented.get() + 1);
        drop(term);
        if more {
            self.redraw();
        }
    }

    fn focus_changed(&self, focused: bool) {
        let Some((session, emit, wake)) = self.with_term(|t| {
            t.focused = focused;
            (t.session.clone(), t.emit.clone(), t.wake.clone())
        }) else {
            return;
        };
        session.set_focused(focused);
        emit.emit(FocusChanged { focused });
        wake.wake();
        self.redraw();
    }

    /// Device pixels from the grid's top left.
    fn local(&self, event: &NSEvent) -> (f32, f32) {
        let at = self.convertPoint_fromView(event.locationInWindow(), None);
        self.local_point(at)
    }

    fn local_point(&self, at: NSPoint) -> (f32, f32) {
        let scale = self.scale();
        let pad = self.with_term(|t| f64::from(t.props.padding)).unwrap_or(0.);
        (((at.x - pad) * scale) as f32, ((at.y - pad) * scale) as f32)
    }

    fn cell_at(&self, at: NSPoint) -> Option<(u16, u16)> {
        let (x, y) = self.local_point(at);
        self.with_term(|t| {
            let (_, fonts) = t.fonts.as_ref()?;
            let size = t.session.size();
            let col = (x / fonts.cell_width as f32).floor().clamp(0., f32::from(size.cols.saturating_sub(1))) as u16;
            let row = (y / fonts.cell_height as f32).floor().clamp(0., f32::from(size.rows.saturating_sub(1))) as u16;
            Some((col, row))
        })
        .flatten()
    }

    fn cursor_rect(&self) -> Option<NSRect> {
        let scale = self.scale();
        self.with_term(|t| {
            let (_, fonts) = t.fonts.as_ref()?;
            let cursor = t.session.frame(|f| f.cursor)?;
            let pad = f64::from(t.props.padding);
            let (cw, ch) = (f64::from(fonts.cell_width) / scale, f64::from(fonts.cell_height) / scale);
            Some(NSRect::new(
                NSPoint::new(pad + f64::from(cursor.col) * cw, pad + f64::from(cursor.row) * ch),
                NSSize::new(cw, ch),
            ))
        })
        .flatten()
    }

    fn mouse_press(&self, event: &NSEvent, button: MouseButton) {
        let Some(session) = self.session() else { return };
        let flags = event.modifierFlags();
        let at = self.convertPoint_fromView(event.locationInWindow(), None);
        if button == MouseButton::Left
            && flags.contains(NSEventModifierFlags::Command)
            && let Some((col, row)) = self.cell_at(at)
            && let Some(link) = session.link_at(col, row)
        {
            self.with_term(|t| {
                t.emit.emit(LinkActivated { link });
                t.wake.wake();
            });
            return;
        }
        self.with_term(|t| t.held = Some(button));
        let (x, y) = self.local(event);
        session.mouse_down(button, x, y, mods_of(flags));
        self.after_input();
    }

    fn mouse_release(&self, event: &NSEvent, button: MouseButton) {
        let Some(session) = self.session() else { return };
        if self.with_term(|t| t.held.take()).flatten() != Some(button) {
            return;
        }
        let (x, y) = self.local(event);
        session.mouse_up(button, x, y, mods_of(event.modifierFlags()));
        self.after_input();
    }

    fn mouse_motion(&self, event: &NSEvent) {
        let Some(session) = self.session() else { return };
        let at = self.convertPoint_fromView(event.locationInWindow(), None);
        self.update_hover(at, event.modifierFlags());
        let held = self.with_term(|t| t.held).flatten();
        let (x, y) = self.local(event);
        session.mouse_moved(x, y, held, mods_of(event.modifierFlags()));
        self.after_input();
    }

    fn update_hover(&self, at: NSPoint, flags: NSEventModifierFlags) {
        let hover = if flags.contains(NSEventModifierFlags::Command) {
            self.cell_at(at).and_then(|(c, r)| self.session().and_then(|s| s.link_at(c, r)))
        } else {
            None
        };
        let changed = self.with_term(|t| {
            let changed = t.hover != hover;
            t.hover = hover.clone();
            changed
        });
        if changed == Some(true) {
            self.redraw();
            let cursor = if hover.is_some() { NSCursor::pointingHandCursor() } else { NSCursor::IBeamCursor() };
            cursor.set();
        }
    }

    /// A key-down for the program. False passes it on (Cmd shortcuts reach the menus that way).
    fn key(&self, event: &NSEvent) -> bool {
        let Some(session) = self.session() else { return false };
        let flags = event.modifierFlags();
        let mods = mods_of(flags);
        if mods.contains(Mods::SUPER) {
            return false;
        }
        let key = key_for_mac_keycode(event.keyCode());
        let action = if event.isARepeat() { KeyAction::Repeat } else { KeyAction::Press };
        let unshifted = event
            .charactersByApplyingModifiers(NSEventModifierFlags::empty())
            .and_then(|s| s.to_string().chars().next())
            .filter(|c| !c.is_control())
            .unwrap_or_else(|| unshifted_us(key));
        let was_marked = self.marked_len() > 0;
        // Ctrl chords go straight to the encoder unless an input method is composing: AppKit's
        // key bindings would turn Ctrl-A into a selector.
        if mods.contains(Mods::CTRL) && !was_marked {
            let input = KeyInput { action, unshifted, ..KeyInput::press(key, mods) };
            session.key(&input);
            self.after_input();
            return true;
        }
        self.with_term(|t| {
            t.in_key_down = true;
            t.typed = None;
        });
        let events = NSArray::from_retained_slice(&[event.retain()]);
        self.interpretKeyEvents(&events);
        let typed = self
            .with_term(|t| {
                t.in_key_down = false;
                t.typed.take()
            })
            .flatten();
        let marked = self.marked_len() > 0;
        match typed {
            Some(text) => {
                let single = text.chars().count() == 1;
                if single && key != Key::Unidentified && !was_marked {
                    let consumed = mods & (Mods::SHIFT | Mods::ALT);
                    session.key(&KeyInput { action, unshifted, consumed, ..KeyInput::typed(key, mods, text) });
                } else {
                    session.commit_text(&text);
                }
            }
            // The input method took the key.
            None if was_marked || marked => {}
            None => {
                session.key(&KeyInput { action, unshifted, ..KeyInput::press(key, mods) });
            }
        }
        self.after_input();
        true
    }
}

impl Drop for TermIvars {
    fn drop(&mut self) {
        VIEWS.with(|v| {
            if let Ok(mut v) = v.try_borrow_mut() {
                v.remove(&self.id);
            }
        });
    }
}

/// The view's renderer without the view: draws a session into an offscreen texture exactly as
/// the view presents it, for tier 2 goldens and the scroll budget, with no window.
pub struct Offscreen {
    renderer: Renderer,
    fonts: Fonts,
    theme: TerminalTheme,
    padding: f32,
    target: Retained<ProtocolObject<dyn objc2_metal::MTLTexture>>,
}

impl Offscreen {
    /// A `size` (points) surface at `scale`, in `tokens`' colours and monospace face.
    pub fn new(tokens: &Theme, font_size: f32, padding: f32, size: (f64, f64), scale: f64) -> Option<Self> {
        let renderer = Renderer::new()?;
        let fonts = Fonts::new(Some(&tokens.fonts.mono.family), f64::from(font_size), scale);
        let target = renderer.target((size.0 * scale) as u32, (size.1 * scale) as u32)?;
        let theme = TerminalTheme::from_theme(tokens, &Default::default());
        Some(Self { renderer, fonts, theme, padding: padding * scale as f32, target })
    }

    /// Themes `session` and fits its grid to the surface, as the view does on a resize.
    pub fn fit(&self, session: &TerminalSession) {
        session.set_theme(&self.theme);
        let pad = f64::from(self.padding);
        let w = self.target.width() as f64 - 2. * pad;
        let h = self.target.height() as f64 - 2. * pad;
        let cols = (w / f64::from(self.fonts.cell_width)).floor().max(1.) as u16;
        let rows = (h / f64::from(self.fonts.cell_height)).floor().max(1.) as u16;
        session.resize(GridSize { cols, rows, cell_width: self.fonts.cell_width, cell_height: self.fonts.cell_height });
    }

    /// One frame: the snapshot, the instances and the GPU pass, waited for.
    pub fn draw(&mut self, session: &TerminalSession) {
        let overlay = Overlay { marked: None, hover: None };
        let pad = self.padding;
        session.frame(|frame| self.renderer.build(frame, &self.theme, &self.fonts, (pad, pad), &overlay));
        self.renderer.draw_to(&self.target, self.theme.background);
    }

    /// Draws a frame and reads it back as RGBA rows.
    pub fn image(&mut self, session: &TerminalSession) -> (u32, u32, Vec<u8>) {
        self.draw(session);
        Renderer::read(&self.target)
    }
}
