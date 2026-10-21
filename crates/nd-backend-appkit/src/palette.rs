//! `command_palette()` on AppKit: the zig branch's Spotlight-style palette
//! (`NDShell/CommandPalette.swift`), ported to objc2. The node's own view is a hidden zero-size
//! handle; a present opens the overlay layer over the window (`crate::overlay::WindowLayer`, a
//! key child panel covering the whole frame, toolbar and sidebar included, above every view and
//! native surface) and puts a dimmed backdrop in it holding a Liquid Glass card: a borderless field over an inset
//! `NSTableView` of one-line rows. The card's top edge is fixed and its height follows the rows.
//!
//! Behaviour is [`PaletteModel`]'s; this module draws it and feeds it the field's edits, the
//! field editor's commands and the table's clicks. Focus stays in the field: the highlighted row
//! draws AppKit's emphasized selection all the same.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use nd_components::{
    CommandPaletteProps, FieldSelection, PALETTE, PaletteEvent, PaletteItem, PaletteKey, PaletteModel,
    emit_palette_event,
};
use nd_ui::events::Emit;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject, Sel};
use objc2::{AnyThread, DefinedClass, MainThreadOnly, Message, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAccessibility, NSAccessibilityGroupRole, NSAnimatablePropertyContainer, NSAnimationContext, NSApplication,
    NSAutoresizingMaskOptions, NSBackgroundStyle, NSBox, NSBoxType, NSColor, NSControl, NSControlTextEditingDelegate,
    NSEvent, NSEventModifierFlags, NSEventType, NSFocusRingType, NSFont, NSFontWeightRegular, NSGlassEffectView,
    NSImage, NSImageScaling, NSImageSymbolConfiguration, NSImageView, NSLineBreakMode, NSResponder, NSScrollView,
    NSTableCellView, NSTableColumn, NSTableColumnResizingOptions, NSTableRowView, NSTableView,
    NSTableViewColumnAutoresizingStyle, NSTableViewDataSource, NSTableViewDelegate, NSTableViewStyle, NSTextField,
    NSTextFieldDelegate, NSTextView, NSUserInterfaceItemIdentification, NSView, NSVisualEffectBlendingMode,
    NSVisualEffectMaterial, NSVisualEffectState, NSVisualEffectView, NSWindow,
};
use objc2_core_foundation::CGSize;
use objc2_core_graphics::CGPath;
use objc2_foundation::{
    MainThreadMarker, NSData, NSIndexSet, NSInteger, NSNotification, NSObject, NSObjectProtocol, NSPoint, NSRange,
    NSRect, NSSize, NSString,
};

use crate::events::{Wake, defer};

/// A label's untruncated size: a truncating label reports no useful intrinsic width.
pub(crate) fn natural_size(field: &NSTextField) -> NSSize {
    field.cell().map_or_else(|| field.intrinsicContentSize(), |c| c.cellSize())
}

fn utf16(s: &str) -> usize {
    s.encode_utf16().count()
}

pub struct PaletteIvars {
    palette: RefCell<Weak<Palette>>,
}

define_class!(
    /// Full-window scrim behind the card; a click on it cancels. Owns the secondary-Return
    /// "submit as typed" key equivalent, which never reaches the field editor's commands.
    // SAFETY: NSView has no subclassing requirements beyond calling super; no Drop.
    #[unsafe(super(NSView, NSResponder))]
    #[thread_kind = MainThreadOnly]
    #[ivars = PaletteIvars]
    pub struct NdPaletteBackdrop;

    impl NdPaletteBackdrop {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, _event: &NSEvent) {
            if let Some(p) = self.ivars().palette.borrow().upgrade() {
                p.user_cancel();
            }
        }

        #[unsafe(method(performKeyEquivalent:))]
        fn perform_key_equivalent(&self, event: &NSEvent) -> bool {
            if self.submit_key(event) { true } else { unsafe { msg_send![super(self), performKeyEquivalent: event] } }
        }

        #[unsafe(method(layout))]
        fn layout(&self) {
            let _: () = unsafe { msg_send![super(self), layout] };
            if let Some(p) = self.ivars().palette.borrow().upgrade() {
                p.layout();
            }
        }
    }
);

impl NdPaletteBackdrop {
    /// Secondary-Return (and keypad Enter) submits the typed text.
    fn submit_key(&self, event: &NSEvent) -> bool {
        let flags = event.modifierFlags();
        let secondary = flags.contains(NSEventModifierFlags::Command) || flags.contains(NSEventModifierFlags::Control);
        if event.r#type() != NSEventType::KeyDown || !secondary || !matches!(event.keyCode(), 36 | 76) {
            return false;
        }
        if let Some(p) = self.ivars().palette.borrow().upgrade() {
            p.key(PaletteKey::SecondaryReturn);
        }
        true
    }
}

define_class!(
    /// Shadow host for the glass card: the glass clips to its own corner radius, so the shadow
    /// lives one level out, its path rebuilt as the card's height follows the rows. Swallows
    /// clicks on the card's own chrome so they never reach the backdrop's cancel.
    // SAFETY: as above.
    #[unsafe(super(NSView, NSResponder))]
    #[thread_kind = MainThreadOnly]
    pub struct NdPaletteCard;

    impl NdPaletteCard {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(layout))]
        fn layout(&self) {
            let _: () = unsafe { msg_send![super(self), layout] };
            if let Some(layer) = self.layer() {
                let r = f64::from(PALETTE.radius);
                let path = unsafe { CGPath::with_rounded_rect(self.bounds(), r, r, std::ptr::null()) };
                layer.setShadowPath(Some(&path));
            }
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, _event: &NSEvent) {}
    }
);

