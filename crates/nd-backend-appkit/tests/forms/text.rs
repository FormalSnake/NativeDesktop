//! Text fields, text area and combo box: AppKit classes, props, native edits through the field
//! editor, echo, reentrancy, IME composition, automation and the semantic tree.

use nd_backend_appkit::objc2::rc::Retained;
use nd_backend_appkit::objc2::runtime::{AnyClass, AnyObject};
use nd_backend_appkit::objc2::{msg_send, sel};
use nd_backend_appkit::objc2_app_kit::{
    NSAccessibility, NSBorderType, NSButton, NSComboBox, NSControlSize, NSFocusRingType, NSScrollView, NSSearchField,
    NSSecureTextField, NSText, NSTextField, NSTextFieldBezelStyle, NSTextView, NSView,
};
use nd_backend_appkit::objc2_foundation::{NSRange, NSSize, NSString};
use nd_components::Icon;
use nd_components::forms::{
    IconEdge, SelectChanged, TextChanged, TextEvent, TextFieldState, TextSubmitted, combo_box, search_field,
    secure_field, text_area, text_field,
};
use nd_ui::prelude::*;
use nd_ui::semantics::SemanticAction;
use nd_ui::{Role, WindowId};

use super::{Checks, Rig};

struct Texts {
    name: SharedString,
    name_changes: Vec<String>,
    submits: Vec<String>,
    upper: SharedString,
    upper_changes: u32,
    pass: SharedString,
    icons: Vec<IconEdge>,
    off: bool,
    notes: SharedString,
    notes_changes: u32,
    fruit: SharedString,
    fruit_picks: Vec<usize>,
    fruit_changes: Vec<String>,
    bound: Entity<TextFieldState>,
    query: Entity<TextFieldState>,
    searches: Vec<String>,
}

impl Render for Texts {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .gap_2()
            .p(px(12.))
            .child(
                text_field("name")
                    .text(self.name.clone())
                    .placeholder("Name")
                    .label("Full name")
                    .on_change(cx.listener(|this, e: &TextChanged, _, cx| {
                        this.name = e.text.clone();
                        this.name_changes.push(e.text.to_string());
                        cx.notify();
                    }))
                    .on_submit(cx.listener(|this, e: &TextSubmitted, _, _| this.submits.push(e.text.to_string()))),
            )
            .child(text_field("upper").text(self.upper.clone()).on_change(cx.listener(
                |this, e: &TextChanged, _, cx| {
                    this.upper = e.text.to_uppercase().into();
                    this.upper_changes += 1;
                    cx.notify();
                },
            )))
            .child(
                secure_field("pass")
                    .text(self.pass.clone())
                    .placeholder("Password")
                    .trailing_icon(Icon::Eye, "Show password")
                    .on_change(cx.listener(|this, e: &TextChanged, _, cx| {
                        this.pass = e.text.clone();
                        cx.notify();
                    }))
                    .on_icon_click(
                        cx.listener(|this, e: &nd_components::forms::FieldIconClicked, _, _| this.icons.push(e.edge)),
                    ),
            )
            .child(text_field("off").placeholder("Off").disabled(self.off))
            .child(text_field("bound").bind(&self.bound))
            .child(search_field("search", &self.query).large().on_submit(cx.listener(
                |this, e: &TextSubmitted, window, cx| {
                    this.searches.push(e.text.to_string());
                    this.bound.update(cx, |s, cx| s.set_text("from app", window, cx));
                },
            )))
            .child(text_area("notes").text(self.notes.clone()).placeholder("Notes").min_lines(4).on_change(
                cx.listener(|this, e: &TextChanged, _, cx| {
                    this.notes = e.text.clone();
                    this.notes_changes += 1;
                    cx.notify();
                }),
            ))
            .child(
                combo_box("fruit", ["Apple", "Banana", "Cherry"])
                    .text(self.fruit.clone())
                    .placeholder("Fruit")
                    .on_change(cx.listener(|this, e: &TextChanged, _, cx| {
                        this.fruit = e.text.clone();
                        this.fruit_changes.push(e.text.to_string());
                        cx.notify();
                    }))
                    .on_select(cx.listener(|this, e: &SelectChanged, _, _| this.fruit_picks.push(e.index))),
            )
    }
}

