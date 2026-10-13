//! `text_field`, `secure_field` and `search_field` as `NSTextField`, `NSSecureTextField` and
//! `NSSearchField`; `text_area` as an `NSTextView` in a bezeled `NSScrollView`; `combo_box` as
//! `NSComboBox`. One delegate per control reports edits, Return and combo picks.
//!
//! Field icons on plain and secure fields are borderless SF Symbol buttons inside the field's
//! bezel, and the field's cell draws and edits its text in the rect between them. A search
//! field puts them on its own search and cancel buttons instead.
//!
//! Text is native-owned while the user edits: a value the app feeds back equals what the field
//! holds and sets nothing, and a different value written during IME composition waits until
//! the composition or the editing session ends (ARCHITECTURE.md 3.13).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use nd_components::forms::{
    ComboBoxProps, FieldIcon, FieldIconClicked, FieldStyle, IconEdge, SelectChanged, TextAreaProps, TextChanged,
    TextFieldProps, TextSubmitted,
};
use nd_core::Size;
use nd_ui::backend::Props;
use nd_ui::events::Synced;
use nd_ui::prelude::SharedString;
use nd_ui::semantics::{ActionError, Probe, SemanticAction};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject, Sel};
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAccessibility, NSActionCell, NSAttributedStringNSStringDrawing, NSAutoresizingMaskOptions, NSBorderType,
    NSButton, NSCell, NSCellImagePosition, NSColor, NSComboBox, NSComboBoxDelegate, NSControl, NSControlSize,
    NSControlTextEditingDelegate, NSFont, NSFontAttributeName, NSForegroundColorAttributeName, NSScrollView,
    NSScroller, NSSearchField, NSSearchFieldCell, NSSearchFieldDelegate, NSSecureTextField, NSSecureTextFieldCell,
    NSText, NSTextDelegate, NSTextField, NSTextFieldBezelStyle, NSTextFieldCell, NSTextFieldDelegate, NSTextView,
    NSTextViewDelegate, NSView,
};
use objc2_foundation::{
    MainThreadMarker, NSArray, NSAttributedString, NSDictionary, NSNotification, NSObject, NSObjectProtocol, NSRect,
    NSSize, NSString,
};

use super::{Ctx, FormControl, NdControlTarget, apply_common, props, set_accessibility_label};
use crate::events::defer;

#[derive(Default)]
pub struct CellInsets {
    leading: Cell<f64>,
    trailing: Cell<f64>,
}

impl CellInsets {
    fn inset(&self, mut r: NSRect) -> NSRect {
        let (l, t) = (self.leading.get(), self.trailing.get());
        r.origin.x += l;
        r.size.width = (r.size.width - l - t).max(0.);
        r
    }
}

// The field editor is placed in `drawingRectForBounds:` too, so this one override keeps typed
// text clear of the icons as well as drawn text.
macro_rules! inset_cell {
    ($name:ident, $($super:ident),+) => {
        define_class!(
            // SAFETY: the cell subclass only overrides `drawingRectForBounds:`, calling super, and
            // its ivars are plain `Cell<f64>`s, so a bitwise `copyWithZone:` stays sound.
            #[unsafe(super($($super),+))]
            #[thread_kind = MainThreadOnly]
            #[ivars = CellInsets]
            struct $name;

            impl $name {
                #[unsafe(method(drawingRectForBounds:))]
                fn drawing_rect(&self, bounds: NSRect) -> NSRect {
                    let r: NSRect = unsafe { msg_send![super(self), drawingRectForBounds: bounds] };
                    self.ivars().inset(r)
                }
            }
        );

        impl $name {
            fn new(mtm: MainThreadMarker) -> Retained<Self> {
                let this = Self::alloc(mtm).set_ivars(CellInsets::default());
                unsafe { msg_send![super(this), initTextCell: &*NSString::new()] }
            }
        }
    };
}

inset_cell!(NdFieldCell, NSTextFieldCell, NSActionCell, NSCell, NSObject);
inset_cell!(NdSecureFieldCell, NSSecureTextFieldCell, NSTextFieldCell, NSActionCell, NSCell, NSObject);

