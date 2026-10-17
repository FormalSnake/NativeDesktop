//! Text entry: `text_field`, `secure_field`, `search_field`, `text_area`. `NSTextField`,
//! `NSSecureTextField`, `NSSearchField` and an `NSTextView` in a scroll view on AppKit.
//!
//! Text is the app's, either as a controlled value (`.text(value)` plus `.on_change`) or bound
//! to a [`TextFieldState`] entity (`.bind(&state)`). While the user edits, the field owns the
//! text: a value the app feeds back equals what the field holds, so nothing is set, and an app
//! write during IME composition waits until it ends (ARCHITECTURE.md 3.13).

use std::rc::Rc;

use nd_ui::prelude::*;
use nd_ui::semantics::{Response, SemanticAction, Semantics, WidgetSemantics};
use nd_ui::{DispatchProps, EntityId, Key, Role, StyleRefinement, deferred};

use super::guard;
use crate::icon::Icon;

/// Fired on every user edit, with the field's whole text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextChanged {
    pub text: SharedString,
}

/// Return in a single-line field (or the search field's search action).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextSubmitted {
    pub text: SharedString,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IconEdge {
    Leading,
    Trailing,
}

/// A click on a field's leading or trailing icon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FieldIconClicked {
    pub edge: IconEdge,
}

/// What last changed a [`TextFieldState`]'s text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TextEvent {
    /// The user typed, pasted or deleted.
    Edited,
    /// The app called [`TextFieldState::set_text`].
    Replaced,
    /// Return in the field.
    Submitted,
}

/// Text a field binds to, for views that keep it as an entity. Observers see every change;
/// [`TextFieldState::last_event`] says whether the user or the app made it.
#[derive(Clone, Debug, Default)]
pub struct TextFieldState {
    text: SharedString,
    placeholder: Option<SharedString>,
    last_event: Option<TextEvent>,
    /// Views that show this state through a bound field; `set_text` re-renders them.
    viewers: Vec<EntityId>,
}

impl TextFieldState {
    pub fn new(_window: &mut Window, _cx: &mut Context<Self>) -> Self {
        Self::default()
    }

    pub fn placeholder(mut self, placeholder: impl Into<SharedString>) -> Self {
        self.placeholder = Some(placeholder.into());
        self
    }

    pub fn text(&self) -> &SharedString {
        &self.text
    }

    pub fn placeholder_text(&self) -> Option<&SharedString> {
        self.placeholder.as_ref()
    }

    pub fn last_event(&self) -> Option<TextEvent> {
        self.last_event
    }

    /// Replaces the text from the app. A field showing it takes the new text unless the user is
    /// composing with an input method, in which case it lands when composition ends.
    pub fn set_text(&mut self, text: impl Into<SharedString>, _window: &mut Window, cx: &mut Context<Self>) {
        self.text = text.into();
        self.last_event = Some(TextEvent::Replaced);
        self.changed(cx);
    }

    fn changed(&self, cx: &mut Context<Self>) {
        cx.notify();
        for viewer in self.viewers.clone() {
            (**cx).notify(viewer);
        }
    }