define_class!(
    /// Focus stays in the field, which leaves the table unemphasized and its selection grey.
    /// The highlighted row is the palette's main affordance, so the row draws AppKit's
    /// emphasized selection (accent colour, inset and curvature of the table's style) anyway.
    // SAFETY: as above.
    #[unsafe(super(NSTableRowView, NSView, NSResponder))]
    #[thread_kind = MainThreadOnly]
    pub struct NdPaletteRowView;

    impl NdPaletteRowView {
        #[unsafe(method(isEmphasized))]
        fn is_emphasized(&self) -> bool {
            true
        }

        #[unsafe(method(setEmphasized:))]
        fn set_emphasized(&self, _emphasized: bool) {}
    }
);

pub struct CellIvars {
    icon: Retained<NSImageView>,
    title: Retained<NSTextField>,
    subtitle: Retained<NSTextField>,
    hint: Retained<NSTextField>,
}

define_class!(
    /// One result row on one line: an icon slot that holds its column even when empty, the
    /// title, the subtitle in secondary text after it, and the hint at the trailing edge. The
    /// subtitle gives way first, then the title; the hint never truncates.
    // SAFETY: as above.
    #[unsafe(super(NSTableCellView, NSView, NSResponder))]
    #[thread_kind = MainThreadOnly]
    #[ivars = CellIvars]
    pub struct NdPaletteCell;

    impl NdPaletteCell {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(layout))]
        fn layout(&self) {
            let _: () = unsafe { msg_send![super(self), layout] };
            self.place();
        }

        // NSTableCellView recolours its `textField` over the emphasized fill but knows nothing
        // of the subtitle, the hint or the symbol.
        #[unsafe(method(setBackgroundStyle:))]
        fn set_background_style(&self, style: NSBackgroundStyle) {
            let _: () = unsafe { msg_send![super(self), setBackgroundStyle: style] };
            self.recolour(style);
        }
    }
);

impl NdPaletteCell {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let label = || {
            let f = NSTextField::labelWithString(&NSString::new(), mtm);
            f.setFont(Some(&NSFont::systemFontOfSize(NSFont::systemFontSize())));
            f.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
            f.setMaximumNumberOfLines(1);
            if let Some(cell) = f.cell() {
                cell.setUsesSingleLineMode(true);
            }
            f
        };
        let ivars = CellIvars { icon: NSImageView::new(mtm), title: label(), subtitle: label(), hint: label() };
        let this = Self::alloc(mtm).set_ivars(ivars);
        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] };
        let iv = this.ivars();
        iv.subtitle.setTextColor(Some(&NSColor::secondaryLabelColor()));
        iv.hint.setTextColor(Some(&NSColor::secondaryLabelColor()));
        iv.hint.setAlignment(objc2_app_kit::NSTextAlignment::Right);
        iv.icon.setContentTintColor(Some(&NSColor::secondaryLabelColor()));
        for v in [&*iv.icon as &NSView, &iv.title, &iv.subtitle, &iv.hint] {
            this.addSubview(v);
        }
        unsafe { this.setTextField(Some(&iv.title)) };
        this
    }

    fn configure(&self, row: &PaletteItem) {
        let iv = self.ivars();
        iv.title.setStringValue(&NSString::from_str(&row.title));
        let sub = row.subtitle.as_deref().unwrap_or("");
        iv.subtitle.setStringValue(&NSString::from_str(sub));
        iv.subtitle.setHidden(sub.is_empty());
        let hint = row.hint.as_deref().unwrap_or("");
        iv.hint.setStringValue(&NSString::from_str(hint));
        iv.hint.setHidden(hint.is_empty());
        let (image, scaling) = match (&row.icon_bytes, row.icon) {
            (Some(bytes), _) if !bytes.is_empty() => {
                let image = NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(bytes));
                (image, NSImageScaling::ScaleProportionallyUpOrDown)
            }
            (_, Some(icon)) => {
                let config =
                    NSImageSymbolConfiguration::configurationWithPointSize_weight(NSFont::systemFontSize(), unsafe {
                        NSFontWeightRegular
                    });
                let image = crate::icons::symbol(icon.lucide()).and_then(|i| i.imageWithSymbolConfiguration(&config));
                (image, NSImageScaling::ScaleNone)
            }
            _ => (None, NSImageScaling::ScaleNone),
        };
        iv.icon.setImageScaling(scaling);
        iv.icon.setHidden(image.is_none());
        iv.icon.setImage(image.as_deref());
        self.setAccessibilityLabel(Some(&NSString::from_str(&row.accessible_label())));
        // A recycled cell keeps the colours its previous row had.
        self.recolour(self.backgroundStyle());
        self.setNeedsLayout(true);
    }

    fn recolour(&self, style: NSBackgroundStyle) {
        let iv = self.ivars();
        let on_fill = style == NSBackgroundStyle::Emphasized;
        let secondary = if on_fill {
            NSColor::alternateSelectedControlTextColor().colorWithAlphaComponent(0.8)
        } else {
            NSColor::secondaryLabelColor()
        };
        iv.subtitle.setTextColor(Some(&secondary));
        iv.hint.setTextColor(Some(&secondary));
        let tint = if on_fill { NSColor::alternateSelectedControlTextColor() } else { NSColor::secondaryLabelColor() };
        iv.icon.setContentTintColor(Some(&tint));
    }

    /// Lays the row out by the priorities above. Every label uses the same font, so centring
    /// them vertically lines their baselines up.
    fn place(&self) {
        let iv = self.ivars();
        let b = self.bounds();
        let (w, h) = (b.size.width, b.size.height);
        let m = PALETTE;
        let inset = f64::from(m.cell_inset);
        let side = f64::from(m.icon_side);
        iv.icon.setFrame(NSRect::new(NSPoint::new(inset, (h - side) / 2.), NSSize::new(side, side)));
        let x0 = inset + side + f64::from(m.cell_gap);
        let natural = |f: &NSTextField| natural_size(f);
        let line = natural(&iv.title).height;
        let y = (h - line) / 2.;
        let mut text_end = w - inset;
        if !iv.hint.isHidden() {
            let hw = natural(&iv.hint).width.ceil();
            let hx = w - inset - hw;
            iv.hint.setFrame(NSRect::new(NSPoint::new(hx, y), NSSize::new(hw, line)));
            text_end = hx - f64::from(m.hint_gap);
        }
        let avail = (text_end - x0).max(0.);
        let tw = natural(&iv.title).width.ceil().min(f64::from(m.title_share) * w).min(avail);
        iv.title.setFrame(NSRect::new(NSPoint::new(x0, y), NSSize::new(tw, line)));
        if !iv.subtitle.isHidden() {
            let sx = x0 + tw + f64::from(m.subtitle_gap);
            let sw = natural(&iv.subtitle).width.ceil().min((text_end - sx).max(0.));
            iv.subtitle.setFrame(NSRect::new(NSPoint::new(sx, y), NSSize::new(sw, line)));
        }
    }

    /// A shown label narrower than its text.
    pub(crate) fn is_truncated(&self) -> bool {
        let iv = self.ivars();
        [&iv.title, &iv.subtitle].iter().any(|f| !f.isHidden() && natural_size(f).width > f.frame().size.width + 0.5)
    }
}