enum Source {
    Field(Retained<NSTextField>),
    Area(Retained<NSTextView>),
}

/// What the control and its delegate share: the text slot and how to read and write the
/// native text.
struct Core {
    cx: Ctx,
    source: Source,
    search: bool,
    text: RefCell<Synced<SharedString>>,
    pending: RefCell<Option<SharedString>>,
    options: RefCell<Vec<SharedString>>,
}

impl Core {
    fn new(cx: Ctx, source: Source, search: bool) -> Rc<Self> {
        Rc::new(Self {
            cx,
            source,
            search,
            text: RefCell::default(),
            pending: RefCell::default(),
            options: RefCell::default(),
        })
    }

    /// The text view the user types into: the field editor while a field is being edited.
    fn editor(&self) -> Option<Retained<NSText>> {
        match &self.source {
            Source::Field(f) => f.currentEditor(),
            Source::Area(t) => Some(Retained::into_super(t.clone())),
        }
    }

    fn current(&self) -> String {
        match &self.source {
            Source::Field(f) => f.currentEditor().map_or_else(|| f.stringValue(), |e| e.string()).to_string(),
            Source::Area(t) => t.string().to_string(),
        }
    }

    fn composing(&self) -> bool {
        self.editor().is_some_and(|e| unsafe { msg_send![&*e, hasMarkedText] })
    }

    /// Writes `text` into the control without echoing; equal text is left alone so the caret
    /// and selection stay where the user put them.
    fn set_native(&self, text: &str) {
        if self.current() == text {
            return;
        }
        let ns = NSString::from_str(text);
        self.cx.emit.programmatic(|| match &self.source {
            Source::Field(f) => match f.currentEditor() {
                Some(editor) => editor.setString(&ns),
                None => f.setStringValue(&ns),
            },
            Source::Area(t) => t.setString(&ns),
        });
    }

    /// The app's text, applied unless the user is composing.
    fn apply(&self, text: &SharedString) {
        if self.text.borrow().value() == Some(text) {
            self.pending.take();
            return;
        }
        // Cloned out so a delegate the setter fires can still borrow the slot.
        let mut synced = self.text.borrow().clone();
        synced.apply(text, |v| {
            if self.composing() {
                *self.pending.borrow_mut() = Some(v.clone());
                return self.current().into();
            }
            self.pending.take();
            self.set_native(v);
            v.clone()
        });
        *self.text.borrow_mut() = synced;
    }

    /// A user edit: report, emit, wake. With composition over, an app write that waited wins.
    fn edited(&self) {
        if !self.composing()
            && let Some(text) = self.pending.take()
        {
            self.set_native(&text);
            self.text.borrow_mut().report(text);
            return;
        }
        let text: SharedString = self.current().into();
        if self.text.borrow().value() == Some(&text) {
            return;
        }
        self.text.borrow_mut().report(text.clone());
        self.cx.send(TextChanged { text });
    }

    /// Editing ended. A waiting app write lands once the field editor is gone, since the
    /// field copies the editor's text back as editing ends.
    fn ended(self: &Rc<Self>) {
        if self.pending.borrow().is_none() {
            return;
        }
        let core = self.clone();
        defer(move || {
            if let Some(text) = core.pending.take() {
                core.set_native(&text);
                core.text.borrow_mut().report(text);
            }
        });
    }

    fn submitted(&self) {
        self.cx.send(TextSubmitted { text: self.current().into() });
    }

    fn picked(&self, index: isize) {
        let Some(text) = usize::try_from(index).ok().and_then(|i| self.options.borrow().get(i).cloned()) else {
            return;
        };
        self.text.borrow_mut().report(text.clone());
        self.cx.emit.emit(SelectChanged { index: index as usize });
        self.cx.send(TextChanged { text });
    }

    /// Automation's `SetValue`: the native text changes, then the user-edit path runs.
    fn replace(&self, text: &str) {
        self.pending.take();
        let ns = NSString::from_str(text);
        match &self.source {
            Source::Field(f) => match f.currentEditor() {
                Some(editor) => editor.setString(&ns),
                None => f.setStringValue(&ns),
            },
            Source::Area(t) => t.setString(&ns),
        }
        self.edited();
    }

