//! Element descriptions: what `render` returns. Each becomes a retained node (or a view
//! boundary) through the reconciler; nothing here touches a backend.
//!
//! Identity (3.14): an element with an explicit id is keyed by it; every other element by the
//! `#[track_caller]` location of its constructor, plus an occurrence index among siblings with
//! the same location. `.when(..)` therefore never shifts a sibling onto another native node.

use std::any::{Any, TypeId};
use std::panic::Location;
use std::rc::Rc;

use nd_core::{NodeId, WindowId};
use nd_gpui::gpui::prelude::FluentBuilder;
use nd_gpui::gpui::{Action, KeyContext, SharedString, StyleRefinement, Styled};

use crate::app::{AnyView, App, EntityId};
use crate::backend::{AnyProps, NodeKind, Props, WidgetKind};
use crate::dispatch::{DispatchTree, Focusable};
use crate::window::Window;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    Id(SharedString),
    Site(&'static Location<'static>),
    Entity(EntityId),
}

impl From<SharedString> for Key {
    fn from(id: SharedString) -> Self {
        Key::Id(id)
    }
}

impl From<&'static str> for Key {
    fn from(id: &'static str) -> Self {
        Key::Id(id.into())
    }
}

pub(crate) type Handler = Rc<dyn Fn(&dyn Any, &mut Window, &mut App)>;

/// Registers one `on_action` listener on the node's entry in its window's dispatch tree.
pub(crate) type ActionRegistrar = Rc<dyn Fn(&mut DispatchTree<App>, NodeId, WindowId)>;

/// What an element contributes to dispatch (ARCHITECTURE.md 3.12): its key context, action
/// listeners and focusability. The retained tree hands them to the window's [`DispatchTree`]
/// on every render.
#[derive(Clone, Default)]
pub struct DispatchProps {
    pub(crate) key_context: Option<KeyContext>,
    pub(crate) actions: Vec<ActionRegistrar>,
    pub(crate) focusable: Option<Focusable>,
}

/// Elements that take part in key dispatch: GPUI's `key_context` and `on_action`.
pub trait InteractiveElement: Sized {
    fn dispatch_props(&mut self) -> &mut DispatchProps;

    /// Names the context bindings match against while focus is in this element's subtree.
    /// Panics on a context that does not parse, like a malformed binding would.
    fn key_context<C, E>(mut self, context: C) -> Self
    where
        C: TryInto<KeyContext, Error = E>,
        E: std::fmt::Debug,
    {
        let context = context.try_into().unwrap_or_else(|e| panic!("invalid key context: {e:?}"));
        self.dispatch_props().key_context = Some(context);
        self
    }

    /// Makes the element a focus target: Tab reaches it in tree order, and bindings match in
    /// its contexts while it has focus.
    fn focusable(mut self) -> Self {
        self.dispatch_props().focusable.get_or_insert_with(Focusable::default);
        self
    }

    /// Focusable, ordered by `index` before tree order in Tab traversal (lower first).
    fn tab_index(mut self, index: isize) -> Self {
        self.dispatch_props().focusable.get_or_insert_with(Focusable::default).tab_index = index;
        self
    }

    /// Focusable; `false` keeps it out of Tab traversal while `focus` can still reach it.
    fn tab_stop(mut self, tab_stop: bool) -> Self {
        self.dispatch_props().focusable.get_or_insert_with(Focusable::default).tab_stop = tab_stop;
        self
    }

    /// Handles `A` when it bubbles through this element from the focused node. The action stops
    /// here unless the listener calls [`App::propagate`].
    fn on_action<A: Action>(mut self, listener: impl Fn(&A, &mut Window, &mut App) + 'static) -> Self {
        let listener = Rc::new(listener);
        let registrar: ActionRegistrar = Rc::new(move |tree, node, window_id| {
            let listener = listener.clone();
            tree.on_action(node, move |action: &A, acx| {
                let app = &mut *acx.cx;
                let Some(mut window) = app.windows.get_mut(&window_id).and_then(|w| w.window.take()) else {
                    acx.propagate();
                    return;
                };
                app.propagate_action = false;
                listener(action, &mut window, app);
                if let Some(slot) = app.windows.get_mut(&window_id) {
                    slot.window = Some(window);
                }
                if std::mem::take(&mut app.propagate_action) {
                    acx.propagate();
                }
            });
        });
        self.dispatch_props().actions.push(registrar);
        self
    }
}