define_class!(
    /// The field's delegate, the table's data source and delegate, and the row click target.
    // SAFETY: NSObject has no subclassing requirements; no Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = PaletteIvars]
    pub struct NdPaletteSource;

    unsafe impl NSObjectProtocol for NdPaletteSource {}

    unsafe impl NSControlTextEditingDelegate for NdPaletteSource {
        #[unsafe(method(controlTextDidChange:))]
        fn control_text_did_change(&self, notification: &NSNotification) {
            let Some(p) = self.ivars().palette.borrow().upgrade() else { return };
            let Some(field) = notification.object().and_then(|o| o.downcast::<NSTextField>().ok()) else { return };
            p.user_edited(&field.stringValue().to_string());
        }

        #[unsafe(method(control:textView:doCommandBySelector:))]
        fn do_command(&self, _control: &NSControl, _text_view: &NSTextView, command: Sel) -> bool {
            self.command(command)
        }
    }

    unsafe impl NSTextFieldDelegate for NdPaletteSource {}

    unsafe impl NSTableViewDataSource for NdPaletteSource {
        #[unsafe(method(numberOfRowsInTableView:))]
        fn number_of_rows(&self, _table: &NSTableView) -> NSInteger {
            self.ivars().palette.borrow().upgrade().map_or(0, |p| p.model.borrow().rows().len() as NSInteger)
        }
    }

    unsafe impl NSTableViewDelegate for NdPaletteSource {
        #[unsafe(method_id(tableView:viewForTableColumn:row:))]
        fn view_for(&self, table: &NSTableView, _column: Option<&NSTableColumn>, row: NSInteger) -> Option<Retained<NSView>> {
            self.cell_for(table, row)
        }

        #[unsafe(method_id(tableView:rowViewForRow:))]
        fn row_view_for(&self, table: &NSTableView, _row: NSInteger) -> Option<Retained<NSTableRowView>> {
            let mtm = MainThreadMarker::from(table);
            let id = NSString::from_str(ROW_ID);
            let reused = unsafe { table.makeViewWithIdentifier_owner(&id, Some(self)) }
                .and_then(|v| v.downcast::<NSTableRowView>().ok());
            let row = reused.unwrap_or_else(|| {
                let row: Retained<NdPaletteRowView> = unsafe { msg_send![NdPaletteRowView::alloc(mtm), init] };
                row.setIdentifier(Some(&id));
                Retained::into_super(row)
            });
            // The card is glass: no opaque plate behind the rows.
            row.setBackgroundColor(&NSColor::clearColor());
            Some(row)
        }
    }

    impl NdPaletteSource {
        #[unsafe(method(rowClicked:))]
        fn row_clicked(&self, sender: &NSTableView) {
            let Some(p) = self.ivars().palette.borrow().upgrade() else { return };
            let row = sender.clickedRow();
            if row >= 0 {
                p.click(row as usize);
            }
        }
    }
);