    /// Automation's `Type`: into the editor the way a keyboard would when the control is
    /// being edited, else appended to its text.
    fn type_text(&self, text: &str) {
        let editing = match &self.source {
            Source::Field(f) => f.currentEditor(),
            Source::Area(t) => t
                .window()
                .and_then(|w| w.firstResponder())
                .is_some_and(|r| std::ptr::eq(&*r as *const _ as *const NSTextView, &**t))
                .then(|| Retained::into_super(t.clone())),
        };
        match editing {
            Some(editor) => {
                let ns = NSString::from_str(text);
                let _: () = unsafe { msg_send![&*editor, insertText: &*ns] };
                self.edited();
            }
            None => self.replace(&format!("{}{text}", self.current())),
        }
    }
}

pub struct DelegateIvars {
    core: Rc<Core>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements and this type does not implement Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = DelegateIvars]
    struct NdTextDelegate;

    unsafe impl NSObjectProtocol for NdTextDelegate {}

    unsafe impl NSControlTextEditingDelegate for NdTextDelegate {
        #[unsafe(method(controlTextDidChange:))]
        fn control_text_did_change(&self, _n: &NSNotification) {
            self.ivars().core.edited();
        }

        #[unsafe(method(controlTextDidEndEditing:))]
        fn control_text_did_end_editing(&self, _n: &NSNotification) {
            self.ivars().core.ended();
        }

        #[unsafe(method(control:textView:doCommandBySelector:))]
        fn do_command(&self, _control: &NSControl, _text_view: &NSTextView, command: Sel) -> bool {
            let core = &self.ivars().core;
            // A search field submits through its own action.
            if command == sel!(insertNewline:) && !core.search {
                core.submitted();
            }
            false
        }
    }

    unsafe impl NSTextFieldDelegate for NdTextDelegate {}
    unsafe impl NSSearchFieldDelegate for NdTextDelegate {}

    unsafe impl NSComboBoxDelegate for NdTextDelegate {
        #[unsafe(method(comboBoxSelectionDidChange:))]
        fn combo_selection_did_change(&self, n: &NSNotification) {
            let Some(combo) = n.object().and_then(|o| o.downcast::<NSComboBox>().ok()) else { return };
            self.ivars().core.picked(combo.indexOfSelectedItem());
        }
    }

    unsafe impl NSTextDelegate for NdTextDelegate {
        #[unsafe(method(textDidChange:))]
        fn text_did_change(&self, _n: &NSNotification) {
            self.ivars().core.edited();
        }

        #[unsafe(method(textDidEndEditing:))]
        fn text_did_end_editing(&self, _n: &NSNotification) {
            self.ivars().core.ended();
        }
    }

    unsafe impl NSTextViewDelegate for NdTextDelegate {}
);

impl NdTextDelegate {
    fn new(mtm: MainThreadMarker, core: Rc<Core>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(DelegateIvars { core });
        unsafe { msg_send![super(this), init] }
    }
}

fn attributes(font: &NSFont, color: Option<&NSColor>) -> Retained<NSDictionary<NSString, AnyObject>> {
    let mut keys: Vec<&NSString> = vec![unsafe { NSFontAttributeName }];
    let mut values: Vec<&AnyObject> = vec![font];
    if let Some(color) = color {
        keys.push(unsafe { NSForegroundColorAttributeName });
        values.push(color);
    }
    NSDictionary::from_slices(&keys, &values)
}

fn text_width(text: &str, font: Option<&NSFont>) -> f64 {
    let Some(font) = font else { return 0. };
    let s = unsafe { NSAttributedString::new_with_attributes(&NSString::from_str(text), &attributes(font, None)) };
    s.size().width.ceil()
}

/// A borderless SF Symbol button sitting inside a field's bezel.
struct IconButton {
    button: Retained<NSButton>,
    _target: Retained<NdControlTarget>,
}