fn class(view: &NSView) -> String {
    view.class().name().to_string_lossy().into_owned()
}

fn field(rig: &Rig, w: WindowId, id: &str) -> Retained<NSTextField> {
    rig.view(w, id).downcast::<NSTextField>().unwrap_or_else(|_| panic!("{id} is not a text field"))
}

fn editor_of(window: &NSView, field: &NSTextField) -> Retained<NSText> {
    let w = window.window().expect("in a window");
    w.makeFirstResponder(Some(field));
    field.currentEditor().expect("field editor")
}

fn insert(editor: &AnyObject, text: &str) {
    let ns = NSString::from_str(text);
    let _: () = unsafe { msg_send![editor, insertText: &*ns] };
}

pub fn run(rig: &Rig, c: &mut Checks) {
    let w = rig.open("Text", |cx| {
        let bound = cx.new(|_| TextFieldState::default());
        let query = cx.new(|_| TextFieldState::default());
        Texts {
            name: SharedString::default(),
            name_changes: Vec::new(),
            submits: Vec::new(),
            upper: SharedString::default(),
            upper_changes: 0,
            pass: SharedString::default(),
            icons: Vec::new(),
            off: true,
            notes: SharedString::default(),
            notes_changes: 0,
            fruit: SharedString::default(),
            fruit_picks: Vec::new(),
            fruit_changes: Vec::new(),
            bound,
            query,
            searches: Vec::new(),
        }
    });
    rig.pump();
    let id = w.id();

    // Classes.
    let name = field(rig, id, "name");
    c.check(class(&name) == "NSTextField", "text_field mounts as NSTextField");
    let pass = field(rig, id, "pass");
    c.check(pass.downcast_ref::<NSSecureTextField>().is_some(), "secure_field mounts as NSSecureTextField");
    c.check(rig.view(id, "search").downcast::<NSSearchField>().is_ok(), "search_field mounts as NSSearchField");
    let combo = rig.view(id, "fruit").downcast::<NSComboBox>().expect("combo");
    c.check(class(&combo) == "NSComboBox", "combo_box mounts as NSComboBox");
    let scroll = rig.view(id, "notes").downcast::<NSScrollView>().expect("text_area is a scroll view");
    let notes = scroll.documentView().and_then(|v| v.downcast::<NSTextView>().ok()).expect("text view");
    // nd's NSTextView subclass, which reads pasted and dropped media when asked to.
    let area_class = class(&notes);
    c.check(
        area_class.contains("NdAreaView"),
        &format!("text_area mounts as an NSTextView in NSScrollView ({area_class})"),
    );
    // The area wears a text field's rounded bezel and the scroll view owns the focus ring, so it
    // matches the fields beside it instead of the square scroll view bezel.
    let bezel = scroll.subviews().iter().find_map(|v| v.downcast::<NSTextField>().ok());
    c.check(
        scroll.borderType() == NSBorderType::NoBorder
            && bezel.is_some_and(|b| b.isBezeled() && b.bezelStyle() == NSTextFieldBezelStyle::RoundedBezel),
        "text_area draws a rounded text field bezel",
    );
    c.check(scroll.focusRingType() == NSFocusRingType::Exterior, "text_area's focus ring goes round its bezel");

    // Props.
    c.check(name.placeholderString().is_some_and(|p| p.to_string() == "Name"), "placeholder reaches NSTextField");
    c.check(name.accessibilityLabel().is_some_and(|l| l.to_string() == "Full name"), "label reaches accessibility");
    c.check(!field(rig, id, "off").isEnabled(), "disabled field is not enabled");
    rig.update(&w, |v, cx| {
        v.off = false;
        cx.notify();
    });
    rig.pump();
    c.check(field(rig, id, "off").isEnabled(), "enabling the field reaches it");
    let combo_items = combo.numberOfItems();
    c.check(combo_items == 3, &format!("combo box has its 3 options ({combo_items})"));

    // Trailing icon: a borderless button inside the secure field, text kept clear of it.
    let icon = pass.subviews().iter().find_map(|v| v.downcast::<NSButton>().ok());
    c.check(icon.as_ref().is_some_and(|b| !b.isBordered()), "secure field holds a borderless icon button");
    if let Some(icon) = &icon {
        let pass_width = pass.bounds().size.width;
        let text_rect = pass.cell().map(|cell| cell.drawingRectForBounds(pass.bounds()));
        let clear =
            text_rect.is_some_and(|r| r.origin.x + r.size.width <= icon.frame().origin.x + 0.5 && pass_width > 0.);
        c.check(clear, "secure field's text rect stops before its icon");
        // A host that resizes the field after mount (a SwiftUI form row) keeps the icon centred.
        let mounted = pass.frame().size;
        for size in [NSSize::new(mounted.width + 80., mounted.height + 14.), mounted] {
            pass.setFrameSize(size);
            pass.layoutSubtreeIfNeeded();
            let (b, f) = (pass.bounds(), icon.frame());
            let off = (f.origin.y + f.size.height / 2.) - (b.origin.y + b.size.height / 2.);
            let edge = b.size.width - (f.origin.x + f.size.width);
            c.check(
                off.abs() <= 0.5 && edge > 0. && edge < b.size.width / 4.,
                &format!("icon stays centred and trailing at {}x{} (off {off}, edge {edge})", size.width, size.height),
            );
        }
        let _ = unsafe { icon.sendAction_to(icon.action(), icon.target().as_deref()) };
        rig.pump();
        c.check(rig.read(&w, |v| v.icons == [IconEdge::Trailing]), "icon click arrives as FieldIconClicked");
        let editor = editor_of(&w.window.contentView().expect("content"), &pass);
        insert(&editor, "pw");
        rig.pump();
        let editor_frame = editor.frame();
        c.check(
            editor_frame.origin.x + editor_frame.size.width <= icon.frame().origin.x + 0.5,
            "field editor sits clear of the icon",
        );
        c.check(rig.read(&w, |v| v.pass == "pw"), "secure edit arrives as TextChanged");
    }
    c.check(rig.node(id, "pass").is_some_and(|n| n.value.is_none()), "secure field exposes no value");

    // Native edit through the field editor, then echo.
    let content = w.window.contentView().expect("content");
    let editor = editor_of(&content, &name);
    insert(&editor, "Ada");
    editor.setSelectedRange(NSRange::new(1, 0));
    rig.pump();
    rig.pump();
    c.check(rig.read(&w, |v| v.name == "Ada"), "typing arrives as TextChanged");
    c.check(rig.read(&w, |v| v.name_changes.len() == 1), "one TextChanged per edit");
    let sel = editor.selectedRange();
    c.check(sel.location == 1 && sel.length == 0, "feeding the text back leaves the caret alone (no set)");

    // Return submits.
    let _: () = unsafe { msg_send![&*editor, doCommandBySelector: sel!(insertNewline:)] };
    rig.pump();
    c.check(rig.read(&w, |v| v.submits == ["Ada"]), "Return arrives as TextSubmitted");

    // Reentrancy: the handler rewrites the text it was told about.
    let upper = field(rig, id, "upper");
    let editor = editor_of(&content, &upper);
    insert(&editor, "abc");
    rig.pump();
    rig.pump();
    c.check(rig.read(&w, |v| v.upper == "ABC" && v.upper_changes == 1), "handler rewrite emits once");
    c.check(upper.stringValue().to_string() == "ABC", "handler rewrite reaches the field");

    // Automation.
    let r = rig.perform(id, "name", SemanticAction::SetValue("Grace".into()));
    c.check(r.is_ok() && rig.read(&w, |v| v.name == "Grace"), "SetValue reaches the app");
    c.check(name.stringValue().to_string() == "Grace", "SetValue shows in the field");
    let r = rig.perform(id, "name", SemanticAction::Type("!".into()));
    c.check(r.is_ok() && rig.read(&w, |v| v.name == "Grace!"), "Type appends through the app");
    let r = rig.perform(id, "fruit", SemanticAction::Type("Kiwi".into()));
    c.check(r.is_ok() && rig.read(&w, |v| v.fruit == "Kiwi"), "Type into an unfocused combo box");

    // IME composition: an app write waits until editing ends.
    let editor = editor_of(&content, &name);
    let marked = NSString::from_str("か");
    let _: () = unsafe {
        msg_send![&*editor, setMarkedText: &*marked, selectedRange: NSRange::new(1, 0), replacementRange: NSRange::new(usize::MAX >> 1, 0)]
    };
    let composing: bool = unsafe { msg_send![&*editor, hasMarkedText] };
    rig.pump();
    rig.update(&w, |v, cx| {
        v.name = "Reset".into();
        cx.notify();
    });
    rig.pump();
    let held = editor.string().to_string();
    c.check(composing && held.contains('か'), &format!("app write waits during composition ({held})"));
    let _: () = unsafe { msg_send![&*editor, unmarkText] };
    w.window.makeFirstResponder(None);
    rig.pump();
    rig.pump();
    let landed = name.stringValue().to_string();
    c.check(landed == "Reset", &format!("app write lands when composition ends ({landed})"));

    // Text area.
    w.window.makeFirstResponder(Some(&notes));
    insert(&notes, "Hello");
    rig.pump();
    rig.pump();
    c.check(rig.read(&w, |v| v.notes == "Hello" && v.notes_changes == 1), "text area edit arrives as TextChanged");
    let placeholder: Option<Retained<AnyObject>> = unsafe { msg_send![&*notes, placeholderAttributedString] };
    c.check(placeholder.is_some(), "text area has a placeholder");
    let line =
        unsafe { notes.layoutManager() }.zip(notes.font()).map_or(0., |(lm, font)| lm.defaultLineHeightForFont(&font));
    let height = scroll.frame().size.height;
    c.check(line > 0. && height >= 4. * line, &format!("min_lines sizes the area ({height} for line {line})"));
    let r = rig.perform(id, "notes", SemanticAction::SetValue("Two\nlines".into()));
    c.check(r.is_ok() && rig.read(&w, |v| v.notes == "Two\nlines"), "SetValue on the text area reaches the app");

    // Combo box pick.
    combo.selectItemAtIndex(1);
    rig.pump();
    rig.pump();
    c.check(rig.read(&w, |v| v.fruit_picks == [1]), "picking an item arrives as SelectChanged");
    c.check(rig.read(&w, |v| v.fruit == "Banana"), "picking an item arrives as TextChanged");

    // Bound state and the search field; the search handler writes the bound state's text.
    let r = rig.perform(id, "bound", SemanticAction::SetValue("bound text".into()));
    let bound = rig.read(&w, |v| v.bound.clone());
    let state = |e: &Entity<TextFieldState>| {
        let app = rig.app.borrow();
        let s = e.read(&app);
        (s.text().to_string(), s.last_event())
    };
    c.check(r.is_ok() && state(&bound) == ("bound text".into(), Some(TextEvent::Edited)), "edit reaches a bound state");
    let search = rig.view(id, "search").downcast::<NSSearchField>().expect("search");
    let large =
        if AnyClass::get(c"NSGlassEffectView").is_some() { NSControlSize::ExtraLarge } else { NSControlSize::Large };
    c.check(search.controlSize() == large, "large() is the largest control size");
    c.check(search.frame().size.height >= 28., &format!("large() field is tall ({})", search.frame().size.height));
    let r = rig.perform(id, "search", SemanticAction::SetValue("query".into()));
    let query = rig.read(&w, |v| v.query.clone());
    c.check(r.is_ok() && state(&query).0 == "query", "search field edit reaches its state");
    let _ = unsafe { search.sendAction_to(search.action(), search.target().as_deref()) };
    rig.pump();
    rig.pump();
    c.check(rig.read(&w, |v| v.searches == ["query"]), "search action arrives as TextSubmitted");
    c.check(state(&query).1 == Some(TextEvent::Submitted), "search submit reaches its state");
    c.check(field(rig, id, "bound").stringValue().to_string() == "from app", "set_text reaches a bound field");

    // Semantic tree.
    let node = rig.node(id, "name").expect("name node");
    c.check(node.role == Role::TextField, "text field role");
    c.check(node.value.as_deref() == Some("Reset"), &format!("text field value ({:?})", node.value));
    c.check(node.placeholder.as_deref() == Some("Name"), "text field placeholder");
    c.check(rig.node(id, "search").is_some_and(|n| n.role == Role::SearchBox), "search field role");
    c.check(rig.node(id, "notes").is_some_and(|n| n.role == Role::TextArea), "text area role");
    c.check(
        rig.node(id, "fruit").is_some_and(|n| n.role == Role::ComboBox && n.value.as_deref() == Some("Banana")),
        "combo box role and value",
    );

    rig.close(w);
}