impl NdPaletteSource {
    /// A field editor command the palette takes: the keys of [`PaletteKey`].
    fn command(&self, command: Sel) -> bool {
        let Some(p) = self.ivars().palette.borrow().upgrade() else { return false };
        let key = if command == sel!(moveUp:) {
            PaletteKey::Up
        } else if command == sel!(moveDown:) {
            PaletteKey::Down
        } else if command == sel!(moveToBeginningOfDocument:) {
            PaletteKey::Home
        } else if command == sel!(moveToEndOfDocument:) {
            PaletteKey::End
        } else if command == sel!(insertTab:) {
            PaletteKey::Tab
        } else if command == sel!(insertBacktab:) {
            PaletteKey::BackTab
        } else if command == sel!(moveRight:) || command == sel!(moveToEndOfLine:) {
            PaletteKey::Right
        } else if command == sel!(insertNewline:) {
            PaletteKey::Return
        } else if command == sel!(cancelOperation:) {
            PaletteKey::Escape
        } else {
            return false;
        };
        p.key(key)
    }

    fn cell_for(&self, table: &NSTableView, row: NSInteger) -> Option<Retained<NSView>> {
        let p = self.ivars().palette.borrow().upgrade()?;
        let mtm = MainThreadMarker::from(table);
        let id = NSString::from_str(CELL_ID);
        let reused = unsafe { table.makeViewWithIdentifier_owner(&id, Some(self)) }
            .and_then(|v| v.downcast::<NdPaletteCell>().ok());
        let cell = reused.unwrap_or_else(|| {
            let cell = NdPaletteCell::new(mtm);
            cell.setIdentifier(Some(&id));
            cell
        });
        if let Some(item) = p.model.borrow().rows().get(row as usize) {
            cell.configure(item);
        }
        Some(Retained::into_super(Retained::into_super(cell)))
    }
}

const CELL_ID: &str = "nd.palette.cell";
const ROW_ID: &str = "nd.palette.row";

/// The presented chrome. Lives only while the palette is open.
struct Ui {
    backdrop: Retained<NdPaletteBackdrop>,
    card: Retained<NdPaletteCard>,
    material: Retained<NSView>,
    body: Retained<NSView>,
    icon: Retained<NSImageView>,
    field: Retained<NSTextField>,
    separator: Retained<NSBox>,
    scroll: Retained<NSScrollView>,
    table: Retained<NSTableView>,
}

pub(crate) struct Palette {
    mtm: MainThreadMarker,
    emit: Emit,
    wake: Wake,
    /// The node's own view: hidden, zero-size, in the tree only so the palette has a window.
    handle: Retained<NSView>,
    source: Retained<NdPaletteSource>,
    model: RefCell<PaletteModel>,
    placeholder: RefCell<Option<String>>,
    ui: RefCell<Option<Ui>>,
    /// The overlay window the presented palette draws in.
    layer: RefCell<Option<crate::overlay::WindowLayer>>,
    open: Cell<bool>,
    /// A present waits for a window to anchor to.
    pending: Cell<bool>,
    return_focus: RefCell<Option<objc2::rc::Weak<NSResponder>>>,
    /// Set while nd writes the field, so the write is not taken for a user edit.
    writing: Cell<bool>,
    weak: Weak<Palette>,
}

pub(crate) struct PaletteNative(Rc<Palette>);

impl PaletteNative {
    pub fn new(mtm: MainThreadMarker, emit: Emit, wake: Wake, props: &CommandPaletteProps) -> Self {
        let handle = NSView::new(mtm);
        handle.setHidden(true);
        let palette = Rc::new_cyclic(|weak: &Weak<Palette>| {
            let source = NdPaletteSource::alloc(mtm).set_ivars(PaletteIvars { palette: RefCell::new(weak.clone()) });
            let source: Retained<NdPaletteSource> = unsafe { msg_send![super(source), init] };
            Palette {
                mtm,
                emit,
                wake,
                handle,
                source,
                model: RefCell::new(PaletteModel::default()),
                placeholder: RefCell::new(None),
                ui: RefCell::new(None),
                layer: RefCell::new(None),
                open: Cell::new(false),
                pending: Cell::new(false),
                return_focus: RefCell::new(None),
                writing: Cell::new(false),
                weak: weak.clone(),
            }
        });
        let this = Self(palette);
        this.update(props);
        this
    }

    pub fn view(&self) -> &NSView {
        &self.0.handle
    }

    pub fn update(&self, props: &CommandPaletteProps) {
        let p = &self.0;
        let placeholder = props.placeholder.as_ref().map(|s| s.to_string());
        if *p.placeholder.borrow() != placeholder {
            *p.placeholder.borrow_mut() = placeholder.clone();
            if let Some(ui) = p.ui.borrow().as_ref() {
                let ph = placeholder.as_deref().map(NSString::from_str);
                ui.field.setPlaceholderString(ph.as_deref());
                ui.field.setAccessibilityLabel(ph.as_deref());
            }
        }
        let reseed = p.model.borrow_mut().set_query(&props.query);
        let reload = p.model.borrow_mut().set_items(&props.items);
        if reload {
            p.reload();
        }
        if reseed || reload {
            p.sync_field(false);
        }
        if props.open != p.open.get() {
            p.open.set(props.open);
            if props.open {
                p.request_present();
            } else {
                p.dismiss(true, true);
            }
        }
    }