impl IconButton {
    fn new(cx: &Ctx, icon: &FieldIcon, edge: IconEdge) -> Self {
        let send = cx.clone();
        let target = NdControlTarget::new(cx.mtm, move |_| send.send(FieldIconClicked { edge }));
        let button = NSButton::new(cx.mtm);
        button.setBordered(false);
        button.setImagePosition(NSCellImagePosition::ImageOnly);
        button.setRefusesFirstResponder(true);
        target.wire(&button);
        let this = Self { button, _target: target };
        this.set(icon);
        this
    }

    fn set(&self, icon: &FieldIcon) {
        self.button.setImage(crate::icons::symbol(icon.icon.lucide()).as_deref());
        self.button.setContentTintColor(Some(&NSColor::secondaryLabelColor()));
        let label = NSString::from_str(&icon.label);
        self.button.setAccessibilityLabel(Some(&label));
        self.button.setToolTip(Some(&label));
        self.button.sizeToFit();
    }

    fn width(&self) -> f64 {
        self.button.frame().size.width
    }
}

enum Insets {
    Plain(Retained<NdFieldCell>),
    Secure(Retained<NdSecureFieldCell>),
}

impl Insets {
    fn ivars(&self) -> &CellInsets {
        match self {
            Self::Plain(c) => c.ivars(),
            Self::Secure(c) => c.ivars(),
        }
    }
}

pub(crate) struct TextFieldControl {
    field: Retained<NSTextField>,
    core: Rc<Core>,
    delegate: Retained<NdTextDelegate>,
    icons: [Option<IconButton>; 2],
    insets: Option<Insets>,
    search_target: Option<Retained<NdControlTarget>>,
    icon_targets: Vec<Retained<NdControlTarget>>,
    props: TextFieldProps,
}

fn has_icons(p: &TextFieldProps) -> bool {
    p.leading.is_some() || p.trailing.is_some()
}

impl TextFieldControl {
    pub fn new(cx: Ctx, props: &TextFieldProps) -> Self {
        let mtm = cx.mtm;
        let field: Retained<NSTextField> = match props.field {
            FieldStyle::Plain => NSTextField::new(mtm),
            FieldStyle::Secure => Retained::into_super(NSSecureTextField::new(mtm)),
            FieldStyle::Search => Retained::into_super(NSSearchField::new(mtm)),
        };
        let core = Core::new(cx.clone(), Source::Field(field.clone()), props.field == FieldStyle::Search);
        let delegate = NdTextDelegate::new(mtm, core.clone());
        let mut this = Self {
            field,
            core,
            delegate,
            icons: [None, None],
            insets: None,
            search_target: None,
            icon_targets: Vec::new(),
            props: props.clone(),
        };
        if props.field != FieldStyle::Search && has_icons(props) {
            this.adopt_inset_cell(mtm, props.field == FieldStyle::Secure);
        }
        let field = &this.field;
        if props.field != FieldStyle::Search {
            field.setBezeled(true);
            field.setBezelStyle(NSTextFieldBezelStyle::RoundedBezel);
        }
        field.setUsesSingleLineMode(true);
        unsafe { field.setDelegate(Some(ProtocolObject::from_ref(&*this.delegate))) };
        if let Some(search) = field.downcast_ref::<NSSearchField>() {
            search.setSendsWholeSearchString(true);
            let core = this.core.clone();
            let target = NdControlTarget::new(mtm, move |_| {
                core.edited();
                core.submitted();
            });
            target.wire(search);
            this.search_target = Some(target);
        }
        this.apply(None, props);
        this
    }

    /// Swaps in a cell that keeps its text clear of the icons, keeping the stock cell's font.
    fn adopt_inset_cell(&mut self, mtm: MainThreadMarker, secure: bool) {
        let font = self.field.font();
        let insets =
            if secure { Insets::Secure(NdSecureFieldCell::new(mtm)) } else { Insets::Plain(NdFieldCell::new(mtm)) };
        let cell: &NSCell = match &insets {
            Insets::Plain(c) => c,
            Insets::Secure(c) => c,
        };
        self.field.setCell(Some(cell));
        self.field.setFont(font.as_deref());
        self.field.setEditable(true);
        self.field.setSelectable(true);
        cell.setScrollable(true);
        cell.setWraps(false);
        self.insets = Some(insets);
    }