    fn record(&mut self, text: Option<SharedString>, event: TextEvent, cx: &mut Context<Self>) {
        if let Some(text) = text {
            self.text = text;
        }
        self.last_event = Some(event);
        self.changed(cx);
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum FieldStyle {
    #[default]
    Plain,
    /// Characters hidden; never readable back through automation.
    Secure,
    /// The platform's search field: magnifier, clear button, rounded on macOS.
    Search,
}

/// An icon inside a field's edge. `label` names it for accessibility and its tooltip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldIcon {
    pub icon: Icon,
    pub label: SharedString,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextFieldProps {
    pub text: SharedString,
    pub placeholder: Option<SharedString>,
    pub field: FieldStyle,
    pub leading: Option<FieldIcon>,
    pub trailing: Option<FieldIcon>,
    pub editable: bool,
    /// The platform's largest field size, for a field that heads a pane, like a sidebar's search.
    pub large: bool,
    /// Accessible name when no visible label names the field.
    pub label: Option<SharedString>,
    pub enabled: bool,
    pub tooltip: Option<SharedString>,
    pub style: StyleRefinement,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextAreaProps {
    pub text: SharedString,
    pub placeholder: Option<SharedString>,
    pub editable: bool,
    /// Lines of height the area asks for before the app's style sizes it.
    pub min_lines: u16,
    /// Grows with its text from `min_lines` up to this many lines, then scrolls.
    pub max_lines: Option<u16>,
    /// Return fires [`TextSubmitted`]; Shift-Return or Option-Return inserts a line break.
    pub submit_on_return: bool,
    /// Pasted or dropped images arrive as [`ImagesPasted`] and dropped files as
    /// [`FilesDropped`], instead of as text.
    pub accepts_media: bool,
    /// Opens the emoji picker at the caret each time it changes: the platform's (the Character
    /// Viewer, the Windows emoji panel) or, where there is none, nd's own. The chosen emoji goes
    /// in at the caret.
    pub emoji_picker: u64,
    /// Points kept clear at the trailing edge, inside the field's outline: room for a button
    /// laid over it (a chat bar's send button).
    pub trailing_room: f32,
    pub label: Option<SharedString>,
    pub enabled: bool,
    pub tooltip: Option<SharedString>,
    pub style: StyleRefinement,
}

/// An image the user pasted or dropped into a text area that accepts media.
#[derive(Clone, PartialEq, Eq)]
pub struct PastedImage {
    pub bytes: std::sync::Arc<[u8]>,
    /// `image/png`, `image/tiff`, ...
    pub mime: SharedString,
}

impl std::fmt::Debug for PastedImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PastedImage({}, {} bytes)", self.mime, self.bytes.len())
    }
}

/// Images pasted or dropped into a text area that accepts media.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImagesPasted {
    pub images: Vec<PastedImage>,
}

/// Files dropped (or pasted as files) into a text area that accepts media.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FilesDropped {
    pub paths: Vec<std::path::PathBuf>,
}

/// A text field with a list of suggestions (`NSComboBox`). The text is free; picking a
/// suggestion fills it and fires [`SelectChanged`](super::SelectChanged) as well.
#[derive(Clone, Debug, PartialEq)]
pub struct ComboBoxProps {
    pub text: SharedString,
    pub options: Vec<SharedString>,
    pub placeholder: Option<SharedString>,
    pub editable: bool,
    /// Completes typed text from the options, as `NSComboBox.completes` does.
    pub completes: bool,
    pub label: Option<SharedString>,
    pub enabled: bool,
    pub tooltip: Option<SharedString>,
    pub style: StyleRefinement,
}

/// Widget kind of `combo_box`.
pub struct ComboBoxKind;

/// Widget kind of `text_field`, `secure_field` and `search_field`.
pub struct TextFieldKind;
/// Widget kind of `text_area`.
pub struct TextAreaKind;

/// Props of a text-entry widget, for the builder they share.
pub trait TextProps: WidgetSemantics + Clone {
    fn text_mut(&mut self) -> &mut SharedString;
    fn placeholder_mut(&mut self) -> &mut Option<SharedString>;
    fn style_mut(&mut self) -> &mut StyleRefinement;
    fn enabled_mut(&mut self) -> &mut bool;
    fn tooltip_mut(&mut self) -> &mut Option<SharedString>;
    fn label_mut(&mut self) -> &mut Option<SharedString>;
    fn editable_mut(&mut self) -> &mut bool;
}

macro_rules! text_props {
    ($props:ty) => {
        impl TextProps for $props {
            fn text_mut(&mut self) -> &mut SharedString {
                &mut self.text
            }
            fn placeholder_mut(&mut self) -> &mut Option<SharedString> {
                &mut self.placeholder
            }
            fn style_mut(&mut self) -> &mut StyleRefinement {
                &mut self.style
            }
            fn enabled_mut(&mut self) -> &mut bool {
                &mut self.enabled
            }
            fn tooltip_mut(&mut self) -> &mut Option<SharedString> {
                &mut self.tooltip
            }
            fn label_mut(&mut self) -> &mut Option<SharedString> {
                &mut self.label
            }
            fn editable_mut(&mut self) -> &mut bool {
                &mut self.editable
            }
        }
    };
}