    pub fn model(&self) -> std::cell::Ref<'_, PaletteModel> {
        self.0.model.borrow()
    }

    /// The card in the window content's coordinates, while presented.
    pub fn card_frame_in(&self, view: &NSView) -> Option<NSRect> {
        let ui = self.0.ui.borrow();
        let card = &ui.as_ref()?.card;
        Some(view.convertRect_fromView(card.bounds(), Some(card)))
    }

    /// Automation: replaces the field's text as the user would.
    pub fn set_text(&self, text: &str) {
        self.0.write_field(text);
        self.0.user_edited(text);
    }

    /// Automation: types after what was typed.
    pub fn type_text(&self, text: &str) {
        let full = format!("{}{text}", self.0.model.borrow().typed());
        self.set_text(&full);
    }

    /// Automation: activates the highlighted row, or the top one.
    pub fn activate(&self) {
        let index = {
            let m = self.0.model.borrow();
            m.highlight().or((!m.rows().is_empty()).then_some(0))
        };
        if let Some(i) = index {
            self.0.click(i);
        }
    }

    pub fn press(&self, key: PaletteKey) -> bool {
        self.0.key(key)
    }

    /// What the presented palette draws, for tests and `paletteLayout`.
    pub fn layout_report(&self) -> Option<PaletteLayout> {
        let ui = self.0.ui.borrow();
        let ui = ui.as_ref()?;
        ui.backdrop.layoutSubtreeIfNeeded();
        let to = |v: &NSView| ui.backdrop.convertRect_fromView(v.bounds(), Some(v));
        let editor = ui.field.currentEditor();
        let selection = editor.as_ref().map(|e| e.selectedRange()).unwrap_or(NSRange::new(0, 0));
        let table = &ui.table;
        let rows = (0..table.numberOfRows())
            .filter_map(|i| {
                let cell = table.viewAtColumn_row_makeIfNecessary(0, i, false)?.downcast::<NdPaletteCell>().ok()?;
                Some(PaletteRowLayout {
                    row: to(&cell),
                    highlighted: table.selectedRow() == i,
                    truncated: cell.is_truncated(),
                    icon: (!cell.ivars().icon.isHidden()).then(|| to(&cell.ivars().icon)),
                    title: to(&cell.ivars().title),
                    subtitle: (!cell.ivars().subtitle.isHidden()).then(|| to(&cell.ivars().subtitle)),
                    hint: (!cell.ivars().hint.isHidden()).then(|| to(&cell.ivars().hint)),
                })
            })
            .collect();
        Some(PaletteLayout {
            window: ui.backdrop.bounds(),
            card: to(&ui.card),
            field: to(&ui.field),
            field_text: ui.field.stringValue().to_string(),
            selection,
            field_focused: editor.is_some(),
            material: ui.material.class().name().to_str().unwrap_or("?").to_owned(),
            list_hidden: ui.scroll.isHidden(),
            rows,
        })
    }
}

/// The geometry a presented palette draws, relative to its backdrop (the window).
#[derive(Clone, Debug)]
pub struct PaletteLayout {
    pub window: NSRect,
    pub card: NSRect,
    pub field: NSRect,
    pub field_text: String,
    pub selection: NSRange,
    pub field_focused: bool,
    pub material: String,
    pub list_hidden: bool,
    pub rows: Vec<PaletteRowLayout>,
}

#[derive(Clone, Debug)]
pub struct PaletteRowLayout {
    pub row: NSRect,
    pub highlighted: bool,
    pub truncated: bool,
    pub icon: Option<NSRect>,
    pub title: NSRect,
    pub subtitle: Option<NSRect>,
    pub hint: Option<NSRect>,
}

impl Drop for PaletteNative {
    // The backdrop lives in the overlay window, not under the handle: an unmount while open
    // would strand it.
    fn drop(&mut self) {
        self.0.pending.set(false);
        self.0.dismiss(true, false);
    }
}

impl Palette {
    fn emit(&self, events: Vec<PaletteEvent>) {
        if events.is_empty() {
            return;
        }
        for event in events {
            emit_palette_event(&self.emit, event);
        }
        self.wake.wake();
    }

    fn request_present(&self) {
        if self.pending.replace(true) {
            return;
        }
        let weak = self.weak.clone();
        // Never inside the transaction that opened it (3.13): the field takes first responder
        // and the window lays out.
        defer(move || {
            if let Some(p) = weak.upgrade() {
                p.present();
            }
        });
    }