pub(crate) struct NodeElement {
    pub kind: NodeKind,
    pub key: Key,
    pub props: Props,
    pub layout: StyleRefinement,
    /// The backend measures it (text, native widgets); containers are sized by Taffy alone.
    pub measured: bool,
    pub children: Vec<AnyElement>,
    pub handlers: Vec<(TypeId, Handler)>,
    pub dispatch: DispatchProps,
}

pub(crate) enum Element {
    Node(Box<NodeElement>),
    View(AnyView),
}

pub struct AnyElement(pub(crate) Element);

impl AnyElement {
    fn node(node: NodeElement) -> Self {
        AnyElement(Element::Node(Box::new(node)))
    }

    pub(crate) fn view(view: AnyView) -> Self {
        AnyElement(Element::View(view))
    }
}

pub trait IntoElement: Sized {
    fn into_any_element(self) -> AnyElement;
}

impl IntoElement for AnyElement {
    fn into_any_element(self) -> AnyElement {
        self
    }
}

impl IntoElement for SharedString {
    #[track_caller]
    fn into_any_element(self) -> AnyElement {
        text(self).into_any_element()
    }
}

impl IntoElement for &'static str {
    #[track_caller]
    fn into_any_element(self) -> AnyElement {
        text(self).into_any_element()
    }
}

impl IntoElement for String {
    #[track_caller]
    fn into_any_element(self) -> AnyElement {
        text(self).into_any_element()
    }
}

pub trait ParentElement: Sized {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>);

    fn child(mut self, child: impl IntoElement) -> Self {
        self.extend([child.into_any_element()]);
        self
    }

    fn children(mut self, children: impl IntoIterator<Item = impl IntoElement>) -> Self {
        self.extend(children.into_iter().map(IntoElement::into_any_element));
        self
    }
}

/// Props of a `div`. Backends map the visual part (background, radius, border, shadow,
/// opacity); layout fields are Taffy's and arrive as frames.
#[derive(Clone, Debug, PartialEq)]
pub struct DivProps {
    pub style: StyleRefinement,
    /// What a layer above `nd-ui` attached to this container (a subtree theme override, a
    /// context menu marker), for the backends that read it.
    pub scope: Option<Scope>,
}

/// Type-erased props a layer above `nd-ui` hangs on a `div`, compared by value.
#[derive(Clone, Debug)]
pub struct Scope(pub Props);

impl Scope {
    pub fn new(props: impl AnyProps) -> Self {
        Scope(Rc::new(props))
    }

    pub fn get<T: Any>(&self) -> Option<&T> {
        self.0.downcast_ref()
    }
}

impl PartialEq for Scope {
    fn eq(&self, other: &Self) -> bool {
        self.0.props_eq(&*other.0)
    }
}

/// Props of the window's content root.
#[derive(Clone, Debug, PartialEq)]
pub struct RootProps;

pub struct Div {
    key: Key,
    style: StyleRefinement,
    children: Vec<AnyElement>,
    dispatch: DispatchProps,
    scope: Option<Scope>,
    handlers: Vec<(TypeId, Handler)>,
}

#[track_caller]
pub fn div() -> Div {
    Div::at(Location::caller())
}

/// A row: flex, items centered.
#[track_caller]
pub fn h_flex() -> Div {
    Div::at(Location::caller()).flex().flex_row().items_center()
}

/// A column.
#[track_caller]
pub fn v_flex() -> Div {
    Div::at(Location::caller()).flex().flex_col()
}

impl Div {
    fn at(site: &'static Location<'static>) -> Self {
        Div {
            key: Key::Site(site),
            style: StyleRefinement::default(),
            children: Vec::new(),
            dispatch: DispatchProps::default(),
            scope: None,
            handlers: Vec::new(),
        }
    }

    /// Keys the element explicitly, for siblings whose order or presence changes.
    pub fn id(mut self, id: impl Into<SharedString>) -> Self {
        self.key = Key::Id(id.into());
        self
    }

    /// Attaches `scope` to the node's props; see [`DivProps::scope`].
    pub fn scope(mut self, scope: Scope) -> Self {
        self.scope = Some(scope);
        self
    }

    pub fn scope_ref(&self) -> Option<&Scope> {
        self.scope.as_ref()
    }