    fn apply(&mut self, old: Option<&TextFieldProps>, new: &TextFieldProps) {
        let field = &self.field;
        if old.is_none_or(|o| o.placeholder != new.placeholder) {
            field.setPlaceholderString(new.placeholder.as_deref().map(NSString::from_str).as_deref());
        }
        if old.is_none_or(|o| o.editable != new.editable) {
            field.setEditable(new.editable);
            field.setSelectable(true);
        }
        apply_common(Some(field), field, old.map(|o| (o.enabled, &o.tooltip)), new.enabled, &new.tooltip);
        if old.is_none_or(|o| o.label != new.label) {
            set_accessibility_label(field, new.label.as_deref());
        }
        if old.is_none_or(|o| o.leading != new.leading || o.trailing != new.trailing) {
            self.apply_icons(new);
        }
        self.core.apply(&new.text);
    }

    fn apply_icons(&mut self, p: &TextFieldProps) {
        if let Some(search) = self.field.downcast_ref::<NSSearchField>() {
            self.icon_targets.clear();
            let Some(cell) = search.cell().and_then(|c| c.downcast::<NSSearchFieldCell>().ok()) else { return };
            let buttons = [
                (cell.searchButtonCell(), &p.leading, IconEdge::Leading),
                (cell.cancelButtonCell(), &p.trailing, IconEdge::Trailing),
            ];
            for (button, icon, edge) in buttons {
                let (Some(button), Some(icon)) = (button, icon) else { continue };
                let send = self.core.cx.clone();
                let target = NdControlTarget::new(self.core.cx.mtm, move |_| send.send(FieldIconClicked { edge }));
                button.setImage(crate::icons::symbol(icon.icon.lucide()).as_deref());
                unsafe {
                    button.setTarget(Some(&target));
                    button.setAction(Some(sel!(changed:)));
                }
                self.icon_targets.push(target);
            }
            return;
        }
        let Some(insets) = &self.insets else { return };
        let cx = self.core.cx.clone();
        for (slot, icon, edge) in [(0, &p.leading, IconEdge::Leading), (1, &p.trailing, IconEdge::Trailing)] {
            match (icon, &self.icons[slot]) {
                (Some(icon), Some(b)) => b.set(icon),
                (Some(icon), None) => {
                    let b = IconButton::new(&cx, icon, edge);
                    self.field.addSubview(&b.button);
                    self.icons[slot] = Some(b);
                }
                (None, Some(b)) => {
                    b.button.removeFromSuperview();
                    self.icons[slot] = None;
                }
                (None, None) => {}
            }
        }
        let widths = self.icons.each_ref().map(|b| b.as_ref().map_or(0., IconButton::width));
        insets.ivars().leading.set(widths[0]);
        insets.ivars().trailing.set(widths[1]);
        self.place_icons();
    }

    /// Lays the icons out against the field's current frame, then lets autoresizing keep them
    /// pinned to their edges and centred as the field's frame changes.
    fn place_icons(&self) {
        let Some(insets) = &self.insets else { return };
        let field = &self.field;
        let cell_height = field.cell().map_or(0., |c| c.cellSize().height);
        let mut bounds = field.bounds();
        if bounds.size.width <= 0. || bounds.size.height <= 0. {
            let width = self.icons.iter().flatten().map(IconButton::width).sum::<f64>() * 4.;
            field.setFrameSize(NSSize::new(width, cell_height));
            bounds = field.bounds();
        }
        let (l, t) = (insets.ivars().leading.replace(0.), insets.ivars().trailing.replace(0.));
        let margin = field.cell().map_or(0., |c| c.drawingRectForBounds(bounds).origin.x - bounds.origin.x);
        insets.ivars().leading.set(l);
        insets.ivars().trailing.set(t);
        for (slot, b) in self.icons.iter().enumerate() {
            let Some(b) = b else { continue };
            let size = b.button.frame().size;
            let x = if slot == 0 { margin } else { bounds.size.width - margin - size.width };
            let y = ((bounds.size.height - size.height) / 2.).round();
            b.button.setFrame(NSRect::new(objc2_foundation::NSPoint::new(x, y), size));
            let pin = if slot == 0 {
                NSAutoresizingMaskOptions::ViewMaxXMargin
            } else {
                NSAutoresizingMaskOptions::ViewMinXMargin
            };
            b.button.setAutoresizingMask(
                pin | NSAutoresizingMaskOptions::ViewMinYMargin | NSAutoresizingMaskOptions::ViewMaxYMargin,
            );
        }
    }
}