    fn present(&self) {
        if !self.pending.get() || !self.open.get() {
            self.pending.set(false);
            return;
        }
        let app = NSApplication::sharedApplication(self.mtm);
        // Over the window the user is looking at, wherever the handle sits in the tree; when an
        // overlay of that window has the keyboard, over its owner.
        let window = app.keyWindow().or_else(|| app.mainWindow()).or_else(|| self.handle.window());
        let window = window
            .map(|w| if w.downcast_ref::<crate::NdWindow>().is_some() { w } else { w.parentWindow().unwrap_or(w) });
        let Some(window) = window else {
            // Not in a window yet: try again shortly, until it is or the palette closes.
            let weak = self.weak.clone();
            crate::events::after(0.05, move || {
                if let Some(p) = weak.upgrade() {
                    p.present();
                }
            });
            return;
        };
        self.pending.set(false);
        // A background-launched host has no key window and the field would never take focus.
        // Automation runs opt into activation so the palette types as it would for a user.
        if !app.isActive() && std::env::var_os("NATIVE_AUTOMATION").is_some_and(|v| v == "1") {
            app.activate();
        }
        let layer = crate::overlay::WindowLayer::new(self.mtm, &window, true);
        let host = layer.host();
        *self.return_focus.borrow_mut() = window.firstResponder().map(|r| objc2::rc::Weak::from_retained(&r));
        let presented = self.model.borrow_mut().present();
        if !presented {
            return;
        }
        let ui = self.build(&host);
        let field = ui.field.clone();
        *self.ui.borrow_mut() = Some(ui);
        let panel = layer.window().retain();
        panel.setAlphaValue(0.);
        layer.show();
        fade(&panel, 1., PALETTE.enter, || {});
        *self.layer.borrow_mut() = Some(layer);
        self.reload();
        self.sync_field(false);
        // Becoming first responder selects the field's whole text: a seeded present wants that.
        panel.makeFirstResponder(Some(&field));
    }

    fn user_cancel(&self) {
        let event = self.model.borrow_mut().dismiss(false);
        self.teardown(true);
        self.emit(event.into_iter().collect());
    }

    fn dismiss(&self, programmatic: bool, animated: bool) {
        self.pending.set(false);
        let event = self.model.borrow_mut().dismiss(programmatic);
        self.teardown(animated);
        self.emit(event.into_iter().collect());
    }

    /// Closes the panel: fading out when `animated`, at once on unmount.
    fn teardown(&self, animated: bool) {
        let Some(ui) = self.ui.borrow_mut().take() else { return };
        let layer = self.layer.borrow_mut().take();
        let window = layer.as_ref().map(|l| l.owner().retain());
        // The keyboard and pointer leave at once; the panel fades out and goes after.
        if let Some(layer) = &layer {
            let panel = layer.window();
            panel.setIgnoresMouseEvents(true);
            if panel.isKeyWindow() {
                layer.owner().makeKeyWindow();
            }
        }
        if let Some(panel) = layer.as_ref().filter(|_| animated).map(|l| l.window().retain()) {
            let parked = RefCell::new(Some((ui, layer)));
            fade(&panel, 0., PALETTE.exit, move || {
                if let Some((ui, layer)) = parked.take() {
                    ui.backdrop.removeFromSuperview();
                    drop(layer);
                }
            });
        } else {
            ui.backdrop.removeFromSuperview();
            drop(layer);
        }
        // Focus goes back where it was; a responder that left the window leaves it with the window.
        let back = self.return_focus.borrow_mut().take().and_then(|w| w.load());
        if let (Some(window), Some(back)) = (window, back) {
            let any: &AnyObject = &back;
            let in_window = any.downcast_ref::<NSView>().is_some_and(|v| v.window().is_some_and(|w| w == window))
                || any.downcast_ref::<NSWindow>().is_some_and(|w| *w == *window);
            if in_window {
                window.makeFirstResponder(Some(&back));
            }
        }
    }

    fn user_edited(&self, text: &str) {
        if self.writing.get() {
            return;
        }
        let events = self.model.borrow_mut().edited(text);
        self.sync_field(false);
        self.emit(events);
    }

    /// A key the field editor offers. True when the palette took it.
    fn key(&self, key: PaletteKey) -> bool {
        let before = self.model.borrow().completion().len();
        let result = self.model.borrow_mut().key(key);
        let Some(events) = result else { return false };
        let accepted = before > 0 && self.model.borrow().completion().is_empty();
        if !self.model.borrow().is_presented() {
            self.teardown(true);
        } else {
            self.sync_highlight();
            self.sync_field(accepted);
        }
        self.emit(events);
        true
    }

    fn click(&self, index: usize) {
        let events = self.model.borrow_mut().click(index);
        self.sync_highlight();
        self.emit(events);
    }

    fn write_field(&self, text: &str) {
        let Some(field) = self.ui.borrow().as_ref().map(|u| u.field.clone()) else { return };
        if field.stringValue().to_string() != text {
            self.writing.set(true);
            field.setStringValue(&NSString::from_str(text));
            self.writing.set(false);
        }
    }

    /// Writes the model's text and selection into the field. `caret_to_end` after an accepted
    /// completion, which leaves the text as it was.
    fn sync_field(&self, caret_to_end: bool) {
        let (text, typed, suffix, selection, presented) = {
            let m = self.model.borrow();
            (m.field_text(), utf16(m.typed()), utf16(m.completion()), m.selection(), m.is_presented())
        };
        if !presented {
            return;
        }
        self.write_field(&text);
        let Some(field) = self.ui.borrow().as_ref().map(|u| u.field.clone()) else { return };
        let Some(editor) = field.currentEditor() else { return };
        match selection {
            FieldSelection::All => editor.setSelectedRange(NSRange::new(0, utf16(&text))),
            FieldSelection::Completion => editor.setSelectedRange(NSRange::new(typed, suffix)),
            FieldSelection::Caret if caret_to_end => editor.setSelectedRange(NSRange::new(typed, 0)),
            FieldSelection::Caret => {}
        }
    }