text_props!(TextFieldProps);
text_props!(TextAreaProps);
text_props!(ComboBoxProps);

fn respond_text(text: &str, enabled: bool, editable: bool, action: &SemanticAction) -> Option<Response> {
    let next = match action {
        SemanticAction::SetValue(v) => v.clone(),
        SemanticAction::Type(t) => format!("{text}{t}"),
        _ => return None,
    };
    if let Some(refused) = guard(enabled) {
        return Some(refused);
    }
    if !editable {
        return Some(super::not_taken("the field is read-only"));
    }
    Some(Ok(Box::new(TextChanged { text: next.into() })))
}

impl WidgetSemantics for TextFieldProps {
    fn semantics(&self) -> Semantics {
        let role = if self.field == FieldStyle::Search { Role::SearchBox } else { Role::TextField };
        let name = self.label.clone().or_else(|| self.tooltip.clone()).unwrap_or_default();
        let mut s = Semantics::new(role, name.to_string())
            .placeholder(self.placeholder.as_ref().map(|p| p.to_string()))
            .enabled(self.enabled);
        if self.field != FieldStyle::Secure {
            s = s.value(self.text.to_string());
        }
        s
    }

    fn respond(&self, action: &SemanticAction) -> Option<Response> {
        respond_text(&self.text, self.enabled, self.editable, action)
    }
}

impl WidgetSemantics for TextAreaProps {
    fn semantics(&self) -> Semantics {
        let name = self.label.clone().or_else(|| self.tooltip.clone()).unwrap_or_default();
        Semantics::new(Role::TextArea, name.to_string())
            .value(self.text.to_string())
            .placeholder(self.placeholder.as_ref().map(|p| p.to_string()))
            .enabled(self.enabled)
    }

    fn respond(&self, action: &SemanticAction) -> Option<Response> {
        respond_text(&self.text, self.enabled, self.editable, action)
    }
}

impl WidgetSemantics for ComboBoxProps {
    fn semantics(&self) -> Semantics {
        let name = self.label.clone().or_else(|| self.tooltip.clone()).unwrap_or_default();
        Semantics::new(Role::ComboBox, name.to_string())
            .value(self.text.to_string())
            .placeholder(self.placeholder.as_ref().map(|p| p.to_string()))
            .enabled(self.enabled)
    }

    fn respond(&self, action: &SemanticAction) -> Option<Response> {
        respond_text(&self.text, self.enabled, self.editable, action)
    }
}

type Handler<E> = Rc<dyn Fn(&E, &mut Window, &mut App)>;

/// The builder every text widget shares. Use the aliases: [`TextField`], [`TextArea`].
pub struct TextInput<P: TextProps> {
    element: NativeElement<P>,
    bound: Option<Entity<TextFieldState>>,
    on_change: Option<Handler<TextChanged>>,
    on_submit: Option<Handler<TextSubmitted>>,
    on_icon: Option<Handler<FieldIconClicked>>,
}

pub type TextField = TextInput<TextFieldProps>;
pub type TextArea = TextInput<TextAreaProps>;
pub type ComboBox = TextInput<ComboBoxProps>;

fn field(id: impl Into<SharedString>, style: FieldStyle) -> TextField {
    let props = TextFieldProps {
        text: SharedString::default(),
        placeholder: None,
        field: style,
        leading: None,
        trailing: None,
        editable: true,
        large: false,
        label: None,
        enabled: true,
        tooltip: None,
        style: StyleRefinement::default(),
    };
    TextInput::new(NativeElement::described::<TextFieldKind>(Key::Id(id.into()), props).focusable())
}

/// A single-line text field.
pub fn text_field(id: impl Into<SharedString>) -> TextField {
    field(id, FieldStyle::Plain)
}

/// A password field: characters hidden, value never exposed to automation reads.
pub fn secure_field(id: impl Into<SharedString>) -> TextField {
    field(id, FieldStyle::Secure)
}

/// The platform's search field, bound to `state`.
pub fn search_field(id: impl Into<SharedString>, state: &Entity<TextFieldState>) -> TextField {
    field(id, FieldStyle::Search).bind(state)
}