    /// Handles native events of type `E` the backend reports for this node (a context menu
    /// request). Replaces an earlier handler for `E`.
    pub fn on<E: 'static>(mut self, handler: impl Fn(&E, &mut Window, &mut App) + 'static) -> Self {
        let handler: Handler = Rc::new(move |event, window, cx| {
            if let Some(event) = event.downcast_ref::<E>() {
                handler(event, window, cx);
            }
        });
        self.handlers.retain(|(t, _)| *t != TypeId::of::<E>());
        self.handlers.push((TypeId::of::<E>(), handler));
        self
    }
}

impl Styled for Div {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl ParentElement for Div {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl FluentBuilder for Div {}

impl InteractiveElement for Div {
    fn dispatch_props(&mut self) -> &mut DispatchProps {
        &mut self.dispatch
    }
}

impl IntoElement for Div {
    fn into_any_element(self) -> AnyElement {
        AnyElement::node(NodeElement {
            kind: NodeKind::View,
            key: self.key,
            props: Rc::new(DivProps { style: self.style.clone(), scope: self.scope }),
            layout: self.style,
            measured: false,
            children: self.children,
            handlers: self.handlers,
            dispatch: self.dispatch,
        })
    }
}

/// Props of a text run.
#[derive(Clone, Debug, PartialEq)]
pub struct TextProps {
    pub text: SharedString,
    pub style: StyleRefinement,
    pub tabular_nums: bool,
}

pub struct Text {
    key: Key,
    props: TextProps,
}

#[track_caller]
pub fn text(text: impl Into<SharedString>) -> Text {
    Text {
        key: Key::Site(Location::caller()),
        props: TextProps { text: text.into(), style: StyleRefinement::default(), tabular_nums: false },
    }
}

impl Text {
    pub fn id(mut self, id: impl Into<SharedString>) -> Self {
        self.key = Key::Id(id.into());
        self
    }

    /// Tabular figures, so changing digits keep their width.
    pub fn tabular_nums(mut self) -> Self {
        self.props.tabular_nums = true;
        self
    }
}

impl Styled for Text {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.props.style
    }
}

impl FluentBuilder for Text {}

impl IntoElement for Text {
    fn into_any_element(self) -> AnyElement {
        AnyElement::node(NodeElement {
            kind: NodeKind::Text,
            key: self.key,
            layout: self.props.style.clone(),
            props: Rc::new(self.props),
            measured: true,
            children: Vec::new(),
            handlers: Vec::new(),
            dispatch: DispatchProps::default(),
        })
    }
}

/// A native widget leaf: `W` names the widget kind the backend maps, `P` its props. The
/// backend measures it and reports its events through the event queue.
pub struct NativeElement<P> {
    kind: WidgetKind,
    key: Key,
    props: P,
    style: StyleRefinement,
    handlers: Vec<(TypeId, Handler)>,
    dispatch: DispatchProps,
}

impl<P: AnyProps> NativeElement<P> {
    pub fn new<W: 'static>(key: impl Into<Key>, props: P) -> Self {
        Self {
            kind: WidgetKind::of::<W>(),
            key: key.into(),
            props,
            style: StyleRefinement::default(),
            handlers: Vec::new(),
            dispatch: DispatchProps::default(),
        }
    }

    pub fn props_mut(&mut self) -> &mut P {
        &mut self.props
    }

    pub fn props(&self) -> &P {
        &self.props
    }

    /// Handles events of type `E` the widget emits. Replaces an earlier handler for `E`.
    pub fn on<E: 'static>(mut self, handler: impl Fn(&E, &mut Window, &mut App) + 'static) -> Self {
        let handler: Handler = Rc::new(move |event, window, cx| {
            if let Some(event) = event.downcast_ref::<E>() {
                handler(event, window, cx);
            }
        });
        self.handlers.retain(|(t, _)| *t != TypeId::of::<E>());
        self.handlers.push((TypeId::of::<E>(), handler));
        self
    }
}

impl<P> Styled for NativeElement<P> {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl<P> FluentBuilder for NativeElement<P> {}

impl<P> InteractiveElement for NativeElement<P> {
    fn dispatch_props(&mut self) -> &mut DispatchProps {
        &mut self.dispatch
    }
}

impl<P: AnyProps> IntoElement for NativeElement<P> {
    fn into_any_element(self) -> AnyElement {
        AnyElement::node(NodeElement {
            kind: NodeKind::Widget(self.kind),
            key: self.key,
            props: Rc::new(self.props),
            layout: self.style,
            measured: true,
            children: Vec::new(),
            handlers: self.handlers,
            dispatch: self.dispatch,
        })
    }
}

/// Fired by a button on activation (click, Space, Return as default button).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClickEvent;

/// What a button means, which each platform draws its own way (a default button, an accent
/// fill, an outline).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ButtonVariant {
    #[default]
    Standard,
    /// The window's default action: Return activates it.
    Primary,
    /// Filled with the accent colour.
    Accent,
    Outline,
    Destructive,
}