    fn reload(&self) {
        let Some(table) = self.ui.borrow().as_ref().map(|u| u.table.clone()) else { return };
        table.reloadData();
        self.sync_highlight();
        if let Some(ui) = self.ui.borrow().as_ref() {
            ui.backdrop.setNeedsLayout(true);
        }
        self.layout();
        self.reassert_focus();
    }

    /// Keeps the field first responder across a reload, without taking the field editor away
    /// mid-edit (that would reselect the text).
    fn reassert_focus(&self) {
        let Some(field) = self.ui.borrow().as_ref().map(|u| u.field.clone()) else { return };
        if field.currentEditor().is_none()
            && let Some(window) = field.window()
        {
            window.makeFirstResponder(Some(&field));
        }
    }

    fn sync_highlight(&self) {
        let Some(table) = self.ui.borrow().as_ref().map(|u| u.table.clone()) else { return };
        match self.model.borrow().highlight() {
            Some(i) => {
                table.selectRowIndexes_byExtendingSelection(&NSIndexSet::indexSetWithIndex(i), false);
                table.scrollRowToVisible(i as NSInteger);
            }
            None => unsafe { table.deselectAll(None) },
        }
    }

    fn build(&self, host: &NSView) -> Ui {
        let mtm = self.mtm;
        let weak = || PaletteIvars { palette: RefCell::new(self.weak.clone()) };
        let backdrop = NdPaletteBackdrop::alloc(mtm).set_ivars(weak());
        let backdrop: Retained<NdPaletteBackdrop> = unsafe { msg_send![super(backdrop), initWithFrame: host.bounds()] };
        backdrop.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        backdrop.setWantsLayer(true);
        if let Some(layer) = backdrop.layer() {
            let dim = NSColor::blackColor().colorWithAlphaComponent(f64::from(PALETTE.dim_alpha));
            layer.setBackgroundColor(Some(&dim.CGColor()));
        }
        backdrop.setAccessibilityIdentifier(Some(&NSString::from_str("nd-command-palette-backdrop")));

        let card: Retained<NdPaletteCard> =
            unsafe { msg_send![NdPaletteCard::alloc(mtm), initWithFrame: NSRect::ZERO] };
        card.setWantsLayer(true);
        if let Some(layer) = card.layer() {
            layer.setShadowColor(Some(&NSColor::blackColor().CGColor()));
            layer.setShadowOpacity(PALETTE.shadow_opacity);
            layer.setShadowRadius(f64::from(PALETTE.shadow_radius));
            // Undirected: a layer's y axis follows its view's flippedness.
            layer.setShadowOffset(CGSize::new(0., 0.));
        }
        card.setAccessibilityElement(true);
        card.setAccessibilityRole(Some(unsafe { NSAccessibilityGroupRole }));
        let label = self.placeholder.borrow().clone().unwrap_or_else(|| "Command Palette".into());
        card.setAccessibilityLabel(Some(&NSString::from_str(&label)));
        backdrop.addSubview(&card);

        let body = crate::views::NdLayerView::new(mtm, &crate::look::ViewLook::default());
        let body: Retained<NSView> = Retained::into_super(body);
        let material: Retained<NSView> = if crate::glass::has_liquid_glass() {
            let glass = NSGlassEffectView::new(mtm);
            glass.setCornerRadius(f64::from(PALETTE.radius));
            glass.setContentView(Some(&body));
            Retained::into_super(glass)
        } else {
            let effect = NSVisualEffectView::new(mtm);
            effect.setMaterial(NSVisualEffectMaterial::Popover);
            effect.setBlendingMode(NSVisualEffectBlendingMode::WithinWindow);
            effect.setState(NSVisualEffectState::Active);
            effect.setWantsLayer(true);
            if let Some(layer) = effect.layer() {
                layer.setCornerRadius(f64::from(PALETTE.radius));
                layer.setMasksToBounds(true);
            }
            effect.addSubview(&body);
            Retained::into_super(effect)
        };
        card.addSubview(&material);

        let icon = NSImageView::new(mtm);
        let glyph =
            NSImageSymbolConfiguration::configurationWithPointSize_weight(f64::from(PALETTE.search_glyph), unsafe {
                NSFontWeightRegular
            });
        icon.setImage(crate::icons::symbol("search").and_then(|i| i.imageWithSymbolConfiguration(&glyph)).as_deref());
        icon.setContentTintColor(Some(&NSColor::secondaryLabelColor()));
        body.addSubview(&icon);

        let field = NSTextField::new(mtm);
        field.setBezeled(false);
        field.setBordered(false);
        field.setDrawsBackground(false);
        field.setFocusRingType(NSFocusRingType::None);
        field.setFont(Some(&NSFont::systemFontOfSize(f64::from(PALETTE.field_font_size))));
        if let Some(cell) = field.cell() {
            cell.setScrollable(true);
            cell.setWraps(false);
            cell.setUsesSingleLineMode(true);
        }
        field.setLineBreakMode(NSLineBreakMode::ByClipping);
        let ph = self.placeholder.borrow().as_deref().map(NSString::from_str);
        field.setPlaceholderString(ph.as_deref());
        field.setAccessibilityLabel(ph.as_deref());
        field.setStringValue(&NSString::from_str(&self.model.borrow().field_text()));
        // SAFETY: the source outlives the field: both belong to this palette.
        unsafe { field.setDelegate(Some(ProtocolObject::from_ref(&*self.source))) };
        body.addSubview(&field);

        let separator = NSBox::new(mtm);
        separator.setBoxType(NSBoxType::Separator);
        body.addSubview(&separator);

        let table = NSTableView::new(mtm);
        let column = NSTableColumn::initWithIdentifier(NSTableColumn::alloc(mtm), &NSString::from_str("nd.palette"));
        column.setResizingMask(NSTableColumnResizingOptions::AutoresizingMask);
        table.addTableColumn(&column);
        table.setHeaderView(None);
        table.setStyle(NSTableViewStyle::Inset);
        table.setColumnAutoresizingStyle(NSTableViewColumnAutoresizingStyle::UniformColumnAutoresizingStyle);
        table.setRowHeight(f64::from(PALETTE.row_height));
        table.setIntercellSpacing(NSSize::new(0., 0.));
        table.setBackgroundColor(&NSColor::clearColor());
        // A click activates the row; focus stays in the field.
        table.setRefusesFirstResponder(true);
        unsafe {
            table.setDataSource(Some(ProtocolObject::from_ref(&*self.source)));
            table.setDelegate(Some(ProtocolObject::from_ref(&*self.source)));
            table.setTarget(Some(&self.source));
            table.setAction(Some(sel!(rowClicked:)));
        }
        table.setAccessibilityLabel(Some(&NSString::from_str("Results")));

        let scroll = NSScrollView::new(mtm);
        scroll.setHasVerticalScroller(true);
        scroll.setAutohidesScrollers(true);
        scroll.setDrawsBackground(false);
        scroll.setDocumentView(Some(&table));
        body.addSubview(&scroll);

        host.addSubview(&backdrop);
        Ui { backdrop, card, material, body, icon, field, separator, scroll, table }
    }