/// Multi-line text, scrolling inside its frame.
pub fn text_area(id: impl Into<SharedString>) -> TextArea {
    let props = TextAreaProps {
        text: SharedString::default(),
        placeholder: None,
        editable: true,
        min_lines: 3,
        max_lines: None,
        submit_on_return: false,
        accepts_media: false,
        emoji_picker: 0,
        trailing_room: 0.,
        label: None,
        enabled: true,
        tooltip: None,
        style: StyleRefinement::default(),
    };
    TextInput::new(NativeElement::described::<TextAreaKind>(Key::Id(id.into()), props).focusable())
}

/// A text field with suggestions.
pub fn combo_box(id: impl Into<SharedString>, options: impl IntoIterator<Item = impl Into<SharedString>>) -> ComboBox {
    let props = ComboBoxProps {
        text: SharedString::default(),
        options: options.into_iter().map(Into::into).collect(),
        placeholder: None,
        editable: true,
        completes: true,
        label: None,
        enabled: true,
        tooltip: None,
        style: StyleRefinement::default(),
    };
    TextInput::new(NativeElement::described::<ComboBoxKind>(Key::Id(id.into()), props).focusable())
}

impl<P: TextProps> TextInput<P> {
    fn new(element: NativeElement<P>) -> Self {
        Self { element, bound: None, on_change: None, on_submit: None, on_icon: None }
    }

    /// The controlled text.
    pub fn text(mut self, text: impl Into<SharedString>) -> Self {
        *self.element.props_mut().text_mut() = text.into();
        self
    }

    pub fn placeholder(mut self, placeholder: impl Into<SharedString>) -> Self {
        *self.element.props_mut().placeholder_mut() = Some(placeholder.into());
        self
    }

    /// Names the field for accessibility when no visible label does.
    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        *self.element.props_mut().label_mut() = Some(label.into());
        self
    }

    /// Selectable but not editable.
    pub fn read_only(mut self, read_only: bool) -> Self {
        *self.element.props_mut().editable_mut() = !read_only;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        *self.element.props_mut().enabled_mut() = !disabled;
        self
    }

    pub fn tooltip(mut self, tooltip: impl Into<SharedString>) -> Self {
        *self.element.props_mut().tooltip_mut() = Some(tooltip.into());
        self
    }

    /// Text and placeholder come from `state`, and edits go back into it.
    pub fn bind(mut self, state: &Entity<TextFieldState>) -> Self {
        self.bound = Some(state.clone());
        self
    }

    pub fn on_change(mut self, f: impl Fn(&TextChanged, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(f));
        self
    }

    pub fn on_submit(mut self, f: impl Fn(&TextSubmitted, &mut Window, &mut App) + 'static) -> Self {
        self.on_submit = Some(Rc::new(f));
        self
    }

    pub fn props(&self) -> &P {
        self.element.props()
    }

    fn finish(self, state: Option<Entity<TextFieldState>>) -> AnyElement {
        let Self { mut element, on_change, on_submit, on_icon, .. } = self;
        let style = element.style().clone();
        *element.props_mut().style_mut() = style;
        let changed_state = state.clone();
        element = element.on(move |e: &TextChanged, window, cx| {
            if let Some(state) = &changed_state {
                state.update(cx, |s, cx| s.record(Some(e.text.clone()), TextEvent::Edited, cx));
            }
            if let Some(f) = &on_change {
                f(e, window, cx);
            }
        });
        element = element.on(move |e: &TextSubmitted, window, cx| {
            if let Some(state) = &state {
                state.update(cx, |s, cx| s.record(None, TextEvent::Submitted, cx));
            }
            if let Some(f) = &on_submit {
                f(e, window, cx);
            }
        });
        if let Some(f) = on_icon {
            element = element.on(move |e: &FieldIconClicked, window, cx| f(e, window, cx));
        }
        element.into_any_element()
    }
}

impl TextField {
    pub fn secure(mut self) -> Self {
        self.element.props_mut().field = FieldStyle::Secure;
        self
    }

    pub fn search(mut self) -> Self {
        self.element.props_mut().field = FieldStyle::Search;
        self
    }

    /// The platform's largest field size: extra large (36 pt) on macOS 26, large before it.
    pub fn large(mut self) -> Self {
        self.element.props_mut().large = true;
        self
    }