impl Drop for TextFieldControl {
    fn drop(&mut self) {
        unsafe { self.field.setDelegate(None) };
    }
}

impl FormControl for TextFieldControl {
    fn view(&self) -> &NSView {
        &self.field
    }

    fn update(&mut self, old: &Props, new: &Props) -> bool {
        let (old, new): (&TextFieldProps, &TextFieldProps) = (props(old), props(new));
        if old.field != new.field || (new.field != FieldStyle::Search && has_icons(old) != has_icons(new)) {
            return false;
        }
        self.apply(Some(old), new);
        self.props = new.clone();
        true
    }

    fn measure(&self, _width: Option<f32>) -> Size {
        field_size(&self.field, self.props.text.is_empty(), self.props.placeholder.as_deref(), self.icon_widths())
    }

    fn probe(&self, probe: &mut Probe) {
        probe.enabled = Some(self.field.isEnabled());
        if self.props.field != FieldStyle::Secure {
            probe.value = Some(self.core.current());
        }
        probe.placeholder = self.field.placeholderString().map(|p| p.to_string());
    }

    fn perform(&self, action: &SemanticAction) -> Option<Result<(), ActionError>> {
        perform_text(&self.core, self.props.enabled, self.props.editable, action)
    }
}

impl TextFieldControl {
    fn icon_widths(&self) -> f64 {
        self.insets.as_ref().map_or(0., |i| i.ivars().leading.get() + i.ivars().trailing.get())
    }
}

fn field_size(field: &NSTextField, empty: bool, placeholder: Option<&str>, extra: f64) -> Size {
    let cell = field.cell().map_or(NSSize::new(0., 0.), |c| c.cellSize());
    let placeholder = placeholder.map_or(0., |p| text_width(p, field.font().as_deref()));
    let width = if empty { cell.width + placeholder } else { cell.width.max(placeholder) };
    Size { width: (width + extra).ceil() as f32, height: cell.height.ceil() as f32 }
}

fn perform_text(
    core: &Core,
    enabled: bool,
    editable: bool,
    action: &SemanticAction,
) -> Option<Result<(), ActionError>> {
    if !matches!(action, SemanticAction::SetValue(_) | SemanticAction::Type(_)) {
        return None;
    }
    if !enabled {
        return Some(Err(ActionError::NotActionable("the control is disabled".into())));
    }
    if !editable {
        return Some(Err(ActionError::NotActionable("the field is read-only".into())));
    }
    match action {
        SemanticAction::SetValue(v) => core.replace(v),
        SemanticAction::Type(t) => core.type_text(t),
        _ => unreachable!(),
    }
    Some(Ok(()))
}

pub(crate) struct ComboBoxControl {
    combo: Retained<NSComboBox>,
    core: Rc<Core>,
    _delegate: Retained<NdTextDelegate>,
    props: ComboBoxProps,
}

impl ComboBoxControl {
    pub fn new(cx: Ctx, props: &ComboBoxProps) -> Self {
        let mtm = cx.mtm;
        let combo = NSComboBox::new(mtm);
        let field: Retained<NSTextField> = Retained::into_super(combo.clone());
        let core = Core::new(cx, Source::Field(field), false);
        let delegate = NdTextDelegate::new(mtm, core.clone());
        unsafe { combo.setDelegate(Some(ProtocolObject::from_ref(&*delegate))) };
        combo.setUsesSingleLineMode(true);
        let this = Self { combo, core, _delegate: delegate, props: props.clone() };
        this.apply(None, props);
        this
    }