    /// Places the card and its contents. The card's top edge rides a fixed fraction of the
    /// window height, so the field does not move while results change; the list follows the
    /// rows up to its ceiling and collapses with its separator when there are none.
    fn layout(&self) {
        let ui = self.ui.borrow();
        let Some(ui) = ui.as_ref() else { return };
        let m = PALETTE;
        let f = f64::from;
        let bounds = ui.backdrop.bounds();
        let (w, h) = (bounds.size.width, bounds.size.height);
        let card_w = f(m.width).min(w - 2. * f(m.margin)).max(0.);
        let field_h = ui.field.intrinsicContentSize().height.ceil();
        let rows = ui.table.numberOfRows();
        let empty = rows == 0;
        ui.separator.setHidden(empty);
        ui.scroll.setHidden(empty);
        // Read back from the table, so the inset style's row metrics are what the card sizes to.
        let content =
            if empty { 0. } else { ui.table.rectOfRow(rows - 1).origin.y + ui.table.rectOfRow(rows - 1).size.height };
        let chrome = f(m.field_top) + field_h + f(m.separator_gap) + 1. + f(m.list_gap);
        let mut top = (h * f(m.top_fraction)).max(f(m.margin));
        let list_h = if empty {
            0.
        } else {
            let room = h - f(m.margin) - top - chrome - f(m.list_gap);
            content.min(f(m.max_list_height)).min(room.max(f(m.row_height)))
        };
        let card_h = chrome + if empty { 0. } else { list_h + f(m.list_gap) };
        // A window too short for the card at its fraction moves it up, never past the margin.
        let overflow = top + card_h - (h - f(m.margin));
        if overflow > 0. {
            top = (top - overflow).max(f(m.margin));
        }
        let card = NSRect::new(NSPoint::new(((w - card_w) / 2.).round(), top.round()), NSSize::new(card_w, card_h));
        if ui.card.frame() != card {
            ui.card.setFrame(card);
        }
        let inner = NSRect::new(NSPoint::new(0., 0.), card.size);
        ui.material.setFrame(inner);
        ui.body.setFrame(inner);
        let icon = f(m.search_icon);
        let field_y = f(m.field_top);
        ui.icon.setFrame(NSRect::new(
            NSPoint::new(f(m.search_icon_inset), field_y + (field_h - icon) / 2.),
            NSSize::new(icon, icon),
        ));
        let field_x = f(m.search_icon_inset) + icon + f(m.search_icon_gap);
        ui.field.setFrame(NSRect::new(
            NSPoint::new(field_x, field_y),
            NSSize::new((card_w - field_x - f(m.field_trailing)).max(0.), field_h),
        ));
        let sep_y = field_y + field_h + f(m.separator_gap);
        ui.separator.setFrame(NSRect::new(NSPoint::new(0., sep_y), NSSize::new(card_w, 1.)));
        ui.scroll.setFrame(NSRect::new(NSPoint::new(0., sep_y + 1. + f(m.list_gap)), NSSize::new(card_w, list_h)));
    }
}

/// Animates `panel`'s alpha to `to` over `duration`, then runs `done`.
fn fade(panel: &NSWindow, to: f64, duration: std::time::Duration, done: impl Fn() + 'static) {
    let target = panel.retain();
    let changes = block2::RcBlock::new(move |context: std::ptr::NonNull<NSAnimationContext>| {
        unsafe { context.as_ref() }.setDuration(duration.as_secs_f64());
        target.animator().setAlphaValue(to);
    });
    let done = block2::RcBlock::new(done);
    NSAnimationContext::runAnimationGroup_completionHandler(&changes, Some(&done));
}