    /// An icon at the field's leading edge; clicks fire [`FieldIconClicked`].
    pub fn leading_icon(mut self, icon: Icon, label: impl Into<SharedString>) -> Self {
        self.element.props_mut().leading = Some(FieldIcon { icon, label: label.into() });
        self
    }

    /// An icon at the field's trailing edge (a reveal or clear button).
    pub fn trailing_icon(mut self, icon: Icon, label: impl Into<SharedString>) -> Self {
        self.element.props_mut().trailing = Some(FieldIcon { icon, label: label.into() });
        self
    }

    pub fn on_icon_click(mut self, f: impl Fn(&FieldIconClicked, &mut Window, &mut App) + 'static) -> Self {
        self.on_icon = Some(Rc::new(f));
        self
    }
}

impl TextArea {
    pub fn min_lines(mut self, lines: u16) -> Self {
        self.element.props_mut().min_lines = lines;
        self
    }

    /// Grows with its text up to `lines`, then scrolls.
    pub fn max_lines(mut self, lines: u16) -> Self {
        self.element.props_mut().max_lines = Some(lines);
        self
    }

    /// Return submits ([`TextInput::on_submit`]); Shift-Return inserts a line break.
    pub fn submit_on_return(mut self) -> Self {
        self.element.props_mut().submit_on_return = true;
        self.element = self.element.takes_return();
        self
    }

    /// Takes pasted and dropped images and files as events instead of text.
    pub fn accepts_media(mut self) -> Self {
        self.element.props_mut().accepts_media = true;
        self
    }

    /// Keeps `points` clear at the trailing edge, inside the outline, for a button the app lays
    /// over the field there; the text wraps before it.
    pub fn trailing_room(mut self, points: f32) -> Self {
        self.element.props_mut().trailing_room = points.max(0.);
        self
    }

    /// Opens the emoji picker at the caret whenever `request` changes (the platform's, or nd's
    /// own where it has none); pass a counter the app bumps. The chosen emoji goes in at the caret.
    pub fn emoji_picker(mut self, request: u64) -> Self {
        self.element.props_mut().emoji_picker = request;
        self
    }

    pub fn on_images(mut self, f: impl Fn(&ImagesPasted, &mut Window, &mut App) + 'static) -> Self {
        self.element = self.element.on(f);
        self
    }

    pub fn on_files(mut self, f: impl Fn(&FilesDropped, &mut Window, &mut App) + 'static) -> Self {
        self.element = self.element.on(f);
        self
    }
}

impl ComboBox {
    pub fn completes(mut self, completes: bool) -> Self {
        self.element.props_mut().completes = completes;
        self
    }

    /// The user picked a suggestion; its text arrives as a [`TextChanged`] too.
    pub fn on_select(mut self, f: impl Fn(&super::SelectChanged, &mut Window, &mut App) + 'static) -> Self {
        self.element = self.element.on(f);
        self
    }
}

impl<P: TextProps> Styled for TextInput<P> {
    fn style(&mut self) -> &mut StyleRefinement {
        self.element.style()
    }
}

impl<P: TextProps> FluentBuilder for TextInput<P> {}

impl<P: TextProps> InteractiveElement for TextInput<P> {
    fn dispatch_props(&mut self) -> &mut DispatchProps {
        self.element.dispatch_props()
    }
}

impl<P: TextProps> IntoElement for TextInput<P> {
    fn into_any_element(mut self) -> AnyElement {
        match self.bound.take() {
            None => self.finish(None),
            // The state is read while the tree reconciles, so the field shows its latest text.
            Some(state) => deferred(move |_window, cx| {
                if let Some(view) = cx.rendering_view()
                    && !state.read(cx).viewers.contains(&view)
                {
                    state.update(cx, |s, _| s.viewers.push(view));
                }
                let s = state.read(cx);
                let (text, placeholder) = (s.text.clone(), s.placeholder.clone());
                let props = self.element.props_mut();
                *props.text_mut() = text;
                if props.placeholder_mut().is_none() {
                    *props.placeholder_mut() = placeholder;
                }
                self.finish(Some(state))
            }),
        }
    }
}