    fn apply(&self, old: Option<&ComboBoxProps>, new: &ComboBoxProps) {
        let combo = &self.combo;
        if old.is_none_or(|o| o.options != new.options) {
            combo.removeAllItems();
            let items: Vec<Retained<NSString>> = new.options.iter().map(|o| NSString::from_str(o)).collect();
            let items: Retained<NSArray> = unsafe { Retained::cast_unchecked(NSArray::from_retained_slice(&items)) };
            unsafe { combo.addItemsWithObjectValues(&items) };
            *self.core.options.borrow_mut() = new.options.clone();
        }
        if old.is_none_or(|o| o.placeholder != new.placeholder) {
            combo.setPlaceholderString(new.placeholder.as_deref().map(NSString::from_str).as_deref());
        }
        combo.setCompletes(new.completes);
        if old.is_none_or(|o| o.editable != new.editable) {
            combo.setEditable(new.editable);
            combo.setSelectable(true);
        }
        apply_common(Some(combo), combo, old.map(|o| (o.enabled, &o.tooltip)), new.enabled, &new.tooltip);
        if old.is_none_or(|o| o.label != new.label) {
            set_accessibility_label(combo, new.label.as_deref());
        }
        self.core.apply(&new.text);
    }
}

impl Drop for ComboBoxControl {
    fn drop(&mut self) {
        unsafe { self.combo.setDelegate(None) };
    }
}

impl FormControl for ComboBoxControl {
    fn view(&self) -> &NSView {
        &self.combo
    }

    fn update(&mut self, old: &Props, new: &Props) -> bool {
        let (old, new): (&ComboBoxProps, &ComboBoxProps) = (props(old), props(new));
        self.apply(Some(old), new);
        self.props = new.clone();
        true
    }

    fn measure(&self, _width: Option<f32>) -> Size {
        let widest = self.props.options.iter().map(|o| text_width(o, self.combo.font().as_deref())).fold(0., f64::max);
        field_size(&self.combo, self.props.text.is_empty(), self.props.placeholder.as_deref(), 0.).max_width(widest)
    }

    fn probe(&self, probe: &mut Probe) {
        probe.enabled = Some(self.combo.isEnabled());
        probe.value = Some(self.core.current());
        probe.placeholder = self.combo.placeholderString().map(|p| p.to_string());
    }

    fn perform(&self, action: &SemanticAction) -> Option<Result<(), ActionError>> {
        perform_text(&self.core, self.props.enabled, self.props.editable, action)
    }
}

trait MaxWidth {
    fn max_width(self, content: f64) -> Self;
}

impl MaxWidth for Size {
    /// Widens a combo box so its widest option fits beside the pop-up button.
    fn max_width(mut self, content: f64) -> Self {
        self.width = self.width.max((content as f32) + self.height);
        self
    }
}

pub(crate) struct TextAreaControl {
    scroll: Retained<NSScrollView>,
    text_view: Retained<NSTextView>,
    core: Rc<Core>,
    _delegate: Retained<NdTextDelegate>,
    enabled: Cell<bool>,
    props: TextAreaProps,
}

impl TextAreaControl {
    pub fn new(cx: Ctx, props: &TextAreaProps) -> Self {
        let mtm = cx.mtm;
        let scroll = NSTextView::scrollableTextView(mtm);
        scroll.setBorderType(NSBorderType::BezelBorder);
        scroll.setAutohidesScrollers(true);
        let text_view = scroll
            .documentView()
            .and_then(|v| v.downcast::<NSTextView>().ok())
            .expect("scrollableTextView holds a text view");
        text_view.setRichText(false);
        text_view.setAllowsUndo(true);
        text_view.setFont(Some(&NSFont::systemFontOfSize(NSFont::systemFontSize())));
        let core = Core::new(cx, Source::Area(text_view.clone()), false);
        let delegate = NdTextDelegate::new(mtm, core.clone());
        text_view.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        let this =
            Self { scroll, text_view, core, _delegate: delegate, enabled: Cell::new(true), props: props.clone() };
        this.apply(None, props);
        this
    }