/// The macOS bezel a button asks for; ignored elsewhere.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Bezel {
    #[default]
    Automatic,
    /// Liquid Glass on macOS 26 and later, the automatic bezel below.
    Glass,
    Bordered,
    Borderless,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ButtonProps {
    pub label: SharedString,
    /// A Lucide icon name (`"share"`), mapped per platform (SF Symbols on macOS).
    pub icon: Option<SharedString>,
    pub variant: ButtonVariant,
    pub bezel: Bezel,
    pub tooltip: Option<SharedString>,
    /// The element's style: a background becomes the bezel colour, a text colour and font
    /// the title's, where the native control can express them.
    pub style: StyleRefinement,
}

impl ButtonProps {
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            icon: None,
            variant: ButtonVariant::Standard,
            bezel: Bezel::Automatic,
            tooltip: None,
            style: StyleRefinement::default(),
        }
    }
}

/// A push button; `NSButton` on AppKit. Its widget kind is `WidgetKind::of::<Button>()`.
pub struct Button(NativeElement<ButtonProps>);

pub fn button(id: impl Into<SharedString>, label: impl Into<SharedString>) -> Button {
    Button(NativeElement::new::<Button>(Key::Id(id.into()), ButtonProps::new(label)))
}

impl Button {
    pub fn on_click(self, handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        Button(self.0.on(handler))
    }

    pub fn props_mut(&mut self) -> &mut ButtonProps {
        self.0.props_mut()
    }

    pub fn variant(mut self, variant: ButtonVariant) -> Self {
        self.0.props_mut().variant = variant;
        self
    }

    pub fn bezel(mut self, bezel: Bezel) -> Self {
        self.0.props_mut().bezel = bezel;
        self
    }

    pub fn icon_name(mut self, icon: impl Into<SharedString>) -> Self {
        self.0.props_mut().icon = Some(icon.into());
        self
    }
}

impl Styled for Button {
    fn style(&mut self) -> &mut StyleRefinement {
        self.0.style()
    }
}

impl FluentBuilder for Button {}

impl InteractiveElement for Button {
    fn dispatch_props(&mut self) -> &mut DispatchProps {
        self.0.dispatch_props()
    }
}

impl IntoElement for Button {
    fn into_any_element(mut self) -> AnyElement {
        self.0.props.style = self.0.style.clone();
        self.0.into_any_element()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct WindowControlsProps {
    /// Pins the OS window controls with their first button's top-left at this point, in
    /// window coordinates; `None` leaves them where the OS puts them.
    pub at: Option<nd_core::Point>,
    /// A layout slot the size of the controls: the backend moves the real buttons onto the
    /// slot's rect on every layout pass, so they ride with whatever moves the slot.
    pub embedded: bool,
}

/// The window's own controls (traffic lights on macOS, ARCHITECTURE.md 5.4). The buttons stay
/// the OS's; the element only says where they go and takes no space in layout. Its widget kind
/// is `WidgetKind::of::<WindowControls>()`.
pub struct WindowControls(NativeElement<WindowControlsProps>);

#[track_caller]
pub fn window_controls() -> WindowControls {
    let props = WindowControlsProps { at: None, embedded: false };
    let element = NativeElement::new::<WindowControls>(Key::Site(Location::caller()), props);
    WindowControls(element.absolute())
}

impl WindowControls {
    /// Takes space in layout, the size of the controls, and the controls follow it.
    pub fn embedded(mut self) -> Self {
        self.0.props_mut().embedded = true;
        self.0.props_mut().at = None;
        self.0.style().position = Some(nd_gpui::gpui::Position::Relative);
        self
    }

    pub fn props(&self) -> &WindowControlsProps {
        &self.0.props
    }

    pub fn at(mut self, at: nd_gpui::gpui::Point<nd_gpui::gpui::Pixels>) -> Self {
        self.0.props_mut().at = Some(nd_core::Point { x: at.x.as_f32(), y: at.y.as_f32() });
        self.0.props_mut().embedded = false;
        self
    }
}

impl IntoElement for WindowControls {
    fn into_any_element(self) -> AnyElement {
        self.0.into_any_element()
    }
}