    fn apply(&self, old: Option<&TextAreaProps>, new: &TextAreaProps) {
        let tv = &self.text_view;
        if old.is_none_or(|o| o.placeholder != new.placeholder) {
            self.set_placeholder(new.placeholder.as_deref());
        }
        if old.is_none_or(|o| o.editable != new.editable || o.enabled != new.enabled) {
            tv.setEditable(new.editable && new.enabled);
            tv.setSelectable(new.enabled);
            let color = if new.enabled { NSColor::textColor() } else { NSColor::disabledControlTextColor() };
            tv.setTextColor(Some(&color));
            self.enabled.set(new.enabled);
        }
        apply_common(None, tv, old.map(|o| (o.enabled, &o.tooltip)), new.enabled, &new.tooltip);
        if old.is_none_or(|o| o.label != new.label) {
            set_accessibility_label(tv, new.label.as_deref());
        }
        self.core.apply(&new.text);
    }

    /// NSTextView draws `placeholderAttributedString` while empty; it is not in the public
    /// headers, so it is set only where the text view answers to it.
    fn set_placeholder(&self, placeholder: Option<&str>) {
        let tv = &self.text_view;
        tv.setAccessibilityPlaceholderValue(placeholder.map(NSString::from_str).as_deref());
        let setter = sel!(setPlaceholderAttributedString:);
        if !tv.respondsToSelector(setter) {
            return;
        }
        let attributed = match (placeholder, tv.font()) {
            (Some(p), Some(font)) => Some(unsafe {
                NSAttributedString::new_with_attributes(
                    &NSString::from_str(p),
                    &attributes(&font, Some(&NSColor::placeholderTextColor())),
                )
            }),
            _ => None,
        };
        let _: () = unsafe { msg_send![&**tv, setPlaceholderAttributedString: attributed.as_deref()] };
    }

    fn line_height(&self) -> f64 {
        let tv = &self.text_view;
        match (unsafe { tv.layoutManager() }, tv.font()) {
            (Some(lm), Some(font)) => lm.defaultLineHeightForFont(&font),
            _ => 0.,
        }
    }
}

impl Drop for TextAreaControl {
    fn drop(&mut self) {
        self.text_view.setDelegate(None);
    }
}

impl FormControl for TextAreaControl {
    fn view(&self) -> &NSView {
        &self.scroll
    }

    fn focus_view(&self) -> &NSView {
        &self.text_view
    }

    fn update(&mut self, old: &Props, new: &Props) -> bool {
        let (old, new): (&TextAreaProps, &TextAreaProps) = (props(old), props(new));
        self.apply(Some(old), new);
        self.props = new.clone();
        true
    }

    fn measure(&self, width: Option<f32>) -> Size {
        let tv = &self.text_view;
        let inset = tv.textContainerInset();
        let padding = unsafe { tv.textContainer() }.map_or(0., |c| c.lineFragmentPadding());
        let placeholder = self.props.placeholder.as_deref().map_or(0., |p| text_width(p, tv.font().as_deref()));
        let content = NSSize::new(
            placeholder + 2. * (inset.width + padding),
            f64::from(self.props.min_lines.max(1)) * self.line_height() + 2. * inset.height,
        );
        let frame = unsafe {
            NSScrollView::frameSizeForContentSize_horizontalScrollerClass_verticalScrollerClass_borderType_controlSize_scrollerStyle(
                content,
                None,
                None,
                self.scroll.borderType(),
                NSControlSize::Regular,
                NSScroller::preferredScrollerStyle(self.core.cx.mtm),
                self.core.cx.mtm,
            )
        };
        Size { width: width.unwrap_or(frame.width.ceil() as f32), height: frame.height.ceil() as f32 }
    }

    fn probe(&self, probe: &mut Probe) {
        probe.enabled = Some(self.enabled.get());
        probe.value = Some(self.core.current());
        probe.placeholder = self.props.placeholder.as_ref().map(|p| p.to_string());
    }

    fn perform(&self, action: &SemanticAction) -> Option<Result<(), ActionError>> {
        perform_text(&self.core, self.props.enabled, self.props.editable, action)
    }
}
